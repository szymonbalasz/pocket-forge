//! The background export (FR-EXP-09, 12, 16..23, 27): snapshot → compose → (rescale) → encode
//! into `.<name>.pf-tmp-<ulid>` → flush → rename (lock retry) → `last` record.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use image::{RgbImage, RgbaImage};
use pf_imaging::{
    COMPOSE_VERSION, ComposeError, EncodeError, ImageRows, Layer as ImgLayer, LayerStack, LibWebp,
    Phase, Placement, STRIP_H, StripFlattener, WebpEncoder, encode_jpeg, encode_png,
    encode_webp_with, flatten_full, resize_lanczos3,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use super::settings::{ExportFormat, stamp};
use crate::context::{Context, FaultPoint};
use crate::core::State;
use crate::error::{CoreError, ErrorCode, is_locked};
use crate::fsx::TEMP_MARKER;
use crate::manifest::{Layer, LayerKind};
use crate::project::store;

/// How long a locked destination is retried before `locked` (FR-EXP-18).
const LOCK_RETRY_FOR: Duration = Duration::from_secs(3);
/// Temp files older than this are swept before a write (FR-EXP-17).
const STALE_TEMP_AGE: Duration = Duration::from_secs(3600);
/// Progress events are at most this frequent (and at least once per strip, FR-EXP-19).
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

type RenameFn = dyn Fn(&Path, &Path) -> io::Result<()> + Send + Sync;
type LockedFn = dyn Fn(&io::Error) -> bool + Send + Sync;

/// Seams for the export's integration tests (EXP §13). The defaults are the real ones.
pub struct ExportSeams {
    /// The WebP encoder (the FR-EXP-12 partition-overflow seam, and the allocator seam).
    pub webp: Arc<dyn WebpEncoder + Send + Sync>,
    /// The final rename (the FR-EXP-18 lock seam).
    pub rename: Box<RenameFn>,
    /// Whether an error means another process holds the file.
    pub is_locked: Box<LockedFn>,
    pub lock_retry_for: Duration,
    /// Sets the cancel flag after this many progress reports (cancel mid-strip / mid-WebP).
    pub cancel_after_progress: Option<usize>,
}

impl Default for ExportSeams {
    fn default() -> Self {
        ExportSeams {
            webp: Arc::new(LibWebp),
            rename: Box::new(|from, to| fs::rename(from, to)),
            is_locked: Box::new(is_locked),
            lock_retry_for: LOCK_RETRY_FOR,
            cancel_after_progress: None,
        }
    }
}

impl std::fmt::Debug for ExportSeams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportSeams")
            .field("lock_retry_for", &self.lock_retry_for)
            .field("cancel_after_progress", &self.cancel_after_progress)
            .finish_non_exhaustive()
    }
}

/// Everything an export needs, taken at start (FR-EXP-21: the stack snapshot).
pub(crate) struct ExportJob {
    pub id: Ulid,
    pub project_id: Ulid,
    pub project_dir: PathBuf,
    /// Project-relative destination (`exports/…`).
    pub rel_path: String,
    pub format: ExportFormat,
    pub width: u32,
    pub height: u32,
    pub rescale: bool,
    pub original: Arc<RgbImage>,
    pub layers: Vec<Layer>,
    pub head_revision: Ulid,
    pub cancel: Arc<AtomicBool>,
    pub seams: Arc<ExportSeams>,
}

/// SHA-256 of the canonical JSON (declared field order, no whitespace) of `layers[]`
/// (manifest-schema §4.10).
pub(crate) fn layers_hash(layers: &[Layer]) -> String {
    let bytes = serde_json::to_vec(layers).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    let mut s = String::from("sha256:");
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Why an export stopped.
#[derive(Debug)]
enum Stop {
    Canceled,
    Failed(CoreError),
}

impl From<CoreError> for Stop {
    fn from(e: CoreError) -> Self {
        Stop::Failed(e)
    }
}

fn failure(code: ErrorCode, message: impl Into<String>) -> Stop {
    Stop::Failed(CoreError::new(code, message))
}

/// `disk_full` by kind or OS code (ERROR_DISK_FULL 112, ERROR_HANDLE_DISK_FULL 39, ENOSPC 28).
fn is_disk_full(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::StorageFull
        || if cfg!(windows) {
            matches!(e.raw_os_error(), Some(39 | 112))
        } else {
            e.raw_os_error() == Some(28)
        }
}

fn io_stop(seams: &ExportSeams, what: &str, e: &io::Error) -> Stop {
    let code = if (seams.is_locked)(e) {
        ErrorCode::Locked
    } else if is_disk_full(e) {
        ErrorCode::DiskFull
    } else {
        ErrorCode::Io
    };
    let message = match code {
        ErrorCode::Locked => "The file is in use (VTT or sync?)".to_owned(),
        _ => format!("{what}: {e}"),
    };
    failure(code, message)
}

fn encode_stop(seams: &ExportSeams, e: EncodeError) -> Stop {
    match e {
        EncodeError::Canceled => Stop::Canceled,
        EncodeError::OutOfMemory => {
            failure(ErrorCode::OutOfMemory, "not enough memory for this export")
        }
        EncodeError::TooLarge { format, limit } => Stop::Failed(
            CoreError::new(ErrorCode::TooLarge, format!("{format:?} max {limit} px")).with_detail(
                json!({ "format": format!("{format:?}").to_lowercase(), "limit": limit }),
            ),
        ),
        EncodeError::Io(io) => io_stop(seams, "writing the export", &io),
        EncodeError::PartitionOverflow | EncodeError::Failed(_) => {
            failure(ErrorCode::EncodeFailed, e.to_string())
        }
    }
}

fn compose_stop(e: ComposeError) -> Stop {
    match e {
        ComposeError::OutOfMemory => {
            failure(ErrorCode::OutOfMemory, "not enough memory for this export")
        }
        other => failure(ErrorCode::Internal, other.to_string()),
    }
}

/// Emits `export:progress` (throttled) and turns the cancel flag into `Break` (FR-EXP-19/20).
struct Reporter<'a> {
    ctx: &'a Context,
    job: &'a ExportJob,
    last: Option<(Phase, Instant)>,
    reports: AtomicUsize,
}

impl Reporter<'_> {
    fn report(&mut self, phase: Phase, fraction: f32) -> ControlFlow<()> {
        let n = self.reports.fetch_add(1, Ordering::Relaxed) + 1;
        if self.job.seams.cancel_after_progress.is_some_and(|k| n >= k) {
            self.job.cancel.store(true, Ordering::SeqCst);
        }
        if self.job.cancel.load(Ordering::SeqCst) {
            return ControlFlow::Break(());
        }
        let due = match self.last {
            None => true,
            Some((p, at)) => p != phase || at.elapsed() >= PROGRESS_EVERY || fraction >= 1.0,
        };
        if due {
            self.last = Some((phase, Instant::now()));
            self.ctx.emit(
                "export:progress",
                json!({ "export_id": self.job.id, "phase": phase.as_str(), "fraction": fraction }),
            );
        }
        ControlFlow::Continue(())
    }

    fn check(&mut self, phase: Phase, fraction: f32) -> Result<(), Stop> {
        match self.report(phase, fraction) {
            ControlFlow::Continue(()) => Ok(()),
            ControlFlow::Break(()) => Err(Stop::Canceled),
        }
    }
}

/// Decodes the drawn patch layers of the snapshot (FR-EXP-22: none skipped, none partial).
fn layer_stack(job: &ExportJob, r: &mut Reporter<'_>) -> Result<LayerStack, Stop> {
    let mut layers = Vec::new();
    let drawn: Vec<&Layer> = job
        .layers
        .iter()
        .filter(|l| l.kind == LayerKind::Patch && l.visible && l.opacity > 0.0)
        .collect();
    for l in drawn {
        let unreadable = |why: String| {
            Stop::Failed(
                CoreError::new(
                    ErrorCode::LayerUnreadable,
                    format!("layer {} can't be read: {why}", l.id),
                )
                .with_detail(json!({ "layer_id": l.id })),
            )
        };
        let file = l
            .file
            .as_deref()
            .ok_or_else(|| unreadable("it has no file".into()))?;
        let image: RgbaImage = image::open(job.project_dir.join(file))
            .map_err(|e| unreadable(e.to_string()))?
            .to_rgba8();
        layers.push(ImgLayer {
            image: Arc::new(image),
            placement: Placement {
                cx: l.cx,
                cy: l.cy,
                scale: l.scale,
                rotation: l.rotation,
            },
            opacity: l.opacity as f32,
            visible: true,
        });
        r.check(Phase::Compose, 0.0)?;
    }
    LayerStack::new(job.original.clone(), &layers).map_err(compose_stop)
}

/// FR-EXP-17: removes `.*.pf-tmp-*` files older than an hour in the destination folder.
fn sweep_stale_temps(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with('.') && name.contains(TEMP_MARKER)
        })
        .filter(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age >= STALE_TEMP_AGE)
        })
        .filter(|e| fs::remove_file(e.path()).is_ok())
        .count()
}

/// The rename with FR-EXP-18's lock retry: backoff for up to `lock_retry_for`, then the error.
fn rename_retrying(seams: &ExportSeams, from: &Path, to: &Path) -> io::Result<()> {
    let start = Instant::now();
    let mut delay = Duration::from_millis(25);
    loop {
        match (seams.rename)(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if (seams.is_locked)(&e) && start.elapsed() < seams.lock_retry_for => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(500));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Encodes into the open temp file.
fn encode_into(
    job: &ExportJob,
    stack: &LayerStack,
    file: &File,
    r: &mut Reporter<'_>,
) -> Result<bool, Stop> {
    let seams = &*job.seams;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    let mut progress = |phase: Phase, f: f32| r.report(phase, f);
    let streams = !job.rescale && job.format != ExportFormat::Webp;
    let mut fallback = false;
    if streams {
        // Strips are flattened as the encoder needs them (ADR 0004, FR-EXP-09).
        let mut rows = StripFlattener::new(stack, STRIP_H).map_err(compose_stop)?;
        let result = match job.format {
            ExportFormat::Png => encode_png(&mut rows, &mut out, Phase::Compose, &mut progress),
            _ => encode_jpeg(&mut rows, &mut out, Phase::Compose, &mut progress),
        };
        result.map_err(|e| encode_stop(seams, e))?;
    } else {
        let (_, h) = stack.dimensions();
        let full = flatten_full::<EncodeError, _>(stack, STRIP_H, |done| {
            match progress(Phase::Compose, done as f32 / h as f32) {
                ControlFlow::Continue(()) => Ok(()),
                ControlFlow::Break(()) => Err(EncodeError::Canceled),
            }
        })
        .map_err(|e| encode_stop(seams, e))?;
        let image = if job.rescale {
            if progress(Phase::Resize, 0.0).is_break() {
                return Err(Stop::Canceled);
            }
            let small =
                resize_lanczos3(&full, job.width, job.height).map_err(|e| encode_stop(seams, e))?;
            drop(full);
            if progress(Phase::Resize, 1.0).is_break() {
                return Err(Stop::Canceled);
            }
            small
        } else {
            full
        };
        match job.format {
            ExportFormat::Png => {
                encode_png(
                    &mut ImageRows::new(&image),
                    &mut out,
                    Phase::Encode,
                    &mut progress,
                )
                .map_err(|e| encode_stop(seams, e))?;
            }
            ExportFormat::Jpeg => {
                encode_jpeg(
                    &mut ImageRows::new(&image),
                    &mut out,
                    Phase::Encode,
                    &mut progress,
                )
                .map_err(|e| encode_stop(seams, e))?;
            }
            ExportFormat::Webp => {
                let webp = encode_webp_with(&*seams.webp, &image, &mut progress)
                    .map_err(|e| encode_stop(seams, e))?;
                fallback = webp.lossless_fallback;
                drop(image);
                if progress(Phase::Write, 0.0).is_break() {
                    return Err(Stop::Canceled);
                }
                out.write_all(&webp.bytes)
                    .map_err(|e| io_stop(seams, "writing the export", &e))?;
            }
        }
    }
    out.flush()
        .map_err(|e| io_stop(seams, "writing the export", &e))?;
    Ok(fallback)
}

/// A finished export.
struct Done {
    path: PathBuf,
    bytes: u64,
    fallback: bool,
}

fn export(ctx: &Context, job: &ExportJob) -> Result<Done, Stop> {
    let seams = &*job.seams;
    let mut r = Reporter {
        ctx,
        job,
        last: None,
        reports: AtomicUsize::new(0),
    };
    r.check(Phase::Compose, 0.0)?;
    let dest = super::settings::abs_path(&job.project_dir, &job.rel_path);
    let dir = dest
        .parent()
        .ok_or_else(|| failure(ErrorCode::Internal, "no destination folder"))?;
    fs::create_dir_all(dir).map_err(|e| {
        Stop::Failed(CoreError::new(
            ErrorCode::DestUnavailable,
            format!("creating {}: {e}", dir.display()),
        ))
    })?;
    sweep_stale_temps(dir);
    let stack = layer_stack(job, &mut r)?;

    let tmp = crate::fsx::temp_path(ctx, &dest);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| io_stop(seams, "creating the temp file", &e))?;
    let result = (|| {
        let fallback = encode_into(job, &stack, &file, &mut r)?;
        r.check(Phase::Write, 0.0)?;
        file.sync_all()
            .map_err(|e| io_stop(seams, "flushing the export", &e))?;
        drop(file);
        ctx.faults
            .hit(FaultPoint::AfterTemp, &dest)
            .map_err(|e| io_stop(seams, "writing the export", &e))?;
        r.check(Phase::Write, 0.5)?;
        rename_retrying(seams, &tmp, &dest)
            .map_err(|e| io_stop(seams, "replacing the export", &e))?;
        Ok(fallback)
    })();
    let fallback = match result {
        Ok(f) => f,
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    };
    // Renamed: the file stands whatever happens next (FR-EXP-16).
    if let Err(e) = ctx.faults.hit(FaultPoint::AfterRename, &dest) {
        return Err(io_stop(seams, "after the rename", &e));
    }
    let bytes = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    let _ = r.report(Phase::Write, 1.0);
    Ok(Done {
        path: dest,
        bytes,
        fallback,
    })
}

/// Commits `last` (FR-EXP-27) if the project is still open; false if it couldn't.
fn commit_last(ctx: &Context, state: &mut State, job: &ExportJob, done: &Done) -> bool {
    let install_id = state.settings.install_id;
    let Some(p) = state
        .project
        .as_mut()
        .filter(|p| p.manifest.project_id == job.project_id)
    else {
        return false;
    };
    let mut fields = Map::new();
    fields.insert(
        "last".into(),
        json!({
            "at": stamp(ctx.now()),
            "revision_id": job.head_revision,
            "layers_hash": layers_hash(&job.layers),
            "path": job.rel_path,
            "format": job.format.as_str(),
            "width": job.width,
            "height": job.height,
            "bytes": done.bytes,
            "fallback": if done.fallback { Value::from("lossless") } else { Value::Null },
            "strip_h": STRIP_H,
            "compose_version": COMPOSE_VERSION,
        }),
    );
    match store::put_export(ctx, install_id, p, fields) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(error = %e, "export written but its record wasn't saved");
            false
        }
    }
}

/// Runs one export to its end and emits `export:done`, `export:failed` or `export:canceled`.
pub(crate) fn run(ctx: &Context, state: &Mutex<State>, job: ExportJob) {
    let result = export(ctx, &job);
    let lock = || state.lock().unwrap_or_else(|e| e.into_inner());
    match result {
        Ok(done) => {
            let record_saved = {
                let mut s = lock();
                let saved = commit_last(ctx, &mut s, &job, &done);
                s.exp.finish(job.id);
                saved
            };
            ctx.emit(
                "export:done",
                json!({
                    "export_id": job.id,
                    "path": done.path,
                    "width": job.width,
                    "height": job.height,
                    "bytes": done.bytes,
                    "format": job.format.as_str(),
                    "fallback": if done.fallback { Value::from("lossless") } else { Value::Null },
                    "record_saved": record_saved,
                }),
            );
        }
        Err(Stop::Canceled) => {
            lock().exp.finish(job.id);
            ctx.emit("export:canceled", json!({ "export_id": job.id }));
        }
        Err(Stop::Failed(e)) => {
            lock().exp.finish(job.id);
            tracing::info!(code = %e.code, message = %e.message, "export failed");
            let mut detail = match e.detail {
                Value::Object(m) => m,
                _ => Map::new(),
            };
            detail.insert("message".into(), Value::from(e.message));
            ctx.emit(
                "export:failed",
                json!({ "export_id": job.id, "code": e.code, "detail": detail }),
            );
        }
    }
}

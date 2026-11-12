//! The background export (FR-EXP-09, 12, 16..23, 27): snapshot → compose → (rescale) → encode
//! into `.<name>.pf-tmp-<ulid>` → flush → rename (lock retry) → `last` record.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use image::{RgbImage, RgbaImage};
use pf_imaging::{
    COMPOSE_VERSION, ComposeError, EncodeError, ImageRows, Layer as ImgLayer, LayerStack, LibWebp,
    Phase, Placement, STRIP_H, StripFlattener, WebpEncoder, encode_jpeg, encode_png,
    encode_png_rgba, encode_webp_rgba, encode_webp_with, flatten_full, resize_lanczos3,
    to_rgb8_flattened,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use super::settings::{ExportFormat, stamp};
use crate::context::{Context, FaultPoint};
use crate::core::State;
use crate::error::{CoreError, ErrorCode, is_locked};
use crate::fsx;
use crate::manifest::{Layer, LayerKind};
use crate::project::store;

/// How long a locked destination is retried before `locked` (FR-EXP-18).
const LOCK_RETRY_FOR: Duration = Duration::from_secs(3);
/// Temp files older than this are swept before a write (FR-EXP-17; PRJ's open sweep uses it too,
/// BL-32).
const STALE_TEMP_AGE: Duration = fsx::STALE_TEMP_FILE_AGE;
/// Progress events are at most this frequent (and at least once per strip, FR-EXP-19).
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
/// A step with no progress hook (Lanczos, the lock retry) still reports this often (FR-EXP-19:
/// at least every 500 ms).
const HEARTBEAT: Duration = Duration::from_millis(200);

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

/// Where an export is written.
pub(crate) enum Dest {
    /// The project's `exports/` (FR-EXP-13): the folder is made if missing and FR-EXP-17's stale
    /// temps are swept. `record`: `last` is committed after the rename (FR-EXP-27, 30).
    Exports {
        path: PathBuf,
        /// Project-relative (`exports/…`).
        rel_path: String,
        record: bool,
    },
    /// A path from the save dialog (FR-EXP-31, 32): only this export's own temp is ever removed
    /// there, nothing is recorded, and the folder becomes the last Save as folder.
    SaveAs(PathBuf),
}

impl Dest {
    pub fn path(&self) -> &Path {
        match self {
            Dest::Exports { path, .. } | Dest::SaveAs(path) => path,
        }
    }
}

/// What an export reads.
pub(crate) enum Input {
    /// The layer stack (FR-EXP-21: the snapshot taken at start).
    Stack {
        project_dir: PathBuf,
        rescale: bool,
        original: Arc<RgbImage>,
        layers: Vec<Layer>,
        head_revision: Ulid,
    },
    /// One image file (FR-EXP-32): `copy` = the bytes as they are (same format), else decoded
    /// and encoded at its own size.
    File { path: PathBuf, copy: bool },
}

/// Everything an export needs, taken at start (FR-EXP-21: the stack snapshot).
pub(crate) struct ExportJob {
    pub id: Ulid,
    pub scope: super::Scope,
    pub input: Input,
    pub dest: Dest,
    pub format: ExportFormat,
    pub width: u32,
    pub height: u32,
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
fn layer_stack(
    project_dir: &Path,
    original: &Arc<RgbImage>,
    all: &[Layer],
    r: &mut Reporter<'_>,
) -> Result<LayerStack, Stop> {
    let mut layers = Vec::new();
    let drawn: Vec<&Layer> = all
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
        let image: RgbaImage = image::open(project_dir.join(file))
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
    LayerStack::new(original.clone(), &layers).map_err(compose_stop)
}

/// FR-EXP-17: removes `.*.pf-tmp-*` files older than an hour in the destination folder.
fn sweep_stale_temps(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    entries
        .flatten()
        .filter(|e| fsx::is_stale_temp_file(e, now, STALE_TEMP_AGE))
        .filter(|e| fs::remove_file(e.path()).is_ok())
        .count()
}

/// The rename with FR-EXP-18's lock retry: backoff for up to `lock_retry_for`, then the error.
/// Each wait is at most `HEARTBEAT` and is followed by a `write` report (FR-EXP-19), which also
/// lets a cancel end the retry (FR-EXP-20).
fn rename_retrying(
    seams: &ExportSeams,
    from: &Path,
    to: &Path,
    r: &mut Reporter<'_>,
) -> Result<(), Stop> {
    let start = Instant::now();
    let mut delay = Duration::from_millis(25);
    loop {
        match (seams.rename)(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if (seams.is_locked)(&e) && start.elapsed() < seams.lock_retry_for => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(HEARTBEAT);
                r.check(Phase::Write, 0.5)?;
            }
            Err(e) => return Err(io_stop(seams, "replacing the export", &e)),
        }
    }
}

/// Runs `work` on a scoped thread and calls `tick` every `every` until it ends: the heartbeat
/// for a step with no progress hook (FR-EXP-19). `work` can't be stopped, so a cancel (`tick`
/// breaks) is kept and returned as `Break` once `work` has ended.
fn with_heartbeat<T: Send>(
    every: Duration,
    mut tick: impl FnMut() -> ControlFlow<()>,
    work: impl FnOnce() -> T + Send,
) -> ControlFlow<(), T> {
    std::thread::scope(|s| {
        let (tx, rx) = mpsc::channel();
        s.spawn(move || {
            let _ = tx.send(work());
        });
        let mut canceled = false;
        loop {
            match rx.recv_timeout(every) {
                Ok(out) if !canceled => return ControlFlow::Continue(out),
                Ok(_) => return ControlFlow::Break(()),
                Err(RecvTimeoutError::Timeout) => canceled = canceled || tick().is_break(),
                // `work` panicked; the scope re-raises the panic.
                Err(RecvTimeoutError::Disconnected) => return ControlFlow::Break(()),
            }
        }
    })
}

/// Encodes into the open temp file.
fn encode_into(
    job: &ExportJob,
    rescale: bool,
    stack: &LayerStack,
    file: &File,
    r: &mut Reporter<'_>,
) -> Result<bool, Stop> {
    let seams = &*job.seams;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    let mut progress = |phase: Phase, f: f32| r.report(phase, f);
    let streams = !rescale && job.format != ExportFormat::Webp;
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
        let image = if rescale {
            if progress(Phase::Resize, 0.0).is_break() {
                return Err(Stop::Canceled);
            }
            let (w, h) = (job.width, job.height);
            let resized = with_heartbeat(
                HEARTBEAT,
                || progress(Phase::Resize, 0.0),
                || resize_lanczos3(&full, w, h),
            );
            let ControlFlow::Continue(small) = resized else {
                return Err(Stop::Canceled);
            };
            let small = small.map_err(|e| encode_stop(seams, e))?;
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

/// FR-EXP-32: copies a same-format source into the open temp file, as it is.
fn copy_into(
    source: &Path,
    file: &File,
    seams: &ExportSeams,
    r: &mut Reporter<'_>,
) -> Result<(), Stop> {
    let mut input = File::open(source).map_err(|e| io_stop(seams, "reading the image", &e))?;
    let total = input.metadata().map(|m| m.len()).unwrap_or(0).max(1);
    let mut out = BufWriter::with_capacity(1 << 20, file);
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        let n = io::Read::read(&mut input, &mut buf)
            .map_err(|e| io_stop(seams, "reading the image", &e))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| io_stop(seams, "writing the export", &e))?;
        done += n as u64;
        r.check(Phase::Write, (done as f32 / total as f32).min(1.0) * 0.5)?;
    }
    out.flush()
        .map_err(|e| io_stop(seams, "writing the export", &e))
}

/// FR-EXP-32: decodes a source file and encodes it at its own size, with no resample. PNG and
/// WebP keep its alpha; JPEG flattens it over the flatten colour `#808080` (#60). Returns the
/// WebP lossless fallback flag.
fn convert_into(
    job: &ExportJob,
    source: &Path,
    file: &File,
    r: &mut Reporter<'_>,
) -> Result<bool, Stop> {
    let seams = &*job.seams;
    r.check(Phase::Compose, 0.0)?;
    let decoded = image::open(source).map_err(|e| {
        failure(
            ErrorCode::Corrupt,
            format!("{} can't be decoded: {e}", source.display()),
        )
    })?;
    r.check(Phase::Compose, 1.0)?;
    let alpha = decoded.color().has_alpha();
    let mut out = BufWriter::with_capacity(1 << 20, file);
    let mut progress = |phase: Phase, f: f32| r.report(phase, f);
    let mut fallback = false;
    match job.format {
        ExportFormat::Png if alpha => {
            encode_png_rgba(&decoded.into_rgba8(), &mut out, &mut progress)
                .map_err(|e| encode_stop(seams, e))?;
        }
        ExportFormat::Png => {
            let rgb = decoded.into_rgb8();
            encode_png(
                &mut ImageRows::new(&rgb),
                &mut out,
                Phase::Encode,
                &mut progress,
            )
            .map_err(|e| encode_stop(seams, e))?;
        }
        ExportFormat::Jpeg => {
            let rgb = to_rgb8_flattened(decoded);
            encode_jpeg(
                &mut ImageRows::new(&rgb),
                &mut out,
                Phase::Encode,
                &mut progress,
            )
            .map_err(|e| encode_stop(seams, e))?;
        }
        ExportFormat::Webp => {
            let webp = if alpha {
                encode_webp_rgba(&decoded.into_rgba8(), &mut progress)
            } else {
                encode_webp_with(&*seams.webp, &decoded.into_rgb8(), &mut progress)
            }
            .map_err(|e| encode_stop(seams, e))?;
            fallback = webp.lossless_fallback;
            if progress(Phase::Write, 0.0).is_break() {
                return Err(Stop::Canceled);
            }
            out.write_all(&webp.bytes)
                .map_err(|e| io_stop(seams, "writing the export", &e))?;
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
    let dest = job.dest.path().to_path_buf();
    let dir = dest
        .parent()
        .ok_or_else(|| failure(ErrorCode::Internal, "no destination folder"))?;
    let dest_unavailable = |e: String| Stop::Failed(CoreError::new(ErrorCode::DestUnavailable, e));
    match &job.dest {
        Dest::Exports { .. } => {
            fs::create_dir_all(dir)
                .map_err(|e| dest_unavailable(format!("creating {}: {e}", dir.display())))?;
            sweep_stale_temps(dir);
        }
        // FR-EXP-31: never made, never swept; only this export's temp is removed there.
        Dest::SaveAs(_) if !dir.is_dir() => {
            return Err(dest_unavailable(format!("{} is not there", dir.display())));
        }
        Dest::SaveAs(_) => {}
    }
    let stack = match &job.input {
        Input::Stack {
            project_dir,
            original,
            layers,
            ..
        } => Some(layer_stack(project_dir, original, layers, &mut r)?),
        Input::File { .. } => None,
    };

    let tmp = crate::fsx::temp_path(ctx, &dest);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| match (&job.dest, e.kind()) {
            (Dest::SaveAs(_), io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound) => {
                dest_unavailable(format!("{} can't be written: {e}", dir.display()))
            }
            _ => io_stop(seams, "creating the temp file", &e),
        })?;
    let result = (|| {
        let fallback = match (&job.input, &stack) {
            (Input::Stack { rescale, .. }, Some(stack)) => {
                encode_into(job, *rescale, stack, &file, &mut r)?
            }
            (Input::File { path, copy: true }, _) => {
                copy_into(path, &file, seams, &mut r)?;
                false
            }
            (Input::File { path, .. }, _) => convert_into(job, path, &file, &mut r)?,
            (Input::Stack { .. }, None) => {
                return Err(failure(ErrorCode::Internal, "no layer stack"));
            }
        };
        r.check(Phase::Write, 0.0)?;
        file.sync_all()
            .map_err(|e| io_stop(seams, "flushing the export", &e))?;
        drop(file);
        ctx.faults
            .hit(FaultPoint::AfterTemp, &dest)
            .map_err(|e| io_stop(seams, "writing the export", &e))?;
        r.check(Phase::Write, 0.5)?;
        rename_retrying(seams, &tmp, &dest, &mut r)?;
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

/// Commits `last` (FR-EXP-27, 30) if the export records one and its project is still open; false
/// if it couldn't.
fn commit_last(ctx: &Context, state: &mut State, job: &ExportJob, done: &Done) -> bool {
    let (
        Dest::Exports { rel_path, .. },
        Input::Stack {
            layers,
            head_revision,
            ..
        },
        super::Scope::Project(project_id),
    ) = (&job.dest, &job.input, job.scope)
    else {
        return false;
    };
    let install_id = state.settings.install_id;
    let Some(p) = state
        .project
        .as_mut()
        .filter(|p| p.manifest.project_id == project_id)
    else {
        return false;
    };
    let mut fields = Map::new();
    fields.insert(
        "last".into(),
        json!({
            "at": stamp(ctx.now()),
            "revision_id": head_revision,
            "layers_hash": layers_hash(layers),
            "path": rel_path,
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

/// FR-EXP-31: the folder of a Save as that succeeded becomes the last Save as folder (app data,
/// never a project file). A failed settings write is only logged: the file is written.
fn remember_save_dir(ctx: &Context, state: &mut State, path: &Path) {
    let Some(dir) = path.parent() else {
        return;
    };
    if state.settings.export.last_save_dir.as_deref() == Some(dir) {
        return;
    }
    let mut next = state.settings.clone();
    next.export.last_save_dir = Some(dir.to_path_buf());
    match crate::settings::save(ctx, &next) {
        Ok(()) => state.settings = next,
        Err(e) => tracing::warn!(error = %e, "the last Save as folder wasn't saved"),
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
                let saved = match &job.dest {
                    Dest::Exports { record: true, .. } => commit_last(ctx, &mut s, &job, &done),
                    // Nothing to record (FR-EXP-31, 32).
                    Dest::Exports { .. } => true,
                    Dest::SaveAs(path) => {
                        remember_save_dir(ctx, &mut s, path);
                        true
                    }
                };
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

#[cfg(test)]
mod heartbeat_tests {
    use super::*;

    #[test]
    fn fr_exp_19_heartbeat_ticks_while_a_hookless_step_runs() {
        let mut ticks = 0;
        let out = with_heartbeat(
            Duration::from_millis(50),
            || {
                ticks += 1;
                ControlFlow::Continue(())
            },
            || {
                std::thread::sleep(Duration::from_millis(600));
                7
            },
        );
        assert_eq!(out, ControlFlow::Continue(7));
        // Nominally 11; a loaded runner may skip some, but never most.
        assert!(ticks >= 4, "{ticks} ticks in 600 ms at 50 ms");
    }

    #[test]
    fn fr_exp_20_heartbeat_keeps_a_cancel_until_the_step_ends() {
        let ended = AtomicBool::new(false);
        let mut ticks = 0;
        let out = with_heartbeat(
            Duration::from_millis(20),
            || {
                ticks += 1;
                ControlFlow::Break(())
            },
            || {
                std::thread::sleep(Duration::from_millis(150));
                ended.store(true, Ordering::SeqCst);
            },
        );
        assert_eq!(out, ControlFlow::Break(()));
        assert!(ended.load(Ordering::SeqCst));
        assert_eq!(ticks, 1, "no ticks after the cancel");
    }
}

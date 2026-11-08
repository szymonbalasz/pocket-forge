//! Home-grid thumbnails `.pforge/thumb.jpg` (FR-PRJ-50, OQ-PRJ-13 A): the working image,
//! ≤ 512 px long side, JPEG q80. Written in the background after creation and after each commit
//! that changes `layers[]`, temp → rename, outside the commit; it never blocks one. Missing or
//! older than the last layer change → rebuilt on open.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use serde_json::json;
use ulid::Ulid;

use crate::context::{Context, EventSink};
use crate::fsx;
use crate::manifest::{LayerKind, Manifest, REVISIONS_DIR, THUMB_FILE};

pub const THUMB_MAX_SIDE: u32 = 512;
pub const THUMB_QUALITY: u8 = 80;

/// One thumbnail to write. Built on the calling thread from the committed manifest.
#[derive(Debug, Clone)]
pub struct ThumbJob {
    pub project_id: Ulid,
    pub dir: PathBuf,
    pub original: PathBuf,
    /// Visible patch layers in draw order: file, top-left, opacity.
    pub patches: Vec<(PathBuf, i64, i64, f32)>,
    /// The temp file name, from the ID seam.
    pub temp: PathBuf,
}

impl ThumbJob {
    pub fn new(ctx: &Context, dir: &Path, m: &Manifest) -> Self {
        let patches = m
            .layers
            .iter()
            .filter(|l| l.kind == LayerKind::Patch && l.visible)
            .filter_map(|l| {
                let (file, rect) = (l.file.as_ref()?, l.rect?);
                Some((dir.join(file), rect.x, rect.y, l.opacity as f32))
            })
            .collect();
        let target = dir.join(THUMB_FILE);
        ThumbJob {
            project_id: m.project_id,
            dir: dir.to_path_buf(),
            original: dir.join(&m.original.file),
            patches,
            temp: fsx::temp_path(ctx, &target),
        }
    }

    /// Renders and writes the thumbnail. Errors are returned for logging only.
    pub fn run(&self) -> Result<(), String> {
        let patches: Vec<_> = self
            .patches
            .iter()
            .map(|(path, x, y, opacity)| pf_imaging::thumbnail::ThumbPatch {
                path,
                x: *x,
                y: *y,
                opacity: *opacity,
            })
            .collect();
        let (jpeg, _, _) = pf_imaging::thumbnail::working_thumbnail_jpeg(
            &self.original,
            &patches,
            THUMB_MAX_SIDE,
            THUMB_QUALITY,
        )
        .map_err(|e| e.to_string())?;
        let target = self.dir.join(THUMB_FILE);
        let result = (|| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.temp)?;
            f.write_all(&jpeg)?;
            f.sync_all()?;
            drop(f);
            fsx::rename_with_retry(&self.temp, &target)
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&self.temp);
            return Err(e.to_string());
        }
        Ok(())
    }
}

/// True if the thumbnail is missing or older than the newest revision file (every layer change
/// writes a revision). Compares file times only, so it works under a fake clock.
pub fn needs_rebuild(dir: &Path) -> bool {
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let Some(thumb) = mtime(&dir.join(THUMB_FILE)) else {
        return true;
    };
    let newest = std::fs::read_dir(dir.join(REVISIONS_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| mtime(&e.path()))
        .max();
    newest.is_some_and(|n| n > thumb)
}

/// The thumbnail worker: one background thread, started on the first request. Requests for the
/// same project coalesce (only the newest is rendered).
#[derive(Clone)]
pub struct Thumbnails {
    tx: Arc<Mutex<Option<Sender<ThumbJob>>>>,
    events: Arc<dyn EventSink>,
    inline: bool,
}

impl std::fmt::Debug for Thumbnails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Thumbnails")
            .field("inline", &self.inline)
            .finish_non_exhaustive()
    }
}

impl Thumbnails {
    pub fn new(ctx: &Context) -> Self {
        Thumbnails {
            tx: Arc::default(),
            events: ctx.events.clone(),
            inline: ctx.inline_background,
        }
    }

    /// Queues a thumbnail. Never blocks on the render.
    pub fn request(&self, job: ThumbJob) {
        if self.inline {
            finish(&*self.events, &job, job.run());
            return;
        }
        let mut tx = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        if tx.is_none() {
            let (send, recv) = channel();
            let events = self.events.clone();
            let spawned = std::thread::Builder::new()
                .name("pf-thumbnails".into())
                .spawn(move || worker(&recv, &*events));
            if let Err(e) = spawned {
                tracing::warn!(error = %e, "can't start the thumbnail worker");
                return;
            }
            *tx = Some(send);
        }
        if let Some(send) = tx.as_ref()
            && send.send(job).is_err()
        {
            *tx = None;
        }
    }
}

fn worker(recv: &Receiver<ThumbJob>, events: &dyn EventSink) {
    while let Ok(first) = recv.recv() {
        // Coalesce whatever queued up meanwhile: the newest request per project wins.
        let mut latest: HashMap<PathBuf, ThumbJob> = HashMap::new();
        latest.insert(first.dir.clone(), first);
        while let Ok(more) = recv.try_recv() {
            latest.insert(more.dir.clone(), more);
        }
        for job in latest.into_values() {
            finish(events, &job, job.run());
        }
    }
}

fn finish(events: &dyn EventSink, job: &ThumbJob, result: Result<(), String>) {
    match result {
        Ok(()) => events.emit(
            "project:thumb-updated",
            json!({ "project_id": job.project_id }),
        ),
        Err(e) => tracing::warn!(dir = %job.dir.display(), error = %e, "thumbnail not written"),
    }
}

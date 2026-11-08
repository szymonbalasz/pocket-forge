//! Pack thumbnails `References/<pack>/.pforge/thumb.jpg` (FR-REF-13): a mosaic of the first
//! four **local** images (≤ 512 px, JPEG q80), built in the background after an add, a remove
//! or a listing that finds it missing or older than the newest image. Never hydrates a file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::json;
use ulid::Ulid;

use super::*;
use crate::context::EventSink;
use crate::fsx;

pub const PACK_THUMB_FILE: &str = "thumb.jpg";
pub const PACK_THUMB_SIDE: u32 = 512;
pub const PACK_THUMB_QUALITY: u8 = 80;
/// Local images handed to the mosaic: the first four that decode are used.
const MAX_SOURCES: usize = 8;

/// One thumbnail to write, built on the calling thread.
#[derive(Debug, Clone)]
pub struct PackThumbJob {
    pub pack_id: Ulid,
    pub dir: PathBuf,
    /// Local images, by name.
    pub sources: Vec<PathBuf>,
    pub temp: PathBuf,
}

impl PackThumbJob {
    fn target(&self) -> PathBuf {
        self.dir.join(PACK_APP_DIR).join(PACK_THUMB_FILE)
    }

    /// Renders and writes the thumbnail; with no usable image, removes a stale one.
    pub fn run(&self) -> Result<bool, String> {
        let target = self.target();
        let jpeg = match pf_imaging::reference::mosaic_jpeg(
            &self.sources,
            PACK_THUMB_SIDE,
            PACK_THUMB_QUALITY,
        ) {
            Ok(j) => j,
            Err(pf_imaging::reference::MosaicError::NoImage) => {
                let _ = std::fs::remove_file(&target);
                return Ok(false);
            }
            Err(e) => return Err(e.to_string()),
        };
        let app_dir = self.dir.join(PACK_APP_DIR);
        if !app_dir.is_dir() {
            std::fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;
            fsx::set_hidden(&app_dir);
        }
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
        Ok(true)
    }
}

/// True if the thumbnail is missing or older than the newest image (file times only).
pub fn needs_rebuild(thumb: &Path, images: &[(String, std::fs::Metadata)]) -> bool {
    let newest = images.iter().filter_map(|(_, m)| m.modified().ok()).max();
    match std::fs::metadata(thumb).and_then(|m| m.modified()) {
        Ok(t) => newest.is_some_and(|n| n > t),
        Err(_) => newest.is_some(),
    }
}

/// Pack folder → the job queued behind the running one (coalesced: the newest wins).
type Pending = HashMap<PathBuf, Option<PackThumbJob>>;
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

/// Queues a thumbnail for the pack folder. Cloud-only images are left out (never hydrated).
/// Inline in tests (`inline_background`), else on a background thread per pack.
pub fn request(ctx: &Context, dir: &Path, pack_id: Ulid) {
    let sources: Vec<PathBuf> = image_files(dir)
        .into_iter()
        .map(|(n, _)| dir.join(n))
        .filter(|p| !ctx.fs.is_cloud_only(p))
        .take(MAX_SOURCES)
        .collect();
    let target = dir.join(PACK_APP_DIR).join(PACK_THUMB_FILE);
    let job = PackThumbJob {
        pack_id,
        dir: dir.to_path_buf(),
        sources,
        temp: fsx::temp_path(ctx, &target),
    };
    let events = ctx.events.clone();
    if ctx.inline_background {
        finish(&*events, &job, job.run());
        return;
    }
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let map = pending.get_or_insert_with(HashMap::new);
    if let Some(slot) = map.get_mut(&job.dir) {
        *slot = Some(job);
        return;
    }
    map.insert(job.dir.clone(), None);
    let spawned = std::thread::Builder::new()
        .name("pf-pack-thumb".into())
        .spawn(move || worker(job, events));
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "can't start a pack thumbnail");
        if let Some(map) = pending.as_mut() {
            map.remove(dir);
        }
    }
}

fn worker(mut job: PackThumbJob, events: Arc<dyn EventSink>) {
    loop {
        finish(&*events, &job, job.run());
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        let Some(map) = pending.as_mut() else {
            return;
        };
        match map.get_mut(&job.dir).and_then(Option::take) {
            Some(next) => job = next,
            None => {
                map.remove(&job.dir);
                return;
            }
        }
    }
}

fn finish(events: &dyn EventSink, job: &PackThumbJob, result: Result<bool, String>) {
    match result {
        Ok(_) => events.emit("ref:thumb-updated", json!({ "pack_id": job.pack_id })),
        Err(e) => {
            tracing::warn!(dir = %job.dir.display(), error = %e, "pack thumbnail not written");
        }
    }
}

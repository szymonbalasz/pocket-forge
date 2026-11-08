//! EDT's local tile cache and the `pf-tiles` registry (EDT FR-07 … FR-11, FR-31, FR-32, R5, R16).
//!
//! - Cache: `<data-dir>/cache/tiles/v<builder>/<key>/` holds a pyramid (`pyramid.json` + one
//!   folder per level) or a downscaled copy (`image.png`), plus an `opened` stamp for eviction.
//!   It is rebuildable and safe to delete at any time. Capped at 2 GiB: the least recently opened
//!   entries that aren't served right now go first.
//! - `pyramid.json` is the per-level completion marker: a level's `complete` flag is set by an
//!   atomic rewrite only after all its tiles are written, so a partial level is never served and
//!   is rebuilt on the next open (FR-EDT-10).
//! - [`Served`] is the scheme registry, behind its **own** lock: tile requests never wait for a
//!   running command or a build (only for a brief map lookup). Lock order: state, then served.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::UNIX_EPOCH;

use pf_imaging::pyramid::{self, BUILDER_VERSION, LevelSpec, TILE_SIZE};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ulid::Ulid;

use crate::context::{EventSink, FaultPoint, Faults};
use crate::core::Core;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx::TEMP_MARKER;
use crate::images::{self, ImageBody, ImageRegistry, SchemePath, SchemeResponse, ServedImage};

/// FR-EDT-11 / R5.
pub const CACHE_CAP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// FR-EDT-31: at most 4 transient images per project, ≤ 256 MiB together.
pub const TRANSIENT_MAX: usize = 4;
pub const TRANSIENT_MAX_BYTES: usize = 256 * 1024 * 1024;
/// `edt_image_open` without `max_side` serves whole images up to this side; above → `use_tiles`.
pub const WHOLE_IMAGE_MAX_SIDE: u32 = 4096;

const PYRAMID_FILE: &str = "pyramid.json";
const OPENED_FILE: &str = "opened";
const COPY_FILE: &str = "image.png";

// ---------------------------------------------------------------- keys

/// FNV-1a 64 (Appendix B: in-crate, no new dependency).
pub fn fnv1a64(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        // Length-prefix each part so concatenations can't collide.
        for b in (part.len() as u64).to_le_bytes().iter().chain(part.iter()) {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// What a cache key names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Pyramid,
    /// A whole image as-is (`None`) or a `max_side` copy.
    Image(Option<u32>),
}

/// `<image-id>-<16 hex>` over image id, source size, source mtime, builder version and kind.
/// Changes whenever any of them changes (FR-EDT-10).
pub fn cache_key(image_id: Ulid, size: u64, mtime_ns: u64, kind: KeyKind) -> String {
    let kind_bytes = match kind {
        KeyKind::Pyramid => b"pyramid".to_vec(),
        KeyKind::Image(None) => b"image".to_vec(),
        KeyKind::Image(Some(m)) => [&b"max_side"[..], &m.to_le_bytes()].concat(),
    };
    let h = fnv1a64(&[
        &image_id.to_bytes(),
        &size.to_le_bytes(),
        &mtime_ns.to_le_bytes(),
        &BUILDER_VERSION.to_le_bytes(),
        &kind_bytes,
    ]);
    format!("{image_id}-{h:016x}")
}

pub fn mtime_ns(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64)
}

/// `<data-dir>/cache/tiles/v<builder>/`.
pub fn cache_root(data_dir: &Path) -> PathBuf {
    data_dir
        .join("cache")
        .join("tiles")
        .join(format!("v{BUILDER_VERSION}"))
}

// ---------------------------------------------------------------- pyramid.json

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceStamp {
    pub size: u64,
    pub mtime_ns: u64,
}

/// One level as the UI and `pyramid.json` see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelEntry {
    pub level: u32,
    /// Level px per image px (2⁻ⁿ).
    pub scale: f64,
    pub cols: u32,
    pub rows: u32,
    pub ext: String,
    pub complete: bool,
}

/// `pyramid.json` (Appendix B).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PyramidFile {
    pub key: String,
    pub width: u32,
    pub height: u32,
    pub tile_size: u32,
    pub builder_version: u32,
    pub source: SourceStamp,
    pub levels: Vec<LevelEntry>,
}

impl PyramidFile {
    fn fresh(key: &str, width: u32, height: u32, source: SourceStamp) -> Self {
        PyramidFile {
            key: key.into(),
            width,
            height,
            tile_size: TILE_SIZE,
            builder_version: BUILDER_VERSION,
            source,
            levels: pyramid::levels(width, height)
                .iter()
                .map(|s| LevelEntry {
                    level: s.level,
                    scale: s.scale(),
                    cols: s.cols,
                    rows: s.rows,
                    ext: s.ext().into(),
                    complete: false,
                })
                .collect(),
        }
    }

    /// The stored file if it describes exactly this pyramid, else a fresh one (nothing built).
    fn load_or_fresh(dir: &Path, key: &str, width: u32, height: u32, source: SourceStamp) -> Self {
        let fresh = PyramidFile::fresh(key, width, height, source);
        let stored = std::fs::read(dir.join(PYRAMID_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<PyramidFile>(&b).ok());
        // Equal except for the `complete` flags.
        let same = |s: &PyramidFile| {
            let mut s = s.clone();
            for (l, f) in s.levels.iter_mut().zip(&fresh.levels) {
                l.complete = f.complete;
            }
            s == fresh
        };
        match stored {
            Some(s) if same(&s) => s,
            _ => fresh,
        }
    }
}

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Temp + rename in the same folder (the cache is local, so no lock retries are needed).
fn write_atomic(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = target.with_file_name(format!(
        ".{name}{TEMP_MARKER}{}-{}",
        std::process::id(),
        TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let r = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, target)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn write_pyramid_file(dir: &Path, file: &PyramidFile) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(file).map_err(std::io::Error::other)?;
    write_atomic(&dir.join(PYRAMID_FILE), &bytes)
}

fn touch_opened(dir: &Path, now_ms: i64) {
    if let Err(e) = std::fs::write(dir.join(OPENED_FILE), now_ms.to_string()) {
        tracing::debug!(error = %e, "tile cache: could not stamp the open time");
    }
}

// ---------------------------------------------------------------- the served registry

#[derive(Debug)]
struct PyramidEntry {
    dir: PathBuf,
    levels: Vec<(LevelSpec, bool)>,
}

/// What the `pf-tiles` scheme may serve: only keys registered for the open asset project.
#[derive(Debug)]
pub struct Served {
    project_id: Option<Ulid>,
    pyramids: HashMap<String, PyramidEntry>,
    images: ImageRegistry,
    /// EDT's `library` key space (OQ-PRJ-16): files outside the open project (home-grid
    /// thumbnails, user-folder files). Survives project open and close.
    library: ImageRegistry,
    /// Transient keys, oldest first, with their sizes.
    transient: VecDeque<(String, usize)>,
    /// Pyramid keys with a build in flight (idempotent `edt_tiles_open`).
    building: HashSet<String>,
    /// The disk cache cap (tests lower it).
    pub cap_bytes: u64,
}

impl Default for Served {
    fn default() -> Self {
        Served {
            project_id: None,
            pyramids: HashMap::new(),
            images: ImageRegistry::default(),
            library: ImageRegistry::default(),
            transient: VecDeque::new(),
            building: HashSet::new(),
            cap_bytes: CACHE_CAP_BYTES,
        }
    }
}

impl Served {
    /// Unregisters every key (project open or close); then serves only `project_id`'s keys.
    /// Running builds notice and stop at their next level.
    pub fn reset(&mut self, project_id: Option<Ulid>) {
        self.project_id = project_id;
        self.pyramids.clear();
        self.images.clear();
        self.transient.clear();
    }

    pub fn is_building(&self, key: &str) -> bool {
        self.building.contains(key)
    }

    fn in_use(&self) -> HashSet<String> {
        self.pyramids
            .keys()
            .chain(self.building.iter())
            .cloned()
            .chain(self.images.keys())
            .collect()
    }
}

pub(crate) type SharedServed = Arc<Mutex<Served>>;

fn lock(s: &Mutex<Served>) -> MutexGuard<'_, Served> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

/// Serves a `pf-tiles` request path. File reads happen outside the lock.
pub fn serve(served: &Mutex<Served>, path: &str) -> SchemeResponse {
    match images::parse_path(path) {
        None => SchemeResponse::status(400),
        Some(SchemePath::Image { key, ext }) => {
            let img = {
                let s = lock(served);
                s.images.get(&key).or_else(|| s.library.get(&key)).cloned()
            };
            match img {
                Some(img) if img.ext == ext => images::read_image(&key, &img),
                _ => SchemeResponse::status(404),
            }
        }
        Some(SchemePath::Tile {
            key,
            level,
            col,
            row,
            ext,
        }) => {
            let found = {
                let s = lock(served);
                s.pyramids.get(&key).and_then(|p| {
                    let (spec, complete) = *p.levels.get(level as usize)?;
                    (col < spec.cols && row < spec.rows && ext == spec.ext())
                        .then(|| (pyramid::tile_path(&p.dir, &spec, col, row), spec, complete))
                })
            };
            match found {
                None => SchemeResponse::status(404),
                Some((_, _, false)) => SchemeResponse::status(425),
                Some((file, spec, true)) => match std::fs::read(&file) {
                    Ok(body) => images::ok(spec.mime(), body),
                    Err(e) => {
                        tracing::warn!(key, level, col, row, error = %e, "pf-tiles: tile unreadable");
                        SchemeResponse::status(404)
                    }
                },
            }
        }
    }
}

// ---------------------------------------------------------------- resolving images

/// An image of the open project, resolved to a file.
struct Resolved {
    path: PathBuf,
    size: u64,
    mtime_ns: u64,
    info: pf_imaging::ImageInfo,
}

fn probe_error(e: pf_imaging::ProbeError) -> CoreError {
    match e {
        pf_imaging::ProbeError::Io(io) => CoreError::io("reading the image", &io),
        e => CoreError::new(probe_error_code(&e), e.to_string()),
    }
}

/// The image must belong to the open project: the original, a layer, a candidate or a mask.
fn resolve(core: &Core, project_id: Ulid, image_id: Ulid) -> CoreResult<Resolved> {
    let path = {
        let state = core.state();
        let p = state
            .project
            .as_ref()
            .ok_or_else(|| CoreError::new(ErrorCode::NoProject, "no asset project is open"))?;
        if p.manifest.project_id != project_id {
            return Err(
                CoreError::new(ErrorCode::UnknownId, "that project isn't open")
                    .with_field("project_id"),
            );
        }
        // PRJ `resolve_image` (#48 (3)): the original by its stable id, layers, candidates.
        p.resolve_image(image_id)?.path
    };
    let meta = std::fs::metadata(&path).map_err(|e| CoreError::io("reading the image", &e))?;
    let info = pf_imaging::probe_file(&path).map_err(probe_error)?;
    Ok(Resolved {
        size: meta.len(),
        mtime_ns: mtime_ns(&meta),
        path,
        info,
    })
}

// ---------------------------------------------------------------- edt_tiles_open

/// `edt_tiles_open` result: the descriptor, at once (FR-EDT-07).
#[derive(Debug, Clone, Serialize)]
pub struct TilesOpenOutput {
    pub key: String,
    /// `<scheme>/t/<key>`: a tile is `${base_url}/${level}/${col}_${row}.${ext}`.
    pub base_url: String,
    pub width: u32,
    pub height: u32,
    pub tile_size: u32,
    /// Ascending by level (0 = 1:1).
    pub levels: Vec<LevelEntry>,
}

pub fn tiles_open(
    core: &Core,
    project_id: Ulid,
    image_id: Ulid,
    rebuild: bool,
) -> CoreResult<TilesOpenOutput> {
    let r = resolve(core, project_id, image_id)?;
    let (w, h) = (r.info.width, r.info.height);
    let key = cache_key(image_id, r.size, r.mtime_ns, KeyKind::Pyramid);
    let dir = cache_root(&core.ctx.data_dir).join(&key);
    let building = lock(&core.served).is_building(&key);
    if rebuild && !building {
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(CoreError::io("clearing the tile cache", &e));
            }
            _ => {}
        }
    }
    let source = SourceStamp {
        size: r.size,
        mtime_ns: r.mtime_ns,
    };
    let mut file = PyramidFile::load_or_fresh(&dir, &key, w, h, source.clone());
    std::fs::create_dir_all(&dir).map_err(|e| CoreError::io("creating the tile cache", &e))?;
    if !building {
        write_pyramid_file(&dir, &file).map_err(|e| CoreError::io("writing pyramid.json", &e))?;
    }
    touch_opened(&dir, core.ctx.now().millis());

    let specs = pyramid::levels(w, h);
    let start_build = {
        let mut s = lock(&core.served);
        if s.project_id != Some(project_id) {
            return Err(CoreError::new(
                ErrorCode::UnknownId,
                "that project isn't open",
            ));
        }
        if s.building.contains(&key) {
            // The build may have marked levels since we read the file: re-read under the lock
            // (the builder writes the file before it takes the lock).
            file = PyramidFile::load_or_fresh(&dir, &key, w, h, source);
        }
        s.pyramids.insert(
            key.clone(),
            PyramidEntry {
                dir: dir.clone(),
                levels: specs
                    .iter()
                    .zip(&file.levels)
                    .map(|(spec, l)| (*spec, l.complete))
                    .collect(),
            },
        );
        let start = !s.building.contains(&key) && file.levels.iter().any(|l| !l.complete);
        if start {
            s.building.insert(key.clone());
        }
        start
    };
    let out = TilesOpenOutput {
        base_url: format!("{}/t/{key}", core.ctx.tiles_base_url),
        key: key.clone(),
        width: w,
        height: h,
        tile_size: TILE_SIZE,
        levels: file.levels.clone(),
    };
    if start_build {
        let job = BuildJob {
            served: core.served.clone(),
            events: core.ctx.events.clone(),
            faults: core.ctx.faults.clone(),
            root: cache_root(&core.ctx.data_dir),
            src: r.path,
            dir,
            key,
            file,
        };
        std::thread::Builder::new()
            .name("pf-tiles-build".into())
            .spawn(move || job.run())
            .map_err(|e| CoreError::internal(format!("starting the tile build: {e}")))?;
    }
    Ok(out)
}

/// A background pyramid build: missing levels, smallest first.
struct BuildJob {
    served: SharedServed,
    events: Arc<dyn EventSink>,
    faults: Faults,
    root: PathBuf,
    src: PathBuf,
    dir: PathBuf,
    key: String,
    file: PyramidFile,
}

impl BuildJob {
    fn run(mut self) {
        let needed: Vec<bool> = self.file.levels.iter().map(|l| !l.complete).collect();
        let size = (self.file.width, self.file.height);
        let (src, dir) = (self.src.clone(), self.dir.clone());
        let result = pyramid::build(&src, &dir, size, &needed, &mut |level| {
            self.level_done(level)
        });
        let in_use = {
            let mut s = lock(&self.served);
            s.building.remove(&self.key);
            s.in_use()
        };
        match result {
            Ok(()) => {
                self.events
                    .emit("edt:tiles-ready", json!({ "key": self.key }));
                evict(&self.root, lock(&self.served).cap_bytes, &in_use);
            }
            Err(e) => {
                let still_open = lock(&self.served).pyramids.contains_key(&self.key);
                tracing::warn!(key = self.key, error = %e, "tile pyramid build stopped");
                if still_open {
                    let code = match &e {
                        pyramid::BuildError::Source(p) => probe_error_code(p),
                        pyramid::BuildError::SizeChanged { .. } => ErrorCode::Corrupt,
                        pyramid::BuildError::Encode(_) => ErrorCode::Internal,
                        pyramid::BuildError::Io(_) | pyramid::BuildError::Stopped(_) => {
                            ErrorCode::Io
                        }
                    };
                    self.events.emit(
                        "edt:tiles-failed",
                        json!({ "key": self.key, "error": { "code": code, "message": e.to_string() } }),
                    );
                }
            }
        }
    }

    /// Marks a level complete (pyramid.json first, then the registry), reports it, then checks
    /// faults and whether the pyramid is still wanted.
    fn level_done(&mut self, level: u32) -> Result<(), String> {
        let l = level as usize;
        self.file.levels[l].complete = true;
        write_pyramid_file(&self.dir, &self.file).map_err(|e| e.to_string())?;
        let still_open = {
            let mut s = lock(&self.served);
            match s.pyramids.get_mut(&self.key) {
                Some(p) => {
                    p.levels[l].1 = true;
                    true
                }
                None => false,
            }
        };
        self.events.emit(
            "edt:tiles-progress",
            json!({ "key": self.key, "level": level }),
        );
        self.faults
            .hit(FaultPoint::AfterLevel(level), &self.dir)
            .map_err(|e| e.to_string())?;
        if still_open {
            Ok(())
        } else {
            Err("the asset project was closed".into())
        }
    }
}

fn probe_error_code(e: &pf_imaging::ProbeError) -> ErrorCode {
    use pf_imaging::ProbeError as P;
    match e {
        P::Unsupported => ErrorCode::Unsupported,
        P::Animated => ErrorCode::Animated,
        P::TooLarge { .. } => ErrorCode::TooLarge,
        P::Corrupt(_) => ErrorCode::Corrupt,
        P::Io(_) => ErrorCode::Io,
    }
}

// ---------------------------------------------------------------- eviction

fn dir_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            _ => e.metadata().map_or(0, |m| m.len()),
        })
        .sum()
}

/// FR-EDT-11: while the cache is over `cap`, deletes the least recently opened entries that
/// aren't in use. Returns the keys removed.
pub fn evict(root: &Path, cap: u64, in_use: &HashSet<String>) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return vec![];
    };
    let mut entries: Vec<(i64, u64, String, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| {
            let path = e.path();
            let opened = std::fs::read_to_string(path.join(OPENED_FILE))
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            let key = e.file_name().to_string_lossy().into_owned();
            (opened, dir_size(&path), key, path)
        })
        .collect();
    let mut total: u64 = entries.iter().map(|e| e.1).sum();
    entries.sort();
    let mut removed = vec![];
    for (_, size, key, path) in entries {
        if total <= cap {
            break;
        }
        if in_use.contains(&key) {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                total -= size;
                removed.push(key);
            }
            Err(e) => tracing::warn!(key, error = %e, "tile cache: could not evict"),
        }
    }
    removed
}

// ---------------------------------------------------------------- edt_image_open

/// `edt_image_open` and transient registration result.
#[derive(Debug, Clone, Serialize)]
pub struct ImageOpenOutput {
    pub key: String,
    pub url: String,
    pub width: u32,
    pub height: u32,
}

pub fn image_open(
    core: &Core,
    project_id: Ulid,
    image_id: Ulid,
    max_side: Option<u32>,
) -> CoreResult<ImageOpenOutput> {
    let r = resolve(core, project_id, image_id)?;
    let key = cache_key(image_id, r.size, r.mtime_ns, KeyKind::Image(max_side));
    let (img, width, height) = match max_side {
        None => {
            if r.info.width > WHOLE_IMAGE_MAX_SIDE || r.info.height > WHOLE_IMAGE_MAX_SIDE {
                return Err(CoreError::new(
                    ErrorCode::UseTiles,
                    format!(
                        "the image is larger than {WHOLE_IMAGE_MAX_SIDE} px per side; open it as tiles"
                    ),
                ));
            }
            let f = r.info.format;
            (
                ServedImage::file(r.path, f.mime(), f.extension()),
                r.info.width,
                r.info.height,
            )
        }
        Some(m) => {
            let dir = cache_root(&core.ctx.data_dir).join(&key);
            let file = dir.join(COPY_FILE);
            let (w, h) = match pf_imaging::probe_file(&file) {
                Ok(info) => (info.width, info.height),
                Err(_) => {
                    let (png, w, h) =
                        pf_imaging::thumbnail::downscaled_png(&r.path, m).map_err(probe_error)?;
                    std::fs::create_dir_all(&dir)
                        .and_then(|()| write_atomic(&file, &png))
                        .map_err(|e| CoreError::io("writing the image copy", &e))?;
                    (w, h)
                }
            };
            touch_opened(&dir, core.ctx.now().millis());
            (ServedImage::file(file, "image/png", "png"), w, h)
        }
    };
    let ext = img.ext;
    let in_use = {
        let mut s = lock(&core.served);
        if s.project_id != Some(project_id) {
            return Err(CoreError::new(
                ErrorCode::UnknownId,
                "that project isn't open",
            ));
        }
        s.images.register(key.clone(), img);
        s.in_use()
    };
    if max_side.is_some() {
        evict(
            &cache_root(&core.ctx.data_dir),
            lock(&core.served).cap_bytes,
            &in_use,
        );
    }
    Ok(ImageOpenOutput {
        url: format!("{}/i/{key}.{ext}", core.ctx.tiles_base_url),
        key,
        width,
        height,
    })
}

// ---------------------------------------------------------------- library images

/// Most `library` keys kept; past it the registry starts over (keys are re-registered by the
/// next listing).
pub const LIBRARY_MAX_KEYS: usize = 20_000;

/// OQ-PRJ-16 A, minimal: registers a PNG/JPEG/WebP file outside the open project under
/// `/i/l-<16 hex>.<ext>` and returns its URL. The key covers path, size and mtime, so it changes
/// when the file does. Reads metadata only (nothing is hydrated until the image is viewed,
/// FR-PRJ-17). `None` for a missing file or another extension.
pub fn register_library_image(core: &Core, path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let (mime, ext) = match ext.as_str() {
        "png" => ("image/png", "png"),
        "jpg" | "jpeg" => ("image/jpeg", "jpg"),
        "webp" => ("image/webp", "webp"),
        _ => return None,
    };
    let meta = std::fs::symlink_metadata(path).ok()?;
    let h = fnv1a64(&[
        path.to_string_lossy().as_bytes(),
        &meta.len().to_le_bytes(),
        &mtime_ns(&meta).to_le_bytes(),
    ]);
    let key = format!("l-{h:016x}");
    let mut s = lock(&core.served);
    if s.library.get(&key).is_none() {
        if s.library.keys().count() >= LIBRARY_MAX_KEYS {
            s.library.clear();
        }
        s.library.register(
            key.clone(),
            ServedImage::file(path.to_path_buf(), mime, ext),
        );
    }
    Some(format!("{}/i/{key}.{ext}", core.ctx.tiles_base_url))
}

// ---------------------------------------------------------------- transient images

/// FR-EDT-31 core fn (not a command): registers an in-memory PNG/JPEG/WebP for the open project
/// and returns its key and URL (`/i/<key>.<ext>`). A fifth image, or going over 256 MiB,
/// unregisters the oldest. All are unregistered when the project closes.
pub fn register_transient_image(
    core: &Core,
    project_id: Ulid,
    bytes: Vec<u8>,
) -> CoreResult<ImageOpenOutput> {
    let info = pf_imaging::probe_bytes(&bytes).map_err(probe_error)?;
    if bytes.len() > TRANSIENT_MAX_BYTES {
        return Err(CoreError::new(
            ErrorCode::TooLarge,
            "a transient image is at most 256 MiB",
        ));
    }
    {
        let state = core.state();
        if state.project.as_ref().map(|p| p.manifest.project_id) != Some(project_id) {
            return Err(
                CoreError::new(ErrorCode::UnknownId, "that project isn't open")
                    .with_field("project_id"),
            );
        }
    }
    let key = format!("t-{}", core.ctx.new_id());
    let (mime, ext) = (info.format.mime(), info.format.extension());
    let len = bytes.len();
    let mut s = lock(&core.served);
    s.images.register(
        key.clone(),
        ServedImage {
            body: ImageBody::Bytes(Arc::new(bytes)),
            mime,
            ext,
        },
    );
    s.transient.push_back((key.clone(), len));
    while s.transient.len() > TRANSIENT_MAX
        || s.transient.iter().map(|t| t.1).sum::<usize>() > TRANSIENT_MAX_BYTES
    {
        if let Some((old, _)) = s.transient.pop_front() {
            s.images.remove(&old);
        }
    }
    Ok(ImageOpenOutput {
        url: format!("{}/i/{key}.{ext}", core.ctx.tiles_base_url),
        key,
        width: info.width,
        height: info.height,
    })
}

#[cfg(test)]
mod tests;

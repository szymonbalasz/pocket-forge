//! Reference packs (REF, #51): folders `<library-root>/References/<pack>/` of images plus a
//! `pack.json` (schema §4.12), the References trash (§4.13) and pack thumbnails.
//!
//! A pack is a plain folder that works in Explorer: files dropped there appear in the app and
//! originals are never touched (#16). The pack name is the folder name (OQ-REF-01 A); drafts
//! and jobs refer to a pack by `pack_id`, so renames keep them valid (FR-REF-03).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use pf_provider::ReferenceRole;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::library;
use crate::manifest::Extra;
use crate::naming;
use crate::settings::Settings;
use crate::time::Timestamp;

pub mod commands;
pub mod packs;
pub mod thumb;
pub mod trash;

#[cfg(test)]
mod tests;

/// The folder in every root that holds the packs (FR-PRJ-51). Not a section.
pub const REFERENCES_DIR: &str = "References";
/// A pack's metadata file (§4.12).
pub const PACK_FILE: &str = "pack.json";
/// A pack's app folder (thumbnail).
pub const PACK_APP_DIR: &str = ".pforge";
/// `pack.json` `schema_version`.
pub const PACK_SCHEMA_VERSION: u32 = 1;
/// Longest pack notes, in characters (FR-REF-03).
pub const MAX_NOTES_CHARS: usize = 2000;

/// Serialises every write to packs and the References trash (one app process). Never held
/// together with the core state lock.
pub(crate) static WRITE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn write_lock() -> std::sync::MutexGuard<'static, ()> {
    WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// One image's entry in `pack.json` `images{}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageMeta {
    /// The image's default role (FR-REF-11).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<ReferenceRole>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// §4.12 `pack.json`. Unknown fields are kept and written after the known ones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackJson {
    pub schema_version: u32,
    pub pack_id: Ulid,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[serde(default)]
    pub notes: String,
    /// Per file name; entries for absent files are kept (§4.12).
    #[serde(default)]
    pub images: BTreeMap<String, ImageMeta>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl PackJson {
    pub fn new(ctx: &Context) -> Self {
        let now = ctx.now();
        PackJson {
            schema_version: PACK_SCHEMA_VERSION,
            pack_id: ctx.new_id(),
            created_at: now,
            updated_at: now,
            notes: String::new(),
            images: BTreeMap::new(),
            extra: Extra::new(),
        }
    }

    /// The default role of `file`, if `pack.json` sets one.
    pub fn role_of(&self, file: &str) -> Option<ReferenceRole> {
        self.images.get(file).and_then(|m| m.role)
    }
}

/// Reads and parses a `pack.json`. Unparseable → `corrupt`.
pub fn read_pack_json(path: &Path) -> CoreResult<PackJson> {
    let bytes = std::fs::read(path)
        .map_err(|e| CoreError::io(format!("reading {}", path.display()), &e))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| corrupt(path, &e))?;
    serde_json::from_value(v).map_err(|e| corrupt(path, &e))
}

fn corrupt(path: &Path, e: &dyn std::fmt::Display) -> CoreError {
    CoreError::new(
        ErrorCode::Corrupt,
        format!("{} is unreadable: {e}", path.display()),
    )
}

/// `<root>/References`.
pub fn references_dir(root: &Path) -> PathBuf {
    root.join(REFERENCES_DIR)
}

/// Creates a missing `References/` in an existing root (FR-PRJ-51).
pub fn ensure_references_dir(root: &Path) -> CoreResult<PathBuf> {
    let dir = references_dir(root);
    if !dir.is_dir() {
        std::fs::create_dir(&dir)
            .map_err(|e| CoreError::io(format!("creating {}", dir.display()), &e))?;
    }
    Ok(dir)
}

/// The available roots (an unavailable root's packs aren't listed).
pub(crate) fn available_roots(ctx: &Context, settings: &Settings) -> Vec<PathBuf> {
    library::effective_roots(ctx, settings)
        .into_iter()
        .filter(|r| r.is_dir())
        .collect()
}

/// True for folder entries under `References/` that are packs: not hidden (`.pforge`, temp
/// folders), not a temp folder.
pub(crate) fn is_pack_dir_name(name: &str) -> bool {
    !name.starts_with('.') && !name.contains(crate::fsx::TEMP_MARKER)
}

/// The pack folders of one root, sorted by name (case-insensitive).
pub(crate) fn pack_dirs(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(references_dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, _)| is_pack_dir_name(n))
        .collect();
    out.sort_by(|a, b| {
        a.0.to_lowercase()
            .cmp(&b.0.to_lowercase())
            .then_with(|| a.0.cmp(&b.0))
    });
    out
}

/// A pack found by id.
#[derive(Debug, Clone)]
pub struct PackLoc {
    pub root: PathBuf,
    pub dir: PathBuf,
    pub name: String,
    pub json: PackJson,
}

impl PackLoc {
    /// `References/<pack>/<file>`: the job-record path (FR-REF-25).
    pub fn rel_path(&self, file: &str) -> String {
        format!("{REFERENCES_DIR}/{}/{file}", self.name)
    }
}

/// Finds a pack by `pack_id` in (root, name) order, so the first of two packs that share an
/// id wins (FR-REF-05). Reads `pack.json` files only; a cloud-only one is hydrated (FR-REF-06).
/// Doesn't adopt anything.
pub fn find_pack(ctx: &Context, settings: &Settings, pack_id: Ulid) -> Option<PackLoc> {
    for root in available_roots(ctx, settings) {
        for (name, dir) in pack_dirs(&root) {
            let file = dir.join(PACK_FILE);
            if library::hydrate_if_needed(ctx, &file).is_err() {
                continue;
            }
            if let Ok(json) = read_pack_json(&file)
                && json.pack_id == pack_id
            {
                return Some(PackLoc {
                    root,
                    dir,
                    name,
                    json,
                });
            }
        }
    }
    None
}

/// Finds a pack by folder name (case-insensitive), the fallback when an id is gone (FR-REF-26).
pub fn find_pack_by_name(ctx: &Context, settings: &Settings, name: &str) -> Option<PackLoc> {
    for root in available_roots(ctx, settings) {
        for (n, dir) in pack_dirs(&root) {
            if !naming::same_name(&n, name) {
                continue;
            }
            let file = dir.join(PACK_FILE);
            if library::hydrate_if_needed(ctx, &file).is_err() {
                continue;
            }
            if let Ok(json) = read_pack_json(&file) {
                return Some(PackLoc {
                    root,
                    dir,
                    name: n,
                    json,
                });
            }
        }
    }
    None
}

pub(crate) fn require_pack(
    ctx: &Context,
    settings: &Settings,
    pack_id: Ulid,
) -> CoreResult<PackLoc> {
    find_pack(ctx, settings, pack_id).ok_or_else(|| {
        CoreError::new(ErrorCode::UnknownId, format!("no reference pack {pack_id}"))
            .with_field("pack_id")
    })
}

/// The supported image files at the top level of `dir` by extension, sorted by name
/// (case-insensitive). Metadata only: nothing is opened, so nothing is hydrated (FR-REF-01).
pub fn image_files(dir: &Path) -> Vec<(String, std::fs::Metadata)> {
    let mut out: Vec<(String, std::fs::Metadata)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.contains(crate::fsx::TEMP_MARKER) {
                return None;
            }
            let meta = std::fs::symlink_metadata(e.path()).ok()?;
            (meta.is_file() && pf_imaging::has_image_extension(&e.path())).then_some((name, meta))
        })
        .collect();
    sort_names(&mut out, |x| &x.0);
    out
}

/// Case-insensitive name order, ties by exact name (FR-REF-16).
pub(crate) fn sort_names<T>(v: &mut [T], name: impl Fn(&T) -> &str) {
    v.sort_by(|a, b| {
        let (a, b) = (name(a), name(b));
        a.to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b))
    });
}

/// Name rules for packs (FR-REF-02 = FR-PRJ-44).
pub(crate) fn check_name(name: &str) -> CoreResult<()> {
    if naming::is_valid_name(name) {
        return Ok(());
    }
    Err(CoreError::new(
        ErrorCode::InvalidName,
        format!(
            "names can't contain <>:\"/\\|?* or control characters, start with a dot, end with a dot or space, be a reserved Windows name, or be longer than {} characters",
            naming::MAX_NAME_CHARS
        ),
    )
    .with_field("name"))
}

/// True if a pack folder (or any entry) under `refs_dir` has `name`, ignoring case
/// (FR-REF-02). `except` is a folder name to ignore (the pack being renamed).
pub(crate) fn name_taken(refs_dir: &Path, name: &str, except: Option<&str>) -> bool {
    std::fs::read_dir(refs_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .any(|n| naming::same_name(&n, name) && except.is_none_or(|x| !naming::same_name(&n, x)))
}

pub(crate) fn name_taken_error(name: &str) -> CoreError {
    CoreError::new(
        ErrorCode::NameTaken,
        format!("a pack named {name:?} already exists"),
    )
    .with_field("name")
}

/// `name` and `.ext` of a file name (`.ext` empty when there is none).
pub(crate) fn split_ext(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
        _ => (name.to_owned(), String::new()),
    }
}

/// The first free file name in `dir` for `name`: `name`, `name (2)`, … (FR-REF-08).
pub(crate) fn free_file_name(dir: &Path, name: &str) -> String {
    let (stem, ext) = split_ext(name);
    let taken = |c: &str| {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| naming::same_name(&e.file_name().to_string_lossy(), &format!("{c}{ext}")))
    };
    let free = naming::first_free(&stem, taken);
    format!("{free}{ext}")
}

/// The startup work for references (FR-REF-12, FR-REF-07): interrupted trash moves are rolled
/// forward, items older than 30 days are purged (clock seam), and pack creation temp folders
/// older than an hour are removed. Never touches anything else.
pub fn sweep_startup(ctx: &Context, settings: &Settings) {
    for root in available_roots(ctx, settings) {
        let refs = references_dir(&root);
        if !refs.is_dir() {
            continue;
        }
        crate::fsx::remove_stale_temp_dirs(&refs, library::STALE_TEMP_DIR_AGE);
        trash::roll_forward(ctx, settings, &root);
    }
    let (count, bytes) = trash::purge(ctx, settings);
    if count > 0 {
        ctx.emit(
            "ref:trash-purged",
            serde_json::json!({ "count": count, "bytes": bytes }),
        );
    }
}

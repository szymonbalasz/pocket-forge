//! The library trash (FR-PRJ-57, 58, #62, schema §4.15):
//! `<library-root>/.pforge/trash/<item-id>/item.json` plus the deleted project folder, whole, as
//! `<item-id>/<name>/`. `item.json` is written first, then the folder moves in one rename, so a
//! project is in its section or in the trash, never both or neither. A move that fails with an
//! error is rolled back at once. An interrupted delete (a kill) leaves the project in its
//! section and an item without a folder.
//!
//! R111: startup never moves a project into or out of the trash and never drops an item
//! because its folder is missing (a sync may still bring it, or the folder was taken back by
//! hand). Such an item is never listed, its restore is `not_found`, and only the purge removes
//! it, 30 days after `trashed_at` (clock seam). A purge marks the item (`purging_at`) before it
//! deletes, so a half-purged project is never offered for Restore. R112: startup doesn't
//! finish an interrupted restore either; it clears `restored_at` and the item is listed again.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use super::{effective_roots, normalise, not_a_project, require_available, root_of};
use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::manifest::{Extra, Kind, MANIFEST_FILE, PFORGE_DIR};
use crate::naming;
use crate::settings::Settings;
use crate::time::Timestamp;

/// How long a deleted project can be restored (#18, #62).
pub const TRASH_KEEP: Duration = Duration::from_secs(30 * 86_400);
/// `item.json` `schema_version`.
pub const ITEM_SCHEMA_VERSION: u32 = 1;
const ITEM_FILE: &str = "item.json";

/// What a library trash item holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashKind {
    /// A whole project folder, under `<item>/<name>/`.
    Project,
}

/// §4.15 `item.json`. Unknown fields are kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrashItem {
    pub schema_version: u32,
    pub id: Ulid,
    pub kind: TrashKind,
    /// `null` for a project whose `project.json` couldn't be read when it was deleted.
    pub project_id: Option<Ulid>,
    /// The section it came from and goes back to: `maps`, `tokens` or `items`.
    pub section: String,
    /// The project's folder name.
    pub name: String,
    pub trashed_at: Timestamp,
    /// Set first when a restore starts, so the item isn't listed or purged meanwhile. Cleared
    /// when the restore fails, and at startup for one that was interrupted (R112).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_at: Option<Timestamp>,
    /// Set first when the purge starts on the item (R111): it is never listed or restored
    /// again, and a purge that couldn't finish is retried at the next startup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purging_at: Option<Timestamp>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// The section spelling in `item.json` and in the commands (`maps`, `tokens`, `items`).
pub fn section_name(kind: Kind) -> String {
    kind.section_dir().to_ascii_lowercase()
}

impl TrashItem {
    /// The section's kind; `None` for a section this build doesn't know (the item is kept and
    /// not listed).
    pub fn kind_of_section(&self) -> Option<Kind> {
        Kind::from_section_dir(&self.section)
    }

    /// True if the item can be restored (FR-PRJ-58): no restore or purge has begun on it and
    /// its project folder is in the trash.
    fn restorable(&self, item_dir: &Path) -> bool {
        self.restored_at.is_none()
            && self.purging_at.is_none()
            && item_dir.join(&self.name).is_dir()
    }
}

/// `<root>/.pforge/trash`.
pub fn trash_dir(root: &Path) -> PathBuf {
    root.join(PFORGE_DIR).join("trash")
}

/// A project folder in its section: `<root>/<section>/<name>`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectLoc {
    pub root: PathBuf,
    pub kind: Kind,
    pub name: String,
    pub dir: PathBuf,
}

/// Validates the folder `project_delete` takes: an existing folder, directly in a section of a
/// root, with a `project.json` (readable or not). `not_found`, `outside_library`,
/// `not_a_project`. Nothing is read, so a cloud-only project is never hydrated.
pub fn locate(ctx: &Context, settings: &Settings, path: &Path) -> CoreResult<ProjectLoc> {
    let dir = std::path::absolute(path)
        .map(|p| normalise(&p))
        .map_err(|e| CoreError::io("resolving the path", &e))?;
    if std::fs::symlink_metadata(&dir).is_err() {
        return Err(CoreError::new(
            ErrorCode::NotFound,
            format!("{} not found", dir.display()),
        ));
    }
    let Some(root) = root_of(ctx, settings, &dir) else {
        return Err(CoreError::new(
            ErrorCode::OutsideLibrary,
            "the folder is outside the library",
        ));
    };
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let section = dir
        .parent()
        .filter(|s| s.parent().is_some_and(|r| super::same_folder(r, &root)));
    let kind = section
        .and_then(|s| s.file_name())
        .and_then(|n| Kind::from_section_dir(&n.to_string_lossy()));
    let is_project = dir.is_dir()
        && !name.contains(fsx::TEMP_MARKER)
        && std::fs::symlink_metadata(dir.join(MANIFEST_FILE)).is_ok();
    match kind {
        Some(kind) if is_project => Ok(ProjectLoc {
            root,
            kind,
            name,
            dir,
        }),
        _ => Err(not_a_project(&dir)),
    }
}

/// `project_id` of a project folder, if its `project.json` says one.
fn project_id_of(dir: &Path) -> Option<Ulid> {
    let bytes = std::fs::read(dir.join(MANIFEST_FILE)).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v["project_id"].as_str()?.parse().ok()
}

fn write_item(ctx: &Context, root: &Path, dir: &Path, item: &TrashItem) -> CoreResult<()> {
    std::fs::create_dir_all(dir)
        .map_err(|e| CoreError::io(format!("creating {}", dir.display()), &e))?;
    fsx::set_hidden(&root.join(PFORGE_DIR));
    fsx::write_json_atomic(ctx, &dir.join(ITEM_FILE), item)
}

/// Step one of a delete: the item, with its `item.json` on disk and the folder still in its
/// section. A kill here leaves an item without a folder, which is never listed and ages out.
pub(crate) fn begin_delete(ctx: &Context, loc: &ProjectLoc) -> CoreResult<(PathBuf, TrashItem)> {
    let item = TrashItem {
        schema_version: ITEM_SCHEMA_VERSION,
        id: ctx.new_id(),
        kind: TrashKind::Project,
        // A cloud-only manifest is not downloaded for this: the id only says which project it was.
        project_id: (!ctx.fs.is_cloud_only(&loc.dir.join(MANIFEST_FILE)))
            .then(|| project_id_of(&loc.dir))
            .flatten(),
        section: section_name(loc.kind),
        name: loc.name.clone(),
        trashed_at: ctx.now(),
        restored_at: None,
        purging_at: None,
        extra: Extra::new(),
    };
    let item_dir = trash_dir(&loc.root).join(item.id.to_string());
    if let Err(e) = write_item(ctx, &loc.root, &item_dir, &item) {
        // No item may claim a project the user was told wasn't deleted.
        let _ = drop_item(&item_dir);
        return Err(e);
    }
    Ok((item_dir, item))
}

/// Drops an item that holds no project: its `item.json` (and a temp file of it), then the item
/// folder itself, which must be empty by then. Never recursive, so a project folder in the
/// item, whatever put it there, is never deleted by a rollback; the item folder then stays.
pub(crate) fn drop_item(item_dir: &Path) -> std::io::Result<()> {
    for e in std::fs::read_dir(item_dir)?.flatten() {
        if e.file_type().is_ok_and(|t| !t.is_dir()) {
            std::fs::remove_file(e.path())?;
        }
    }
    std::fs::remove_dir(item_dir)
}

/// `project_delete` (FR-PRJ-57) for a closed project: `item.json` first, then the folder moves,
/// whole, in one rename. A move that fails is rolled back: the folder stays and the item goes.
/// Nothing in the folder is read or hydrated.
pub fn delete_project(ctx: &Context, loc: &ProjectLoc) -> CoreResult<Ulid> {
    let (item_dir, item) = begin_delete(ctx, loc)?;
    if let Err(e) = fsx::move_dir(ctx, &loc.dir, &item_dir.join(&item.name)) {
        if let Err(again) = drop_item(&item_dir) {
            // Left without a folder: never listed, and it ages out.
            tracing::warn!(item = %item.id, error = %again, "trash item not dropped after a failed delete");
        }
        return Err(e);
    }
    Ok(item.id)
}

// ---------------------------------------------------------------- list, restore

/// One row of Recently deleted (FR-PRJ-58).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrashEntry {
    pub id: Ulid,
    pub name: String,
    pub section: String,
    pub trashed_at: Timestamp,
    /// When it will be purged.
    pub purge_at: Timestamp,
}

/// Every readable item of one root's trash, with its folder.
fn items_of(root: &Path) -> Vec<(PathBuf, TrashItem)> {
    std::fs::read_dir(trash_dir(root))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let dir = e.path();
            let bytes = std::fs::read(dir.join(ITEM_FILE)).ok()?;
            let item: TrashItem = serde_json::from_slice(&bytes).ok()?;
            Some((dir, item))
        })
        .collect()
}

fn available_roots(ctx: &Context, settings: &Settings) -> Vec<PathBuf> {
    effective_roots(ctx, settings)
        .into_iter()
        .filter(|r| r.is_dir())
        .collect()
}

/// `library_trash_list(section)` (FR-PRJ-58): the items of every root for that section, newest
/// first; only those whose project folder is in the trash and whose purge hasn't begun.
/// `library_unavailable` as the home grid.
pub fn list(ctx: &Context, settings: &Settings, kind: Kind) -> CoreResult<Vec<TrashEntry>> {
    let mut out: Vec<TrashEntry> = require_available(ctx, settings)?
        .iter()
        .flat_map(|r| items_of(r))
        .filter(|(d, i)| i.restorable(d) && i.kind_of_section() == Some(kind))
        .map(|(_, i)| TrashEntry {
            id: i.id,
            name: i.name,
            section: i.section,
            trashed_at: i.trashed_at,
            purge_at: i.trashed_at.plus(TRASH_KEEP),
        })
        .collect();
    out.sort_by(|a, b| b.trashed_at.cmp(&a.trashed_at).then(b.id.cmp(&a.id)));
    Ok(out)
}

/// `library_trash_restore` (FR-PRJ-58): moves the folder back to its section in the same root,
/// with a ` (2)` suffix if the name is taken by then. Returns the project folder. A restore
/// that fails part-way clears `restored_at`, so the item is listed again. An item whose folder
/// isn't in the trash is `not_found` and is kept (R111); one being restored or purged is
/// `unknown_id`, as it is never listed.
pub fn restore(ctx: &Context, settings: &Settings, id: Ulid) -> CoreResult<PathBuf> {
    let found = available_roots(ctx, settings).into_iter().find_map(|r| {
        items_of(&r)
            .into_iter()
            .find(|(_, i)| i.id == id && i.restored_at.is_none() && i.purging_at.is_none())
            .map(|(d, i)| (r.clone(), d, i))
    });
    let unknown =
        || CoreError::new(ErrorCode::UnknownId, format!("no trash item {id}")).with_field("id");
    let (root, item_dir, mut item) = found.ok_or_else(unknown)?;
    if item.kind_of_section().is_none() {
        return Err(unknown());
    }
    if !item.restorable(&item_dir) {
        return Err(folder_missing(&item));
    }
    item.restored_at = Some(ctx.now());
    write_item(ctx, &root, &item_dir, &item)?;
    finish_restore(ctx, &root, &item_dir, &item)
        .inspect_err(|_| reopen_item(ctx, &root, &item_dir, &item))
}

fn folder_missing(item: &TrashItem) -> CoreError {
    CoreError::new(
        ErrorCode::NotFound,
        format!("{:?} is no longer in Recently deleted.", item.name),
    )
    .with_field("id")
}

/// Moves the project folder back to its section, then removes the item folder. `not_found` if
/// the folder isn't in the item: the item is never removed for that (R111).
fn finish_restore(
    ctx: &Context,
    root: &Path,
    item_dir: &Path,
    item: &TrashItem,
) -> CoreResult<PathBuf> {
    let kind = item
        .kind_of_section()
        .ok_or_else(|| CoreError::internal("a trash item of an unknown section"))?;
    let src = item_dir.join(&item.name);
    if !src.is_dir() {
        return Err(folder_missing(item));
    }
    let section = root.join(kind.section_dir());
    std::fs::create_dir_all(&section)
        .map_err(|e| CoreError::io(format!("creating {}", section.display()), &e))?;
    let taken = |c: &str| {
        std::fs::read_dir(&section)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| naming::same_name(&e.file_name().to_string_lossy(), c))
    };
    let target = section.join(naming::first_free(&item.name, taken));
    fsx::move_dir(ctx, &src, &target)?;
    if let Err(e) = drop_item(item_dir) {
        // The project is back; the leftover item (no folder) is unlisted and ages out.
        tracing::warn!(item = %item.id, error = %e, "restored trash item folder not removed");
    }
    Ok(target)
}

/// A restore that failed part-way shows in Recently deleted again.
fn reopen_item(ctx: &Context, root: &Path, item_dir: &Path, item: &TrashItem) {
    let mut item = item.clone();
    item.restored_at = None;
    if let Err(e) = write_item(ctx, root, item_dir, &item) {
        tracing::warn!(item = %item.id, error = %e, "failed restore not reopened");
    }
}

// ---------------------------------------------------------------- startup

/// Interrupted restores (`restored_at` set) at startup: `restored_at` is cleared and nothing
/// moves (R111, R112), so an item whose folder is still in the trash is listed again and the
/// user restores it. Returns the items cleared, which this startup's purge leaves alone.
/// Nothing else is touched: startup never moves a project to match an item and never drops an
/// item.
pub(crate) fn clear_interrupted_restores(ctx: &Context, root: &Path) -> Vec<Ulid> {
    let mut cleared = Vec::new();
    for (item_dir, item) in items_of(root) {
        if item.restored_at.is_none() || item.purging_at.is_some() {
            continue;
        }
        reopen_item(ctx, root, &item_dir, &item);
        cleared.push(item.id);
    }
    cleared
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// Purges items whose `trashed_at` + 30 days is before now (clock seam), and items a purge
/// already began on (`purging_at`). Returns `(count, bytes)`. An item is skipped while its
/// `restored_at` is set, and when it is in `spared` (its interrupted restore was cleared in
/// this startup). The item is marked first (`purging_at`, temp → rename), then the project
/// folder goes and `item.json` last, so an item that can't be fully deleted is never listed or
/// restored and is tried again at the next startup. Items whose `item.json` can't be read are
/// left alone.
pub fn purge(ctx: &Context, settings: &Settings, spared: &[Ulid]) -> (usize, u64) {
    let now = ctx.now();
    let (mut count, mut bytes) = (0, 0);
    for root in available_roots(ctx, settings) {
        for (dir, mut item) in items_of(&root) {
            if item.restored_at.is_some() {
                continue;
            }
            if item.purging_at.is_none() {
                if item.trashed_at.plus(TRASH_KEEP) >= now || spared.contains(&item.id) {
                    continue;
                }
                item.purging_at = Some(now);
                if let Err(e) = write_item(ctx, &root, &dir, &item) {
                    tracing::warn!(item = %item.id, error = %e, "library trash purge not started");
                    continue;
                }
            }
            let size = dir_size(&dir);
            let content = dir.join(&item.name);
            let emptied = match std::fs::remove_dir_all(&content) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
            match emptied.and_then(|()| std::fs::remove_dir_all(&dir)) {
                Ok(()) => {
                    count += 1;
                    bytes += size;
                }
                Err(e) => {
                    tracing::warn!(item = %item.id, error = %e, "library trash purge failed; tried again at the next start")
                }
            }
        }
    }
    (count, bytes)
}

/// The startup work for the library trash (FR-PRJ-58): interrupted restores are listed again
/// (nothing moves), then items older than 30 days are purged (`trash:purged{count, bytes}`).
pub fn sweep_startup(ctx: &Context, settings: &Settings) {
    let mut spared = Vec::new();
    for root in available_roots(ctx, settings) {
        spared.extend(clear_interrupted_restores(ctx, &root));
    }
    let (count, bytes) = purge(ctx, settings, &spared);
    if count > 0 {
        ctx.emit(
            "trash:purged",
            serde_json::json!({ "count": count, "bytes": bytes }),
        );
    }
}

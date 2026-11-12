//! The library trash (FR-PRJ-57, 58, #62, schema §4.15):
//! `<library-root>/.pforge/trash/<item-id>/item.json` plus the deleted project folder, whole, as
//! `<item-id>/<name>/`. `item.json` is written first, then the folder moves in one rename; an
//! interrupted delete (a kill) is rolled forward or dropped at startup, so a project is in its
//! section or in the trash, never both or neither. A move that fails with an error is rolled
//! back at once. Items older than 30 days (clock seam) are purged at startup. It mirrors the
//! References trash (`references::trash`, §4.13).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use super::{effective_roots, normalise, not_a_project, require_available, root_of};
use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::job::{JOB_FILE, JobFile};
use crate::manifest::{Extra, JOBS_DIR, Kind, MANIFEST_FILE, PFORGE_DIR};
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
    /// Set first when a restore starts, so an interrupted restore is finished at startup
    /// instead of being listed again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_at: Option<Timestamp>,
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

/// R97 for a closed project: the id of a job file in an active state (not terminal, not
/// `fetched`). Cloud-only and unreadable job files are skipped; nothing is hydrated.
pub fn active_job_on_disk(ctx: &Context, dir: &Path) -> Option<Ulid> {
    let jobs = dir.join(JOBS_DIR);
    std::fs::read_dir(&jobs)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let file = e.path().join(JOB_FILE);
            if ctx.fs.is_cloud_only(&file) {
                return None;
            }
            let job: JobFile = serde_json::from_slice(&std::fs::read(&file).ok()?).ok()?;
            crate::region_edit::jobs::is_active(job.state).then_some(job.id)
        })
        .next()
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
/// section. A kill here is what the startup roll-forward finishes.
pub(crate) fn begin_delete(ctx: &Context, loc: &ProjectLoc) -> CoreResult<(PathBuf, TrashItem)> {
    let item = TrashItem {
        schema_version: ITEM_SCHEMA_VERSION,
        id: ctx.new_id(),
        kind: TrashKind::Project,
        // A cloud-only manifest is not downloaded for this: the id is only a roll-forward guard.
        project_id: (!ctx.fs.is_cloud_only(&loc.dir.join(MANIFEST_FILE)))
            .then(|| project_id_of(&loc.dir))
            .flatten(),
        section: section_name(loc.kind),
        name: loc.name.clone(),
        trashed_at: ctx.now(),
        restored_at: None,
        extra: Extra::new(),
    };
    let item_dir = trash_dir(&loc.root).join(item.id.to_string());
    if let Err(e) = write_item(ctx, &loc.root, &item_dir, &item) {
        // No item may claim a project the user was told wasn't deleted.
        let _ = std::fs::remove_dir_all(&item_dir);
        return Err(e);
    }
    Ok((item_dir, item))
}

/// `project_delete` (FR-PRJ-57) for a closed project: `item.json` first, then the folder moves,
/// whole, in one rename. A move that fails is rolled back: the folder stays and the item goes.
/// Nothing in the folder is read or hydrated.
pub fn delete_project(ctx: &Context, loc: &ProjectLoc) -> CoreResult<Ulid> {
    let (item_dir, item) = begin_delete(ctx, loc)?;
    if let Err(e) = fsx::move_dir(ctx, &loc.dir, &item_dir.join(&item.name)) {
        let _ = std::fs::remove_dir_all(&item_dir);
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
/// first. `library_unavailable` as the home grid.
pub fn list(ctx: &Context, settings: &Settings, kind: Kind) -> CoreResult<Vec<TrashEntry>> {
    let mut out: Vec<TrashEntry> = require_available(ctx, settings)?
        .iter()
        .flat_map(|r| items_of(r))
        .filter(|(_, i)| i.restored_at.is_none() && i.kind_of_section() == Some(kind))
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
/// that fails part-way clears `restored_at`, so the item is listed again.
pub fn restore(ctx: &Context, settings: &Settings, id: Ulid) -> CoreResult<PathBuf> {
    let found = available_roots(ctx, settings).into_iter().find_map(|r| {
        items_of(&r)
            .into_iter()
            .find(|(_, i)| i.id == id && i.restored_at.is_none())
            .map(|(d, i)| (r.clone(), d, i))
    });
    let unknown =
        || CoreError::new(ErrorCode::UnknownId, format!("no trash item {id}")).with_field("id");
    let (root, item_dir, mut item) = found.ok_or_else(unknown)?;
    if item.kind_of_section().is_none() {
        return Err(unknown());
    }
    item.restored_at = Some(ctx.now());
    write_item(ctx, &root, &item_dir, &item)?;
    finish_restore(ctx, &root, &item_dir, &item)
        .inspect_err(|_| reopen_item(ctx, &root, &item_dir, &item))
}

/// Moves the project folder back if it is still in the item folder, then removes the item
/// folder. Also finishes an interrupted restore at startup.
fn finish_restore(
    ctx: &Context,
    root: &Path,
    item_dir: &Path,
    item: &TrashItem,
) -> CoreResult<PathBuf> {
    let kind = item
        .kind_of_section()
        .ok_or_else(|| CoreError::internal("a trash item of an unknown section"))?;
    let section = root.join(kind.section_dir());
    std::fs::create_dir_all(&section)
        .map_err(|e| CoreError::io(format!("creating {}", section.display()), &e))?;
    let src = item_dir.join(&item.name);
    let mut target = section.join(&item.name);
    if src.is_dir() {
        let taken = |c: &str| {
            std::fs::read_dir(&section)
                .into_iter()
                .flatten()
                .flatten()
                .any(|e| naming::same_name(&e.file_name().to_string_lossy(), c))
        };
        target = section.join(naming::first_free(&item.name, taken));
        fsx::move_dir(ctx, &src, &target)?;
    }
    if let Err(e) = std::fs::remove_dir_all(item_dir) {
        // The project is back; the leftover folder (still `restored_at`) goes at startup.
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

/// What the startup did with an interrupted delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Settled {
    /// The folder is in the item (already, or moved now).
    InTrash,
    /// The folder isn't this item's to take: the item is removed.
    Dropped,
}

/// Rolls interrupted deletes forward or drops them, and finishes interrupted restores (§4.15).
/// The folder is moved only if it is still in its section, is the same project (`project_id`)
/// and isn't in the item yet; otherwise an item without its folder is dropped.
pub(crate) fn roll_forward(ctx: &Context, root: &Path) {
    for (item_dir, item) in items_of(root) {
        if item.restored_at.is_some() {
            if let Err(e) = finish_restore(ctx, root, &item_dir, &item) {
                tracing::warn!(item = %item.id, error = %e, "library trash restore not finished");
                reopen_item(ctx, root, &item_dir, &item);
            }
            continue;
        }
        if finish_delete(ctx, root, &item_dir, &item) == Settled::Dropped {
            tracing::warn!(item = %item.id, "library trash item without its folder dropped");
            if let Err(e) = std::fs::remove_dir_all(&item_dir) {
                tracing::warn!(item = %item.id, error = %e, "dropped trash item not removed");
            }
        }
    }
}

fn finish_delete(ctx: &Context, root: &Path, item_dir: &Path, item: &TrashItem) -> Settled {
    let dst = item_dir.join(&item.name);
    if std::fs::symlink_metadata(&dst).is_ok() {
        return Settled::InTrash;
    }
    let Some(kind) = item.kind_of_section() else {
        return Settled::Dropped;
    };
    let origin = root.join(kind.section_dir()).join(&item.name);
    // The same project, not a later one that took the name.
    let same = origin.is_dir()
        && std::fs::symlink_metadata(origin.join(MANIFEST_FILE)).is_ok()
        && (ctx.fs.is_cloud_only(&origin.join(MANIFEST_FILE))
            || project_id_of(&origin) == item.project_id);
    if same && fsx::move_dir(ctx, &origin, &dst).is_ok() {
        Settled::InTrash
    } else {
        Settled::Dropped
    }
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

/// Purges items whose `trashed_at` + 30 days is before now (clock seam). Returns
/// `(count, bytes)`. An item is skipped while its `restored_at` is set. The project folder goes
/// first and `item.json` last, so an item that can't be fully deleted stays listed and is tried
/// again at the next startup. Items whose `item.json` can't be read are left alone.
pub fn purge(ctx: &Context, settings: &Settings) -> (usize, u64) {
    let now = ctx.now();
    let (mut count, mut bytes) = (0, 0);
    for root in available_roots(ctx, settings) {
        for (dir, item) in items_of(&root) {
            if item.trashed_at.plus(TRASH_KEEP) >= now || item.restored_at.is_some() {
                continue;
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

/// The startup work for the library trash (FR-PRJ-57, 58): interrupted deletes and restores
/// are settled, then items older than 30 days are purged (`trash:purged{count, bytes}`).
pub fn sweep_startup(ctx: &Context, settings: &Settings) {
    for root in available_roots(ctx, settings) {
        roll_forward(ctx, &root);
    }
    let (count, bytes) = purge(ctx, settings);
    if count > 0 {
        ctx.emit(
            "trash:purged",
            serde_json::json!({ "count": count, "bytes": bytes }),
        );
    }
}

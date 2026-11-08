//! The References trash (FR-REF-12, OQ-REF-03 A, schema §4.13):
//! `<root>/References/.pforge/trash/<item-id>/item.json` plus the moved files. `item.json` is
//! written first, then the move; an interrupted move is rolled forward at startup. Items older
//! than 30 days (clock seam) are purged at startup.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use super::*;
use crate::fsx;

/// How long a trashed item can be restored (#18).
pub const TRASH_KEEP: Duration = Duration::from_secs(30 * 86_400);
/// `item.json` `schema_version`.
pub const ITEM_SCHEMA_VERSION: u32 = 1;
const ITEM_FILE: &str = "item.json";

/// What a trash item holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashKind {
    /// A whole pack folder, under `<item>/<pack name>/`.
    Pack,
    /// Files of a pack, under `<item>/<file>`.
    Images,
}

/// §4.13 `item.json`. Unknown fields are kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrashItem {
    pub schema_version: u32,
    pub id: Ulid,
    pub kind: TrashKind,
    pub pack_id: Option<Ulid>,
    /// `References/<pack>`: where the pack (or the images) came from.
    pub from: String,
    /// The pack's image files (pack) or the moved files (images).
    pub files: Vec<String>,
    pub trashed_at: Timestamp,
    /// Set first when a restore starts, so an interrupted restore is finished at startup
    /// instead of rolled back into the trash (added field, schema §4.13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_at: Option<Timestamp>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl TrashItem {
    /// R61 (like R57): a trashed `pack.json` (a conflict-resolution loser, or an unreadable one)
    /// is not restorable. Back in the pack it can only land as a conflict copy, which the next
    /// listing trashes again. Packs and images are.
    pub fn restorable(&self) -> bool {
        !(self.kind == TrashKind::Images
            && !self.files.is_empty()
            && self
                .files
                .iter()
                .all(|f| f == PACK_FILE || naming::is_conflict_copy(f, PACK_FILE)))
    }

    /// The pack folder name from `from`.
    pub fn pack_name(&self) -> &str {
        self.from
            .strip_prefix(&format!("{REFERENCES_DIR}/"))
            .unwrap_or(&self.from)
    }
}

/// `<root>/References/.pforge/trash`.
pub fn trash_dir(root: &Path) -> PathBuf {
    references_dir(root).join(PACK_APP_DIR).join("trash")
}

fn write_item(ctx: &Context, dir: &Path, item: &TrashItem) -> CoreResult<()> {
    std::fs::create_dir_all(dir)
        .map_err(|e| CoreError::io(format!("creating {}", dir.display()), &e))?;
    fsx::set_hidden(&references_dir_of_trash(dir));
    fsx::write_json_atomic(ctx, &dir.join(ITEM_FILE), item)
}

/// `References/.pforge` for an item folder.
fn references_dir_of_trash(item_dir: &Path) -> PathBuf {
    item_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn new_item(
    ctx: &Context,
    kind: TrashKind,
    pack_id: Option<Ulid>,
    pack: &str,
    files: Vec<String>,
) -> TrashItem {
    TrashItem {
        schema_version: ITEM_SCHEMA_VERSION,
        id: ctx.new_id(),
        kind,
        pack_id,
        from: format!("{REFERENCES_DIR}/{pack}"),
        files,
        trashed_at: ctx.now(),
        restored_at: None,
        extra: Extra::new(),
    }
}

/// Moves files of the pack folder `dir` to a new trash item. The caller holds the write lock.
pub(crate) fn trash_files(
    ctx: &Context,
    root: &Path,
    pack_id: Option<Ulid>,
    pack_name: &str,
    dir: &Path,
    files: &[String],
) -> CoreResult<Ulid> {
    let item = new_item(ctx, TrashKind::Images, pack_id, pack_name, files.to_vec());
    let item_dir = trash_dir(root).join(item.id.to_string());
    write_item(ctx, &item_dir, &item)?;
    for f in files {
        fsx::move_file(&dir.join(f), &item_dir.join(f))?;
    }
    Ok(item.id)
}

/// `ref_pack_remove_images` (FR-REF-12).
pub fn remove_images(ctx: &Context, loc: &PackLoc, files: &[String]) -> CoreResult<Ulid> {
    let _w = write_lock();
    super::packs::require_files(loc, files)?;
    let id = trash_files(
        ctx,
        &loc.root,
        Some(loc.json.pack_id),
        &loc.name,
        &loc.dir,
        files,
    )?;
    thumb::request(ctx, &loc.dir, loc.json.pack_id);
    Ok(id)
}

/// `ref_pack_delete` (FR-REF-12): the whole folder goes to the trash.
pub fn delete_pack(ctx: &Context, loc: &PackLoc) -> CoreResult<Ulid> {
    let _w = write_lock();
    let files = image_files(&loc.dir).into_iter().map(|(n, _)| n).collect();
    let item = new_item(
        ctx,
        TrashKind::Pack,
        Some(loc.json.pack_id),
        &loc.name,
        files,
    );
    let item_dir = trash_dir(&loc.root).join(item.id.to_string());
    write_item(ctx, &item_dir, &item)?;
    fsx::move_file(&loc.dir, &item_dir.join(&loc.name))?;
    Ok(item.id)
}

// ---------------------------------------------------------------- list, restore

/// One row of Recently deleted.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrashEntry {
    pub id: Ulid,
    pub kind: TrashKind,
    pub pack_id: Option<Ulid>,
    pub pack_name: String,
    pub files: Vec<String>,
    pub trashed_at: Timestamp,
    /// When it will be purged.
    pub purge_at: Timestamp,
    /// False for a trashed `pack.json` (R61): no Restore.
    pub restorable: bool,
}

/// Every item of one root's trash, with its folder.
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

/// `ref_trash_list`: newest first.
pub fn list(ctx: &Context, settings: &Settings) -> Vec<TrashEntry> {
    let mut out: Vec<TrashEntry> = available_roots(ctx, settings)
        .iter()
        .flat_map(|r| items_of(r))
        .filter(|(_, i)| i.restored_at.is_none())
        .map(|(_, i)| TrashEntry {
            id: i.id,
            kind: i.kind,
            pack_id: i.pack_id,
            pack_name: i.pack_name().to_owned(),
            files: i.files.clone(),
            trashed_at: i.trashed_at,
            purge_at: i.trashed_at.plus(TRASH_KEEP),
            restorable: i.restorable(),
        })
        .collect();
    out.sort_by(|a, b| b.trashed_at.cmp(&a.trashed_at).then(b.id.cmp(&a.id)));
    out
}

/// `ref_trash_restore` (FR-REF-12): puts the item back, with a ` (2)` suffix if the name is
/// taken by then. Returns the restored path (the pack folder).
pub fn restore(ctx: &Context, settings: &Settings, id: Ulid) -> CoreResult<PathBuf> {
    let _w = write_lock();
    let (root, item_dir, mut item) = available_roots(ctx, settings)
        .into_iter()
        .find_map(|r| {
            items_of(&r)
                .into_iter()
                .find(|(_, i)| i.id == id && i.restored_at.is_none())
                .map(|(d, i)| (r.clone(), d, i))
        })
        .ok_or_else(|| {
            CoreError::new(ErrorCode::UnknownId, format!("no trash item {id}")).with_field("id")
        })?;
    if !item.restorable() {
        return Err(CoreError::new(
            ErrorCode::NotRestorable,
            "a trashed pack.json can't be restored; the pack keeps the version that was kept",
        )
        .with_field("id"));
    }
    item.restored_at = Some(ctx.now());
    write_item(ctx, &item_dir, &item)?;
    let path = finish_restore(ctx, settings, &root, &item_dir, &item)?;
    if let Some(pack_id) = item.pack_id {
        thumb::request(ctx, &path, pack_id);
    }
    Ok(path)
}

/// Moves what's still in the item folder back, then removes the folder. Also finishes an
/// interrupted restore at startup.
fn finish_restore(
    ctx: &Context,
    settings: &Settings,
    root: &Path,
    item_dir: &Path,
    item: &TrashItem,
) -> CoreResult<PathBuf> {
    let refs = ensure_references_dir(root)?;
    let path = match item.kind {
        TrashKind::Pack => {
            let src = item_dir.join(item.pack_name());
            let name = naming::first_free(item.pack_name(), |c| name_taken(&refs, c, None));
            let target = refs.join(&name);
            if src.is_dir() {
                fsx::move_file(&src, &target)?;
            }
            target
        }
        TrashKind::Images => {
            let dir = item
                .pack_id
                .and_then(|p| find_pack(ctx, settings, p))
                .filter(|l| l.root == root)
                .map(|l| l.dir)
                .unwrap_or_else(|| refs.join(item.pack_name()));
            std::fs::create_dir_all(&dir)
                .map_err(|e| CoreError::io(format!("creating {}", dir.display()), &e))?;
            for f in &item.files {
                let src = item_dir.join(f);
                if std::fs::symlink_metadata(&src).is_err() {
                    continue;
                }
                let free = free_file_name(&dir, f);
                fsx::move_file(&src, &dir.join(free))?;
            }
            dir
        }
    };
    std::fs::remove_dir_all(item_dir)
        .map_err(|e| CoreError::io(format!("removing {}", item_dir.display()), &e))?;
    Ok(path)
}

// ---------------------------------------------------------------- startup

/// Rolls interrupted trash moves forward and finishes interrupted restores (§4.13). A file is
/// moved only if it is still at its origin and not yet in the item folder.
pub(crate) fn roll_forward(ctx: &Context, settings: &Settings, root: &Path) {
    for (item_dir, item) in items_of(root) {
        let result = if item.restored_at.is_some() {
            finish_restore(ctx, settings, root, &item_dir, &item).map(|_| ())
        } else {
            finish_trash(root, &item_dir, &item)
        };
        if let Err(e) = result {
            tracing::warn!(item = %item.id, error = %e, "References trash roll-forward failed");
        }
    }
}

fn finish_trash(root: &Path, item_dir: &Path, item: &TrashItem) -> CoreResult<()> {
    let src_dir = root.join(&item.from);
    match item.kind {
        TrashKind::Pack => {
            let dst = item_dir.join(item.pack_name());
            if src_dir.is_dir() && !dst.exists() {
                fsx::move_file(&src_dir, &dst)?;
            }
        }
        TrashKind::Images => {
            for f in &item.files {
                let (src, dst) = (src_dir.join(f), item_dir.join(f));
                if src.is_file() && std::fs::symlink_metadata(&dst).is_err() {
                    fsx::move_file(&src, &dst)?;
                }
            }
        }
    }
    Ok(())
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
/// `(count, bytes)`. Items whose `item.json` can't be read are left alone.
pub fn purge(ctx: &Context, settings: &Settings) -> (usize, u64) {
    let _w = write_lock();
    let now = ctx.now();
    let (mut count, mut bytes) = (0, 0);
    for root in available_roots(ctx, settings) {
        for (dir, item) in items_of(&root) {
            if item.trashed_at.plus(TRASH_KEEP) >= now {
                continue;
            }
            let size = dir_size(&dir);
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {
                    count += 1;
                    bytes += size;
                }
                Err(e) => {
                    tracing::warn!(item = %item.id, error = %e, "References trash purge failed")
                }
            }
        }
    }
    (count, bytes)
}

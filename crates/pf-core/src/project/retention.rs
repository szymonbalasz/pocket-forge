//! Retention (PRJ slice 5): the trash list and restore, the 30-day purge on open (FR-PRJ-32)
//! and disk use per category (FR-PRJ-33). Trash items are recorded by the store API
//! ([`super::store::trash_files`]); their file moves follow each commit (schema §4.8).

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use ulid::Ulid;

use super::{OpenProject, commit};
use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::job::JOB_FILE;
use crate::manifest::*;
use crate::time::Timestamp;

/// How long a trash item is kept (FR-PRJ-32, #18).
pub const TRASH_KEEP: Duration = Duration::from_secs(30 * 86_400);

/// When `item` is purged.
pub fn purge_at(item: &TrashItem) -> Timestamp {
    item.trashed_at.plus(TRASH_KEEP)
}

/// Bytes of a file from its metadata only (never opens it, so nothing is hydrated, FR-PRJ-17).
fn file_bytes(path: &Path) -> u64 {
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map_or(0, |m| m.len())
}

// ---------------------------------------------------------------- list

/// One row of `project_trash_list` (asset-project §7 Storage).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrashRow {
    pub id: Ulid,
    pub kind: TrashKind,
    /// The first project path the item came from (the UI's label).
    pub path: String,
    /// Every project path in the item.
    pub files: Vec<String>,
    /// Size of the trashed copies (metadata only).
    pub bytes: u64,
    pub trashed_at: Timestamp,
    pub purge_at: Timestamp,
    pub reason: String,
    /// True if `project_trash_restore` takes it (R51); the UI offers Restore only then.
    pub restorable: bool,
}

/// R51 (BL-24): only these kinds can be restored. Any other item (orphan, mask, layer,
/// manifest) holds files nothing references, so the next open would sweep them back.
pub fn restorable_kind(kind: TrashKind) -> bool {
    matches!(
        kind,
        TrashKind::Candidates | TrashKind::JobInputs | TrashKind::ConflictCopy
    )
}

/// `project_trash_list`: the trash, newest first.
pub fn trash_list(p: &OpenProject) -> Vec<TrashRow> {
    let mut rows: Vec<TrashRow> = p
        .manifest
        .trash
        .iter()
        .map(|t| TrashRow {
            id: t.id,
            kind: t.kind,
            path: t.files.first().cloned().unwrap_or_default(),
            files: t.files.clone(),
            bytes: item_bytes(p, t),
            trashed_at: t.trashed_at,
            purge_at: purge_at(t),
            reason: t.reason.clone(),
            restorable: restorable_kind(t.kind),
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse((r.trashed_at, r.id)));
    rows
}

/// Bytes of an item's files: the trashed copy, or the file still at its old place (a move that
/// hasn't been rolled forward yet).
fn item_bytes(p: &OpenProject, t: &TrashItem) -> u64 {
    t.files
        .iter()
        .map(|f| {
            let trashed = file_bytes(&p.path(&t.trashed_path(f)));
            if trashed > 0 {
                trashed
            } else {
                file_bytes(&p.path(f))
            }
        })
        .sum()
}

// ---------------------------------------------------------------- restore

/// `project_trash_restore` (FR-PRJ-31): brings every file of an item back to its old path and
/// removes the item, in one commit. Only R51's kinds are restored (`not_restorable` otherwise,
/// `detail.kind` names the kind). Candidates of the item leave the trash (`trash_id` cleared;
/// their status is unchanged). Files move **before** the commit, so a crash in between leaves
/// the item indexed and the next open moves them back into the trash (state before).
pub fn trash_restore(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    item_id: Ulid,
) -> CoreResult<()> {
    p.require_writable()?;
    let item = p
        .manifest
        .trash
        .iter()
        .find(|t| t.id == item_id)
        .cloned()
        .ok_or_else(|| {
            CoreError::new(ErrorCode::UnknownId, "unknown trash item").with_field("item_id")
        })?;
    if !restorable_kind(item.kind) {
        return Err(CoreError::new(
            ErrorCode::NotRestorable,
            "this trash item holds files nothing uses; it can't be restored",
        )
        .with_field("item_id")
        .with_detail(json!({ "kind": item.kind })));
    }
    for from in &item.files {
        if p.path(&item.trashed_path(from)).is_file() && p.path(from).exists() {
            return Err(CoreError::new(
                ErrorCode::Conflict,
                format!("{from} exists again; move it away to restore the trashed copy"),
            )
            .with_field("item_id"));
        }
    }
    let mut moved: Vec<&String> = vec![];
    for from in &item.files {
        let src = p.path(&item.trashed_path(from));
        if src.is_file() {
            fsx::move_file(&src, &p.path(from))?;
            moved.push(from);
        }
    }
    let result = commit(ctx, install_id, p, |_, _, m, _| {
        m.trash.retain(|t| t.id != item_id);
        for c in m.candidates.iter_mut() {
            if c.trash_id == Some(item_id) {
                c.trash_id = None;
                if c.status == CandidateStatus::Trashed {
                    // A sweep put it there; back to an ordinary leftover.
                    c.status = CandidateStatus::Dismissed;
                }
            }
        }
        Ok(())
    });
    if let Err(e) = result {
        // Not committed: put the files back (the next commit or open retries any that fail).
        for from in moved {
            let _ = fsx::move_file(&p.path(from), &p.path(&item.trashed_path(from)));
        }
        return Err(e);
    }
    let dir = p.path(&format!("{TRASH_DIR}/{item_id}"));
    remove_empty_tree(&dir);
    Ok(())
}

/// Removes `dir` and its subfolders bottom-up if they hold no files.
fn remove_empty_tree(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                remove_empty_tree(&e.path());
            }
        }
    }
    fsx::remove_dir_if_empty(dir);
}

// ---------------------------------------------------------------- purge

/// What a purge removed (`trash:purged{count, bytes}`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Purged {
    pub count: usize,
    pub bytes: u64,
}

/// FR-PRJ-32: on open, deletes trash items older than 30 days (clock seam). The files go first,
/// then one commit drops the items, so a crash in between only repeats the purge. Accepted or
/// starred candidates and files any layer or revision references are never deleted (#18), even
/// if an item names them. Nothing else is ever deleted.
pub fn purge_expired(ctx: &Context, install_id: Ulid, p: &mut OpenProject) -> CoreResult<Purged> {
    if p.read_only.is_some() {
        return Ok(Purged::default());
    }
    let now = ctx.now();
    let expired: Vec<TrashItem> = p
        .manifest
        .trash
        .iter()
        .filter(|t| purge_at(t) < now)
        .cloned()
        .collect();
    if expired.is_empty() {
        return Ok(Purged::default());
    }
    let protected = protected_files(p);
    let mut out = Purged::default();
    for item in &expired {
        for from in &item.files {
            if protected.contains(from.as_str()) {
                tracing::warn!(file = %from, "a kept file was in the trash; not purged");
                continue;
            }
            let path = p.path(&item.trashed_path(from));
            let bytes = file_bytes(&path);
            match std::fs::remove_file(&path) {
                Ok(()) => out.bytes += bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    tracing::warn!(error = %e, file = %path.display(), "purge failed; retried at the next open");
                }
            }
        }
        remove_empty_tree(&p.path(&format!("{TRASH_DIR}/{}", item.id)));
        out.count += 1;
    }
    let ids: HashSet<Ulid> = expired.iter().map(|t| t.id).collect();
    commit(ctx, install_id, p, |_, _, m, _| {
        m.trash.retain(|t| !ids.contains(&t.id));
        Ok(())
    })?;
    ctx.emit(
        "trash:purged",
        json!({ "project_id": p.manifest.project_id, "count": out.count, "bytes": out.bytes }),
    );
    Ok(out)
}

/// Project paths that must never be deleted: accepted and starred candidates, layer files of
/// the stack and of every revision, the original.
fn protected_files(p: &OpenProject) -> HashSet<String> {
    let m = &p.manifest;
    let mut keep: HashSet<String> = m
        .candidates
        .iter()
        .filter(|c| c.starred || c.status == CandidateStatus::Accepted)
        .map(|c| c.file.clone())
        .collect();
    keep.extend(m.layers.iter().filter_map(|l| l.file.clone()));
    keep.extend(p.refs.layers.iter().map(|id| layer_file(*id)));
    keep.insert(m.original.file.clone());
    keep
}

// ---------------------------------------------------------------- disk use

/// Disk-use categories (FR-PRJ-33).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct UsageBytes {
    pub original: u64,
    pub layers: u64,
    pub masks: u64,
    pub job_inputs: u64,
    pub results: u64,
    pub trash: u64,
    pub user_folders: u64,
    pub exports: u64,
    pub other: u64,
}

impl UsageBytes {
    pub fn total(&self) -> u64 {
        self.original
            + self.layers
            + self.masks
            + self.job_inputs
            + self.results
            + self.trash
            + self.user_folders
            + self.exports
            + self.other
    }
}

/// `project_disk_usage` (FR-PRJ-33, Storage panel).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiskUsage {
    pub bytes: UsageBytes,
    pub total: u64,
    pub trash_count: usize,
    /// When the oldest trash item is purged, or null with an empty trash.
    pub next_purge: Option<Timestamp>,
}

/// Bytes per category from file metadata only: never opens a file, so a cloud-only file is
/// never hydrated (FR-PRJ-17, 33).
pub fn disk_usage(p: &OpenProject) -> DiskUsage {
    let mut b = UsageBytes::default();
    walk(&p.dir, "", &mut |rel, bytes| {
        let slot = match category(rel) {
            Category::Original => &mut b.original,
            Category::Layers => &mut b.layers,
            Category::Masks => &mut b.masks,
            Category::JobInputs => &mut b.job_inputs,
            Category::Results => &mut b.results,
            Category::Trash => &mut b.trash,
            Category::UserFolders => &mut b.user_folders,
            Category::Exports => &mut b.exports,
            Category::Other => &mut b.other,
        };
        *slot += bytes;
    });
    DiskUsage {
        total: b.total(),
        bytes: b,
        trash_count: p.manifest.trash.len(),
        next_purge: p.manifest.trash.iter().map(purge_at).min(),
    }
}

enum Category {
    Original,
    Layers,
    Masks,
    JobInputs,
    Results,
    Trash,
    UserFolders,
    Exports,
    Other,
}

fn category(rel: &str) -> Category {
    let under = |dir: &str| rel.starts_with(&format!("{dir}/"));
    if under(WORKING_DIR) {
        Category::Original
    } else if under(LAYERS_DIR) {
        Category::Layers
    } else if under(MASKS_DIR) {
        Category::Masks
    } else if under(JOBS_DIR) && !rel.ends_with(&format!("/{JOB_FILE}")) {
        Category::JobInputs
    } else if under(RESULTS_DIR) {
        Category::Results
    } else if under(TRASH_DIR) {
        Category::Trash
    } else if USER_DIRS.iter().any(|d| under(d)) {
        Category::UserFolders
    } else if under(EXPORTS_DIR) {
        Category::Exports
    } else {
        Category::Other
    }
}

fn walk(dir: &Path, prefix: &str, f: &mut dyn FnMut(&str, u64)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
            continue;
        };
        if meta.is_dir() {
            walk(&e.path(), &rel, f);
        } else if meta.is_file() {
            f(&rel, meta.len());
        }
    }
}

#[cfg(test)]
mod tests;

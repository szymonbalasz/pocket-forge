//! Sync conflicts (PRJ slice 6, #39): conflict copies and seq detection (FR-PRJ-34, 35), the
//! conflict view data and the resolve (FR-PRJ-36). Nothing is merged: the user keeps one
//! version, the other becomes a `conflict` revision, files from both sides stay and the indexes
//! are rebuilt. `job.json` and thumbnail copies are settled without asking (OQ-PRJ-10 A).

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;
use ulid::Ulid;

use super::{OpenProject, Warning, commit_with, read_json, sweep, write_job};
use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::job::{JOB_FILE, JobFile, JobState};
use crate::manifest::*;
use crate::naming;
use crate::time::Timestamp;

/// What put the project in conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictSource {
    /// A sync client's conflict copy of `project.json` (FR-PRJ-34), by file name.
    Copy(String),
    /// The on-disk `commit{seq, install_id}` isn't the last one loaded or written (FR-PRJ-35).
    Seq,
}

pub(crate) fn conflict_error() -> CoreError {
    CoreError::new(
        ErrorCode::Conflict,
        "this project was also changed on another machine; pick the version to keep",
    )
}

/// Conflict copies of `project.json` in the project folder, sorted.
pub(crate) fn manifest_copies(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.contains(fsx::TEMP_MARKER) && naming::is_conflict_copy(n, MANIFEST_FILE))
        .collect();
    names.sort();
    names
}

/// Puts the project in conflict: the state is kept, writes are refused until it's resolved,
/// and `project:conflict` fires once.
pub(crate) fn mark(ctx: &Context, p: &mut OpenProject, source: ConflictSource) {
    let first = p.conflict.is_none();
    p.conflict = Some(source);
    if !p.warnings.iter().any(|w| w.code == "conflict") {
        p.warnings.push(Warning::new(
            "conflict",
            "This project was also changed on another machine.",
        ));
    }
    if first {
        ctx.emit(
            "project:conflict",
            json!({ "project_id": p.manifest.project_id }),
        );
    }
}

/// Run before every manifest write (FR-PRJ-34, 35): refuses with `conflict` while one is open,
/// when a `project.json` conflict copy appears, or when the on-disk commit changed.
pub(crate) fn check_before_commit(ctx: &Context, p: &mut OpenProject) -> CoreResult<()> {
    if p.conflict.is_some() {
        return Err(conflict_error());
    }
    if let Some(copy) = manifest_copies(&p.dir).into_iter().next() {
        mark(ctx, p, ConflictSource::Copy(copy));
        return Err(conflict_error());
    }
    match super::check_commit_unchanged(p) {
        Err(e) if e.code == ErrorCode::Conflict => {
            mark(ctx, p, ConflictSource::Seq);
            Err(conflict_error())
        }
        other => other,
    }
}

// ---------------------------------------------------------------- job.json and thumb copies

/// Diagram order of the job states (schema §4.6), for "the later state wins" (OQ-PRJ-10 A).
fn state_rank(s: JobState) -> u8 {
    match s {
        JobState::Requested => 0,
        JobState::Submitted => 1,
        JobState::Unknown => 2,
        JobState::Queued => 3,
        JobState::Running => 4,
        JobState::Succeeded => 5,
        JobState::Fetched => 6,
        JobState::Closed | JobState::Failed | JobState::Canceled | JobState::Lost => 7,
    }
}

/// True if `a` is later than `b`: a later state, or the same rank written later.
pub(crate) fn later(a: &JobFile, b: &JobFile) -> bool {
    (state_rank(a.state), a.updated_at) > (state_rank(b.state), b.updated_at)
}

/// Copies of `job.json` in one job folder, as project paths.
fn job_copies(p: &OpenProject, job_dir: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(p.path(job_dir))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.contains(fsx::TEMP_MARKER) && naming::is_conflict_copy(n, JOB_FILE))
        .map(|n| format!("{job_dir}/{n}"))
        .collect();
    out.sort();
    out
}

/// OQ-PRJ-10 A: in each job folder, a `job.json` conflict copy with the later state wins. The
/// winner becomes `job.json` and the copy file then holds the loser, so the trash keeps it
/// (a crash in between leaves two equal files, settled the same way). Never drops a job handle
/// or paid candidates. Nothing is written in a read-only project.
pub(crate) fn settle_job_copies(ctx: &Context, p: &mut OpenProject) {
    if p.read_only.is_some() {
        return;
    }
    let ids: Vec<Ulid> = p.jobs.keys().copied().collect();
    for id in ids {
        let dir = job_dir(id);
        for copy in job_copies(p, &dir) {
            let Ok(theirs) = read_json::<JobFile>(&p.path(&copy)) else {
                continue;
            };
            let Some(ours) = p.jobs.get(&id).cloned() else {
                continue;
            };
            if theirs.id != id || !later(&theirs, &ours) {
                continue;
            }
            let swapped = write_job(ctx, p, &theirs)
                .and_then(|()| fsx::write_json_atomic(ctx, &p.path(&copy), &ours));
            match swapped {
                Ok(()) => {
                    if let Some(row) = p.manifest.jobs.iter_mut().find(|r| r.id == id) {
                        row.state = theirs.state;
                        row.iteration = theirs.iteration.as_ref().map(|i| i.n);
                    }
                    p.jobs.insert(id, theirs);
                }
                Err(e) => tracing::warn!(error = %e, "job copy not settled; retried later"),
            }
        }
    }
}

/// `job.json` and thumbnail conflict copies not in the trash yet (FR-PRJ-34): the next commit
/// sends them there.
pub(crate) fn side_copies(p: &OpenProject) -> Vec<String> {
    let in_trash: HashSet<&str> = p
        .manifest
        .trash
        .iter()
        .flat_map(|t| t.files.iter().map(String::as_str))
        .collect();
    let mut out = vec![];
    for id in p.jobs.keys() {
        out.extend(job_copies(p, &job_dir(*id)));
    }
    for e in std::fs::read_dir(p.path(PFORGE_DIR))
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = e.file_name().to_string_lossy().into_owned();
        if naming::is_conflict_copy(&name, "thumb.jpg") && e.file_type().is_ok_and(|t| t.is_file())
        {
            out.push(format!("{PFORGE_DIR}/{name}"));
        }
    }
    out.retain(|f| !in_trash.contains(f.as_str()));
    out
}

// ---------------------------------------------------------------- the conflict view

/// Which version to keep (`project_resolve_conflict`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Keep {
    This,
    Other,
}

/// One side of the conflict view (FR-PRJ-36, asset-project §7).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConflictSide {
    /// `this` | `other`: what `keep` takes.
    pub side: Keep,
    /// The file it was read from (`project.json`, `project-OTHERPC.json`), or null for this
    /// machine's state in memory.
    pub file: Option<String>,
    /// False if the file can't be read (yet).
    pub readable: bool,
    /// `commit.at`.
    pub committed_at: Option<Timestamp>,
    pub seq: Option<u64>,
    /// `this` = written by this machine (its install id), else `other`.
    pub machine: &'static str,
    /// The layer stack, draw order (OQ-PRJ-11: drawn by EDT when it can, else listed).
    pub layers: Vec<Layer>,
    /// The newest revision of that version.
    pub newest_revision: Option<RevisionEntry>,
    /// Project paths that version needs but aren't here yet ("Still syncing").
    pub missing_files: Vec<String>,
    pub can_keep: bool,
}

/// `project_conflict_get`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConflictView {
    /// `copy` (a `project.json` conflict copy) or `seq` (changed on disk while open).
    pub source: &'static str,
    pub this: ConflictSide,
    pub other: ConflictSide,
}

struct Sides {
    this: Option<Manifest>,
    this_file: Option<String>,
    other: Option<Manifest>,
    other_file: Option<String>,
    copy: Option<String>,
}

fn no_conflict() -> CoreError {
    CoreError::new(ErrorCode::NoConflict, "this project has no conflict")
}

fn read_manifest(p: &OpenProject, name: &str) -> Option<Manifest> {
    read_json::<Manifest>(&p.path(name)).ok()
}

fn sides(p: &OpenProject) -> CoreResult<Sides> {
    match p.conflict.as_ref().ok_or_else(no_conflict)? {
        ConflictSource::Copy(copy) => Ok(Sides {
            this: read_manifest(p, MANIFEST_FILE),
            this_file: Some(MANIFEST_FILE.into()),
            other: read_manifest(p, copy),
            other_file: Some(copy.clone()),
            copy: Some(copy.clone()),
        }),
        ConflictSource::Seq => Ok(Sides {
            this: Some(p.manifest.clone()),
            this_file: None,
            other: read_manifest(p, MANIFEST_FILE),
            other_file: Some(MANIFEST_FILE.into()),
            copy: None,
        }),
    }
}

/// Files a version needs to be kept: the original, its layer files, its head revision and the
/// draft's mask.
fn missing_files(p: &OpenProject, m: &Manifest) -> Vec<String> {
    // A base image's file is a layer file (FR-GEN-27); a project made by Generate new has no
    // original (FR-GEN-02).
    let mut need: Vec<String> = m.original.iter().map(|o| o.file.clone()).collect();
    need.extend(m.layers.iter().filter_map(|l| l.file.clone()));
    if m.revisions.iter().any(|r| r.id == m.head_revision) {
        need.push(revision_file(m.head_revision));
    }
    if let Some(id) = m.draft.as_ref().and_then(|d| d.mask_id) {
        need.push(mask_file(id));
    }
    need.retain(|f| std::fs::symlink_metadata(p.path(f)).is_err());
    need
}

fn side(
    p: &OpenProject,
    install_id: Ulid,
    which: Keep,
    m: Option<&Manifest>,
    file: Option<String>,
) -> ConflictSide {
    let missing = m.map(|m| missing_files(p, m)).unwrap_or_default();
    ConflictSide {
        side: which,
        file,
        readable: m.is_some(),
        committed_at: m.map(|m| m.commit.at),
        seq: m.map(|m| m.commit.seq),
        machine: match m {
            Some(m) if m.commit.install_id == install_id => "this",
            _ => "other",
        },
        layers: m.map(|m| m.layers.clone()).unwrap_or_default(),
        newest_revision: m.and_then(|m| m.revisions.iter().max_by_key(|r| r.seq).cloned()),
        can_keep: m.is_some() && missing.is_empty(),
        missing_files: missing,
    }
}

/// BL-22 (OQ-PRJ-11 A): while a conflict is open, the original or a patch layer of either
/// version resolves too, so EDT can draw the other version's stack read-only. Returns the
/// project path and whether it's the original; None if neither version has `image_id`. A patch
/// is always `layer_file(id)` (write-once, by id); a synced copy's original path must stay
/// inside the project folder.
pub(crate) fn side_image(p: &OpenProject, image_id: Ulid) -> Option<(String, bool)> {
    let s = sides(p).ok()?;
    [s.this, s.other].into_iter().flatten().find_map(|m| {
        if m.original_image_id() == Some(image_id) {
            let f = m.base_file()?;
            let inside = Path::new(f)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
            return inside.then(|| (f.to_owned(), true));
        }
        m.layers
            .iter()
            .any(|l| l.id == image_id && l.kind == LayerKind::Patch)
            .then(|| (layer_file(image_id), false))
    })
}

/// `project_conflict_get` (FR-PRJ-36): both versions. `no_conflict` if there is none.
pub fn conflict_get(p: &OpenProject, install_id: Ulid) -> CoreResult<ConflictView> {
    let s = sides(p)?;
    Ok(ConflictView {
        source: match p.conflict {
            Some(ConflictSource::Copy(_)) => "copy",
            _ => "seq",
        },
        this: side(p, install_id, Keep::This, s.this.as_ref(), s.this_file),
        other: side(p, install_id, Keep::Other, s.other.as_ref(), s.other_file),
    })
}

/// `project_resolve_conflict` (FR-PRJ-36, #39): one commit makes the kept version current,
/// writes the other's layers + draft as a `conflict` revision (the head stays the kept one's),
/// and trashes the conflict copy. Files from both sides stay; the job, revision and trash
/// indexes are rebuilt from the files. A version with missing files → `files_missing`.
/// The command then runs RGN's runner tick, which replays the spool at once (FR-PRJ-40, R48).
/// Returns the `conflict` revision.
pub fn resolve_conflict(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    keep: Keep,
) -> CoreResult<RevisionEntry> {
    p.require_writable()?;
    let mut s = sides(p)?;
    // Walk 2 N1: a draft refused during the conflict is this machine's newest state. It goes with
    // this version: current if kept, else in the `conflict` revision. Never dropped silently.
    if let (Some(pending), Some(this)) = (p.pending_draft.clone(), s.this.as_mut()) {
        super::apply_draft(ctx, this, pending.draft, pending.export);
    }
    let (kept, lost) = match keep {
        Keep::This => (s.this, s.other),
        Keep::Other => (s.other, s.this),
    };
    let Some(mut kept) = kept else {
        return Err(CoreError::new(
            ErrorCode::FilesMissing,
            "that version can't be read yet (still syncing?)",
        ));
    };
    let missing = missing_files(p, &kept);
    if !missing.is_empty() {
        return Err(CoreError::new(
            ErrorCode::FilesMissing,
            "that version's files haven't arrived yet (still syncing)",
        )
        .with_detail(json!({ "files": missing })));
    }
    if let Some(lost) = &lost {
        // The trash is an index of files on disk: keep both sides' items.
        for t in &lost.trash {
            if !kept.trash.iter().any(|k| k.id == t.id) {
                kept.trash.push(t.clone());
            }
        }
    }

    // Install the kept version, keeping what's needed to roll back if the commit fails.
    let disk = super::disk_commit(p);
    let saved = (
        std::mem::replace(&mut p.manifest, kept),
        p.last_commit,
        p.conflict.take(),
        p.jobs.clone(),
        p.refs.clone(),
        p.warnings.clone(),
    );
    p.last_commit = disk;
    let result = (|| {
        sweep::load_history(p)?;
        sweep::add_missing_candidates(p);
        let not_kept = match &lost {
            Some(l) if l.commit.install_id == install_id => "this machine",
            _ => "other machine",
        };
        commit_with(ctx, install_id, p, true, |ctx, op, m, plan| {
            if let Some(l) = &lost {
                let rev = RevisionFile {
                    schema_version: REVISION_SCHEMA_VERSION,
                    id: ctx.new_id(),
                    seq: m.revisions.iter().map(|r| r.seq + 1).max().unwrap_or(0),
                    created_at: ctx.now(),
                    reason: RevisionReason::Conflict,
                    label: Some(format!("Not kept ({not_kept})")),
                    parent: l
                        .revisions
                        .iter()
                        .any(|r| r.id == l.head_revision)
                        .then_some(l.head_revision),
                    layers: l.layers.clone(),
                    draft: l.draft.clone(),
                    extra: Extra::new(),
                };
                m.revisions.push(rev.entry());
                plan.revisions.push(rev);
            }
            // A copy that's gone already (deleted, or synced away) leaves no empty trash item.
            if let Some(copy) = s.copy.as_ref().filter(|c| op.path(c).is_file()) {
                super::store::trash_files(
                    ctx,
                    m,
                    TrashKind::ConflictCopy,
                    vec![copy.clone()],
                    "sync conflict resolved",
                );
            }
            Ok(())
        })
    })();
    if let Err(e) = result {
        p.manifest = saved.0;
        p.last_commit = saved.1;
        p.conflict = saved.2;
        p.jobs = saved.3;
        p.refs = saved.4;
        p.warnings = saved.5;
        return Err(e);
    }
    p.warnings.retain(|w| w.code != "conflict");
    p.pending_draft = None;
    p.request_thumbnail(ctx);
    let entry = p
        .manifest
        .revisions
        .iter()
        .filter(|r| r.reason == RevisionReason::Conflict)
        .max_by_key(|r| (r.seq, r.created_at))
        .or_else(|| {
            p.manifest
                .revisions
                .iter()
                .find(|r| r.id == p.manifest.head_revision)
        })
        .cloned()
        .ok_or_else(|| CoreError::internal("no revision after the resolve"))?;
    // A second copy (rare) puts the project straight back in conflict at the next commit.
    Ok(entry)
}

#[cfg(test)]
mod tests;

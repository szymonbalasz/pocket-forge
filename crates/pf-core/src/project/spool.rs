//! The pending-write spool (FR-PRJ-40, R12, schema §4.9): while a write can't commit
//! (`conflict`, `locked` after 3 s, `read_only`), candidate bytes and job updates go to
//! `<data-dir>/pending/<project-id>/` and are replayed in `seq` order once the project is
//! writable (on open, after a resolve or relink, and on the 10 s retry). A crash in between
//! loses no paid output.
//!
//! An entry is `<seq>.json` (written last: it is the entry) plus `<seq>-<cand-id>.<ext>` bytes.
//! Each entry is deleted after its commit. An entry that can no longer apply (e.g. its job is
//! already past that state) moves to `dropped/`, never deleted.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ulid::Ulid;

use super::OpenProject;
use super::store::{self, NewCandidate};
use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::job::JobFile;
use crate::time::Timestamp;

/// The spool folder of a project.
pub fn spool_dir(ctx: &Context, project_id: Ulid) -> PathBuf {
    ctx.data_dir.join("pending").join(project_id.to_string())
}

/// Errors that send a write to the spool instead of losing it.
pub(crate) fn spoolable(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::Conflict | ErrorCode::Locked | ErrorCode::ReadOnly
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Op {
    AddCandidates,
    UpdateJob,
}

/// `<seq>.json` (schema §4.9).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    seq: u64,
    op: Op,
    job_id: Ulid,
    payload: Value,
    created_at: Timestamp,
}

/// One spooled candidate (payload of `add_candidates`); its bytes are `<seq>-<id>.<ext>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpooledCandidate {
    id: Ulid,
    ext: String,
    seed: Option<i64>,
    provider_meta: Value,
    invalid: bool,
}

fn entry_seqs(dir: &Path) -> Vec<u64> {
    let mut seqs: Vec<u64> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json")?.parse::<u64>().ok()
        })
        .collect();
    seqs.sort_unstable();
    seqs
}

fn bytes_file(dir: &Path, seq: u64, id: Ulid, ext: &str) -> PathBuf {
    dir.join(format!("{seq}-{id}.{ext}"))
}

/// How many entries wait for the project.
pub fn pending_count(ctx: &Context, project_id: Ulid) -> usize {
    entry_seqs(&spool_dir(ctx, project_id)).len()
}

fn write_entry(
    ctx: &Context,
    p: &OpenProject,
    op: Op,
    job_id: Ulid,
    payload: Value,
) -> CoreResult<u64> {
    let dir = spool_dir(ctx, p.manifest.project_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| CoreError::io(format!("creating {}", dir.display()), &e))?;
    let seq = entry_seqs(&dir).last().map_or(1, |s| s + 1);
    let entry = Entry {
        seq,
        op,
        job_id,
        payload,
        created_at: ctx.now(),
    };
    fsx::write_json_atomic(ctx, &dir.join(format!("{seq}.json")), &entry)?;
    Ok(seq)
}

/// Spools a job update: the whole job file after it (replay writes it unless the job on disk
/// is already later).
pub(crate) fn spool_job(ctx: &Context, p: &OpenProject, job: &JobFile) -> CoreResult<u64> {
    write_entry(ctx, p, Op::UpdateJob, job.id, json!({ "job": job }))
}

/// Spools candidates with their ids (the bytes first, then the entry).
pub(crate) fn spool_candidates(
    ctx: &Context,
    p: &OpenProject,
    job_id: Ulid,
    candidates: &[(Ulid, NewCandidate)],
) -> CoreResult<u64> {
    let dir = spool_dir(ctx, p.manifest.project_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| CoreError::io(format!("creating {}", dir.display()), &e))?;
    let seq = entry_seqs(&dir).last().map_or(1, |s| s + 1);
    let mut rows = vec![];
    for (id, c) in candidates {
        let ext = pf_imaging::sniff(&c.bytes).map_or("bin", |f| f.extension());
        fsx::write_atomic(ctx, &bytes_file(&dir, seq, *id, ext), &c.bytes)?;
        rows.push(SpooledCandidate {
            id: *id,
            ext: ext.into(),
            seed: c.seed,
            provider_meta: c.provider_meta.clone(),
            invalid: c.invalid,
        });
    }
    let entry = Entry {
        seq,
        op: Op::AddCandidates,
        job_id,
        payload: json!({ "candidates": rows }),
        created_at: ctx.now(),
    };
    fsx::write_json_atomic(ctx, &dir.join(format!("{seq}.json")), &entry)?;
    Ok(seq)
}

/// What a replay did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Replayed {
    pub applied: usize,
    pub dropped: usize,
    /// Entries still waiting (the project is still blocked).
    pub remaining: usize,
}

/// Replays the spool in `seq` order while the project is writable. Stops at the first entry
/// that is blocked again (kept for the next try).
pub fn replay(ctx: &Context, install_id: Ulid, p: &mut OpenProject) -> Replayed {
    let dir = spool_dir(ctx, p.manifest.project_id);
    let seqs = entry_seqs(&dir);
    let mut out = Replayed::default();
    if seqs.is_empty() {
        return out;
    }
    if p.read_only.is_some() || p.conflict.is_some() {
        out.remaining = seqs.len();
        return out;
    }
    for (i, seq) in seqs.iter().enumerate() {
        let path = dir.join(format!("{seq}.json"));
        let entry: Option<Entry> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let result = match &entry {
            Some(e) => apply(ctx, install_id, p, &dir, e),
            None => Err(CoreError::new(ErrorCode::Corrupt, "unreadable spool entry")),
        };
        match result {
            Ok(()) => {
                out.applied += 1;
                remove_entry(&dir, *seq);
            }
            Err(e) if spoolable(e.code) => {
                out.remaining = seqs.len() - i;
                return out;
            }
            Err(e) => {
                tracing::warn!(seq, code = %e.code, "spool entry can't apply; kept in dropped/: {}", e.message);
                out.dropped += 1;
                drop_entry(&dir, *seq);
            }
        }
    }
    if out.applied > 0 {
        tracing::info!(applied = out.applied, "spool replayed");
    }
    fsx::remove_dir_if_empty(&dir);
    out
}

fn apply(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    dir: &Path,
    e: &Entry,
) -> CoreResult<()> {
    match e.op {
        Op::UpdateJob => {
            let job: JobFile = serde_json::from_value(e.payload["job"].clone())
                .map_err(|err| CoreError::new(ErrorCode::Corrupt, format!("spooled job: {err}")))?;
            store::apply_spooled_job(ctx, p, job)
        }
        Op::AddCandidates => {
            let rows: Vec<SpooledCandidate> =
                serde_json::from_value(e.payload["candidates"].clone()).map_err(|err| {
                    CoreError::new(ErrorCode::Corrupt, format!("spooled candidates: {err}"))
                })?;
            let mut cands = vec![];
            for r in rows {
                let path = bytes_file(dir, e.seq, r.id, &r.ext);
                let bytes = std::fs::read(&path)
                    .map_err(|err| CoreError::io("reading spooled candidate bytes", &err))?;
                cands.push((
                    r.id,
                    NewCandidate {
                        bytes,
                        seed: r.seed,
                        provider_meta: r.provider_meta,
                        invalid: r.invalid,
                    },
                ));
            }
            store::add_candidates_with_ids(ctx, install_id, p, e.job_id, cands).map(|_| ())
        }
    }
}

/// Files of one entry: `<seq>.json` and its `<seq>-*` bytes.
fn entry_files(dir: &Path, seq: u64) -> Vec<PathBuf> {
    let prefix = format!("{seq}-");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .map(|e| e.path())
        .collect();
    files.push(dir.join(format!("{seq}.json")));
    files
}

/// Deletes an applied entry: the json first (then it's gone), then its bytes.
fn remove_entry(dir: &Path, seq: u64) {
    let _ = std::fs::remove_file(dir.join(format!("{seq}.json")));
    for f in entry_files(dir, seq) {
        let _ = std::fs::remove_file(f);
    }
}

fn drop_entry(dir: &Path, seq: u64) {
    let dropped = dir.join("dropped");
    let _ = std::fs::create_dir_all(&dropped);
    // Bytes first, the json last, so a crash never leaves an entry without its bytes.
    for f in entry_files(dir, seq) {
        if let Some(name) = f.file_name() {
            let _ = std::fs::rename(&f, dropped.join(name));
        }
    }
}

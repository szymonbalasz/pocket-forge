//! The pending-write spool for job updates and candidate bytes (FR-RGN-37, R12, schema §4.9).
//!
//! **Interim, RGN-local:** PRJ's spool (PRJ slice 6, FR-PRJ-40) isn't on `main` yet. This one
//! writes the schema §4.9 layout, so PRJ's can take it over: `<data-dir>/pending/<project-id>/`
//! holds `<seq>.json` = `{seq, op, job_id, payload, created_at}` plus `<seq>-<NN>.<ext>` bytes.
//! The bytes are written before the entry (the entry's rename is the commit point). Entries are
//! replayed in `seq` order once the project is writable; each is deleted after its commit.
//!
//! The job runner spools when a store write can't commit (`conflict`, `locked`, `read_only`,
//! I/O) and when candidates arrive for a project that isn't open. Paid output is never dropped.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::job::JobState;
use crate::manifest::Extra;
use crate::project::OpenProject;
use crate::project::store::{self, JobUpdate, NewCandidate};
use crate::time::Timestamp;

/// A job update as spooled: the same fields as PRJ's [`JobUpdate`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpoolUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<JobState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_returned: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Value>,
    /// `Some(Value::Null)` clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

impl From<SpoolUpdate> for JobUpdate {
    fn from(u: SpoolUpdate) -> Self {
        JobUpdate {
            state: u.state,
            provider: u.provider,
            seed_returned: u.seed_returned,
            estimate: None,
            cost: u.cost,
            error: u.error,
        }
    }
}

/// One spooled candidate (its bytes are in the file next to the entry).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpooledCandidate {
    file: String,
    #[serde(default)]
    seed: Option<i64>,
    #[serde(default)]
    provider_meta: Value,
    #[serde(default)]
    invalid: bool,
    #[serde(default)]
    extra: Extra,
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

/// A store error the spool absorbs (FR-RGN-37): the write is retried after the cause is gone.
pub fn spoolable(e: &CoreError) -> bool {
    matches!(
        e.code,
        ErrorCode::Conflict | ErrorCode::Locked | ErrorCode::ReadOnly | ErrorCode::Io
    )
}

fn dir(ctx: &Context, project_id: Ulid) -> PathBuf {
    ctx.data_dir.join("pending").join(project_id.to_string())
}

fn entries(dir: &Path) -> Vec<(u64, PathBuf)> {
    let mut out: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let seq = name.strip_suffix(".json")?.parse().ok()?;
            Some((seq, e.path()))
        })
        .collect();
    out.sort();
    out
}

/// True if the project has spooled writes waiting.
pub fn has_pending(ctx: &Context, project_id: Ulid) -> bool {
    !entries(&dir(ctx, project_id)).is_empty()
}

fn next_seq(dir: &Path) -> u64 {
    let from_names = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.split(['.', '-']).next()?.parse::<u64>().ok()
        })
        .max();
    from_names.map_or(1, |s| s + 1)
}

fn write_entry(ctx: &Context, dir: &Path, entry: &Entry) -> CoreResult<()> {
    fsx::write_json_atomic(ctx, &dir.join(format!("{}.json", entry.seq)), entry)
}

fn ensure_dir(dir: &Path) -> CoreResult<()> {
    std::fs::create_dir_all(dir)
        .map_err(|e| CoreError::io(format!("creating {}", dir.display()), &e))
}

/// Spools a job update.
pub fn push_update(
    ctx: &Context,
    project_id: Ulid,
    job_id: Ulid,
    update: &SpoolUpdate,
) -> CoreResult<()> {
    let dir = dir(ctx, project_id);
    ensure_dir(&dir)?;
    let entry = Entry {
        seq: next_seq(&dir),
        op: Op::UpdateJob,
        job_id,
        payload: serde_json::to_value(update).map_err(|e| CoreError::internal(e.to_string()))?,
        created_at: ctx.now(),
    };
    write_entry(ctx, &dir, &entry)
}

/// Spools fetched candidates: the bytes first, then the entry.
pub fn push_candidates(
    ctx: &Context,
    project_id: Ulid,
    job_id: Ulid,
    candidates: &[NewCandidate],
) -> CoreResult<()> {
    let dir = dir(ctx, project_id);
    ensure_dir(&dir)?;
    let seq = next_seq(&dir);
    let mut listed = vec![];
    for (i, c) in candidates.iter().enumerate() {
        let ext = pf_imaging::sniff(&c.bytes).map_or("bin", |f| f.extension());
        let file = format!("{seq}-{:02}.{ext}", i + 1);
        fsx::write_atomic(ctx, &dir.join(&file), &c.bytes)?;
        listed.push(SpooledCandidate {
            file,
            seed: c.seed,
            provider_meta: c.provider_meta.clone(),
            invalid: c.invalid,
            extra: c.extra.clone(),
        });
    }
    let entry = Entry {
        seq,
        op: Op::AddCandidates,
        job_id,
        payload: serde_json::to_value(&listed).map_err(|e| CoreError::internal(e.to_string()))?,
        created_at: ctx.now(),
    };
    write_entry(ctx, &dir, &entry)
}

fn remove_entry(dir: &Path, entry: &Entry, path: &Path) {
    if let Op::AddCandidates = entry.op
        && let Ok(list) = serde_json::from_value::<Vec<SpooledCandidate>>(entry.payload.clone())
    {
        for c in list {
            let _ = std::fs::remove_file(dir.join(c.file));
        }
    }
    let _ = std::fs::remove_file(path);
}

/// What a replay did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Replayed {
    /// Jobs whose candidates were committed by this replay.
    pub fetched: Vec<Ulid>,
    pub entries: usize,
}

/// Replays the project's spool in `seq` order (FR-RGN-37). Stops at the first entry that still
/// can't commit (it's retried later). An entry the store refuses for good (a transition the job
/// already passed, an unknown job) is dropped with a log line.
pub fn replay(ctx: &Context, install_id: Ulid, p: &mut OpenProject) -> CoreResult<Replayed> {
    let mut out = Replayed::default();
    if p.read_only.is_some() {
        return Ok(out);
    }
    let dir = dir(ctx, p.manifest.project_id);
    for (_, path) in entries(&dir) {
        let entry: Entry = match std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
        {
            Some(e) => e,
            None => {
                tracing::warn!(path = %path.display(), "unreadable spool entry; kept");
                continue;
            }
        };
        let result = match entry.op {
            Op::UpdateJob => serde_json::from_value::<SpoolUpdate>(entry.payload.clone())
                .map_err(|e| CoreError::new(ErrorCode::Corrupt, e.to_string()))
                .and_then(|u| store::update_job(ctx, p, entry.job_id, u.into()).map(|_| ())),
            Op::AddCandidates => {
                let list: Vec<SpooledCandidate> = serde_json::from_value(entry.payload.clone())
                    .map_err(|e| CoreError::new(ErrorCode::Corrupt, e.to_string()))?;
                let mut cands = vec![];
                for c in list {
                    let bytes = std::fs::read(dir.join(&c.file))
                        .map_err(|e| CoreError::io("reading a spooled candidate", &e))?;
                    cands.push(NewCandidate {
                        bytes,
                        seed: c.seed,
                        provider_meta: c.provider_meta,
                        invalid: c.invalid,
                        extra: c.extra,
                    });
                }
                store::add_candidates(ctx, install_id, p, entry.job_id, cands).map(|_| {
                    out.fetched.push(entry.job_id);
                })
            }
        };
        match result {
            Ok(()) => {
                remove_entry(&dir, &entry, &path);
                out.entries += 1;
            }
            Err(e) if spoolable(&e) => return Ok(out),
            Err(e)
                if entry.op == Op::AddCandidates
                    && p.jobs.get(&entry.job_id).is_some_and(|j| {
                        !matches!(j.state, JobState::Fetched | JobState::Closed)
                    }) =>
            {
                // Paid output is never dropped: kept for the next replay.
                tracing::warn!(seq = entry.seq, error = %e, "spooled candidates refused; kept");
            }
            Err(e) => {
                tracing::warn!(seq = entry.seq, error = %e, "spool entry refused; dropped");
                remove_entry(&dir, &entry, &path);
            }
        }
    }
    if entries(&dir).is_empty() {
        let _ = std::fs::remove_dir(&dir);
    }
    Ok(out)
}

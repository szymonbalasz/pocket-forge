//! The pending-write spool (FR-PRJ-40, FR-RGN-37, R12, R48; schema §4.9). PRJ owns it; RGN's job
//! runner is its only writer and replays it on open and on every tick while entries wait.
//!
//! `<data-dir>/pending/<project-id>/` holds `<seq>.json` = `{seq, op, job_id, payload,
//! created_at}` plus `<seq>-<NN>.<ext>` candidate bytes. The bytes are written before the entry
//! (the entry's rename is the commit point). Entries are replayed in `seq` order once the project
//! is writable and out of conflict; each is deleted after its commit. Paid output is never
//! dropped. Keyed by project id, so candidates for a closed project wait here too.
//!
//! Taken over from RGN's interim `region_edit/spool.rs` (R48), which now re-exports this.

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

/// How many entries wait for the project.
pub fn pending_count(ctx: &Context, project_id: Ulid) -> usize {
    entries(&dir(ctx, project_id)).len()
}

/// The spool folder of a project.
pub fn spool_dir(ctx: &Context, project_id: Ulid) -> PathBuf {
    dir(ctx, project_id)
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

/// The event that tells the user a spooled write was dropped (BL-25):
/// `{project_id, job_id, seq, op: update_job|add_candidates, code}`.
pub const EVENT_SPOOL_DROPPED: &str = "project:spool-dropped";

/// A spool entry the store refused for good (BL-25).
#[derive(Debug, Clone, PartialEq)]
pub struct Dropped {
    pub seq: u64,
    pub job_id: Ulid,
    /// `update_job` or `add_candidates`.
    pub op: &'static str,
    /// The store's error code.
    pub code: ErrorCode,
}

/// What a replay did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Replayed {
    /// Jobs whose candidates were committed by this replay.
    pub fetched: Vec<Ulid>,
    pub entries: usize,
    /// Entries refused for good and deleted; each also fired [`EVENT_SPOOL_DROPPED`].
    pub dropped: Vec<Dropped>,
}

/// Replays the project's spool in `seq` order (FR-RGN-37). Stops at the first entry that still
/// can't commit (it's retried later). An entry the store refuses for good (a transition the job
/// already passed, an unknown job) is dropped with a log line and an [`EVENT_SPOOL_DROPPED`]
/// event, so the user is told (BL-25).
pub fn replay(ctx: &Context, install_id: Ulid, p: &mut OpenProject) -> CoreResult<Replayed> {
    let mut out = Replayed::default();
    // Read-only, or a conflict the user hasn't resolved: every store write would refuse.
    if p.read_only.is_some() || p.conflict.is_some() {
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
                let d = Dropped {
                    seq: entry.seq,
                    job_id: entry.job_id,
                    op: match entry.op {
                        Op::UpdateJob => "update_job",
                        Op::AddCandidates => "add_candidates",
                    },
                    code: e.code,
                };
                ctx.emit(
                    EVENT_SPOOL_DROPPED,
                    serde_json::json!({
                        "project_id": p.manifest.project_id,
                        "job_id": d.job_id,
                        "seq": d.seq,
                        "op": d.op,
                        "code": d.code,
                    }),
                );
                out.dropped.push(d);
            }
        }
    }
    if entries(&dir).is_empty() {
        let _ = std::fs::remove_dir(&dir);
    }
    Ok(out)
}

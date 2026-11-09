//! The pending-write spool (FR-PRJ-40, FR-RGN-37, R12, R48; schema §4.9). PRJ owns it; RGN's job
//! runner is its only writer and replays it on open and on every tick while entries wait.
//!
//! `<data-dir>/pending/<project-id>/` holds `<seq>.json` = `{seq, op, job_id, payload,
//! created_at}` plus `<seq>-<NN>.<ext>` candidate bytes. The bytes are written before the entry
//! (the entry's rename is the commit point). Entries are replayed in `seq` order once the project
//! is writable and out of conflict; each is deleted after its commit (the `.json` first, then its
//! bytes, R102). Paid output is never dropped. Keyed by project id, so candidates for a closed
//! project wait here too. An entry that can never replay (its bytes are gone, its candidate files
//! are taken, it won't parse) moves to `quarantine/` with its bytes, so later entries go on (R102).
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
/// A folder moved in Explorer counts (R102): the entry waits under the project id and replays
/// when the project is reopened from its new place.
pub fn spoolable(e: &CoreError) -> bool {
    matches!(
        e.code,
        ErrorCode::Conflict | ErrorCode::Locked | ErrorCode::ReadOnly | ErrorCode::Io
    ) || crate::project::is_folder_moved(e)
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

/// Where entries that can never replay are kept, with their bytes (R102).
pub const QUARANTINE_DIR: &str = "quarantine";

fn next_seq(dir: &Path) -> u64 {
    // Quarantined seqs stay taken, so a later entry never collides with one.
    let from_names = [dir.to_path_buf(), dir.join(QUARANTINE_DIR)]
        .iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten())
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

/// The byte files an entry lists (none for a job update).
fn entry_files(entry: &Entry) -> Vec<String> {
    match entry.op {
        Op::AddCandidates => serde_json::from_value::<Vec<SpooledCandidate>>(entry.payload.clone())
            .map(|l| l.into_iter().map(|c| c.file).collect())
            .unwrap_or_default(),
        Op::UpdateJob => vec![],
    }
}

/// R102: the `.json` goes first, so a crash or an AV lock in between leaves stray bytes (never
/// replayed), not an entry whose bytes are gone. Bytes stay if the `.json` can't be removed.
fn remove_entry(dir: &Path, entry: &Entry, path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        tracing::warn!(path = %path.display(), error = %e, "spool entry not removed");
        return;
    }
    for f in entry_files(entry) {
        let _ = std::fs::remove_file(dir.join(f));
    }
}

/// R102: moves an entry that can never replay to `quarantine/`, its bytes first (an entry there
/// always finds them). False if it couldn't move (then it stays and is retried).
fn quarantine(dir: &Path, path: &Path, files: &[String]) -> bool {
    let q = dir.join(QUARANTINE_DIR);
    for f in files {
        let src = dir.join(f);
        if src.exists() && fsx::move_file(&src, &q.join(f)).is_err() {
            return false;
        }
    }
    let name = path.file_name().unwrap_or_default();
    fsx::move_file(path, &q.join(name)).is_ok()
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
    /// Seqs of entries that can never replay, moved to `quarantine/` (R102).
    pub quarantined: Vec<u64>,
}

/// The event that tells the user a spooled write was set aside (R102):
/// `{project_id, job_id, seq, op}`. Its files stay in `quarantine/`; nothing is dropped.
pub const EVENT_SPOOL_QUARANTINED: &str = "project:spool-quarantined";

/// `detail.reason` of a spooled entry whose payload won't parse or whose bytes are gone.
const SPOOL_BYTES_MISSING: &str = "spool_bytes_missing";

/// True if retrying `e` can never help (R102): the entry's candidate bytes are gone, or its
/// files already belong to candidates.
fn never_replays(e: &CoreError) -> bool {
    e.detail["reason"] == store::FILE_TAKEN || e.detail["reason"] == SPOOL_BYTES_MISSING
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
    let pid = p.manifest.project_id;
    let dir = dir(ctx, pid);
    for (seq, path) in entries(&dir) {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "spool entry unreadable; kept");
                continue;
            }
        };
        let Ok(entry) = serde_json::from_slice::<Entry>(&bytes) else {
            // R102: it will never parse; set aside so the entries after it go on.
            tracing::warn!(seq, "spool entry won't parse; quarantined");
            if quarantine(&dir, &path, &[]) {
                out.quarantined.push(seq);
            }
            continue;
        };
        let result = match entry.op {
            Op::UpdateJob => serde_json::from_value::<SpoolUpdate>(entry.payload.clone())
                .map_err(|e| CoreError::new(ErrorCode::Corrupt, e.to_string()))
                .and_then(|u| store::update_job(ctx, p, entry.job_id, u.into()).map(|_| ())),
            Op::AddCandidates => spooled_candidates(&dir, &entry).and_then(|cands| {
                store::add_candidates(ctx, install_id, p, entry.job_id, cands).map(|_| {
                    out.fetched.push(entry.job_id);
                })
            }),
        };
        // Candidates of a job that hasn't got them yet (or whose job.json isn't readable here):
        // paid output, never dropped.
        let paid_waiting = entry.op == Op::AddCandidates
            && p.jobs
                .get(&entry.job_id)
                .is_none_or(|j| !matches!(j.state, JobState::Fetched | JobState::Closed));
        match result {
            Ok(()) => {
                remove_entry(&dir, &entry, &path);
                out.entries += 1;
            }
            Err(e) if spoolable(&e) && !never_replays(&e) => return Ok(out),
            Err(e) if never_replays(&e) || paid_waiting => {
                // R102: an entry that can't replay doesn't hold up the ones after it; set aside
                // with its bytes (nothing paid is dropped).
                tracing::warn!(seq, error = %e, "spool entry can't replay; quarantined");
                if !quarantine(&dir, &path, &entry_files(&entry)) {
                    return Ok(out);
                }
                out.quarantined.push(seq);
                ctx.emit(
                    EVENT_SPOOL_QUARANTINED,
                    serde_json::json!({
                        "project_id": pid,
                        "job_id": entry.job_id,
                        "seq": seq,
                        "op": op_name(entry.op),
                    }),
                );
            }
            Err(e) => {
                tracing::warn!(seq = entry.seq, error = %e, "spool entry refused; dropped");
                remove_entry(&dir, &entry, &path);
                let d = Dropped {
                    seq: entry.seq,
                    job_id: entry.job_id,
                    op: op_name(entry.op),
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

fn op_name(op: Op) -> &'static str {
    match op {
        Op::UpdateJob => "update_job",
        Op::AddCandidates => "add_candidates",
    }
}

/// The candidates of an `add_candidates` entry with their bytes. A payload that won't parse or a
/// byte file that is gone can never replay (R102); other read errors are retried.
fn spooled_candidates(dir: &Path, entry: &Entry) -> CoreResult<Vec<NewCandidate>> {
    let missing = |msg: String| {
        CoreError::new(ErrorCode::Corrupt, msg)
            .with_detail(serde_json::json!({ "reason": SPOOL_BYTES_MISSING }))
    };
    let list: Vec<SpooledCandidate> =
        serde_json::from_value(entry.payload.clone()).map_err(|e| missing(e.to_string()))?;
    let mut cands = vec![];
    for c in list {
        let bytes = match std::fs::read(dir.join(&c.file)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(missing(format!("spooled candidate {} is gone", c.file)));
            }
            Err(e) => return Err(CoreError::io("reading a spooled candidate", &e)),
        };
        cands.push(NewCandidate {
            bytes,
            seed: c.seed,
            provider_meta: c.provider_meta,
            invalid: c.invalid,
            extra: c.extra,
        });
    }
    Ok(cands)
}

#[cfg(test)]
mod tests;

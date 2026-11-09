//! The job runner (FR-RGN-17–24, 35, 37, R14): owns the poll loop and its persistence.
//!
//! - Every job the runner tracks has its own copy of the job file, which is authoritative
//!   between polls. Each provider answer becomes **one commit per transition**: PRJ's
//!   `update_job` while the project is open; `job.json` written directly once it's closed (the
//!   job file is the commit point for job state, schema §3); the spool when a write can't commit
//!   (FR-RGN-37).
//! - A `succeeded` job fetches its candidates into `results/NNN <slug>/` (PRJ `add_candidates`,
//!   FR-RGN-24/39), or into the spool when its project isn't open. Then the runner lets go.
//! - Polling continues after the project closes while the app runs; quitting stops it, and
//!   [`on_project_open`] resumes from the job files (FR-RGN-22).
//! - [`tick`] does the due work and never holds the state lock across a provider call. The app
//!   runs [`run_pump`] on a thread of its own; tests call [`tick`] / [`drive`] directly.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use pf_provider::{
    Cost, CostAmount, Provider, ProviderError, ProviderJobState, ProviderRecord, ProviderResult,
    long_running, poll_delay,
};
use serde_json::{Value, json};
use ulid::Ulid;

use super::spool::{self, SpoolUpdate};
use crate::context::Context;
use crate::core::{Core, State};
use crate::error::{CoreError, CoreResult};
use crate::fsx;
use crate::job::{JOB_FILE, JobFile, JobState};
use crate::manifest::{Extra, job_dir};
use crate::project::OpenProject;
use crate::project::store::{self, NewCandidate};
use crate::time::Timestamp;

/// `rgn:job-state {job_id, state, progress, cost, error?, long_running}` (spec §9).
pub const EVENT_JOB_STATE: &str = "rgn:job-state";
/// `rgn:candidate-ready {job_id, candidate_id, status: ready|invalid}`.
pub const EVENT_CANDIDATE_READY: &str = "rgn:candidate-ready";

/// How long the pump sleeps with nothing due (it also retries spool replays then).
pub const IDLE: Duration = Duration::from_secs(30);

/// Non-transient poll errors in a row before a job goes `lost` with its handle kept (R102).
pub const MAX_POLL_ERRORS: u32 = 10;

/// A candidate's aspect may differ from the sent one by this much before it gets a warning
/// (FR-RGN-24).
pub const ASPECT_TOLERANCE: f64 = 0.01;

/// Wakes the pump early (a submit, a resume).
#[derive(Debug, Default)]
pub struct Wake {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl Wake {
    pub fn notify(&self) {
        *self.flag.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.cv.notify_all();
    }

    /// Sleeps up to `d`, or until [`Wake::notify`].
    pub fn wait(&self, d: Duration) {
        let mut flag = self.flag.lock().unwrap_or_else(|e| e.into_inner());
        if !*flag {
            flag = self
                .cv
                .wait_timeout(flag, d)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
        *flag = false;
    }
}

/// One job the runner is driving.
#[derive(Debug, Clone)]
struct Tracked {
    project_id: Ulid,
    /// Absolute project folder (for direct `job.json` writes once the project is closed).
    project_dir: PathBuf,
    job: JobFile,
    next_at: Timestamp,
    last_error: Option<ProviderError>,
    /// Bumped by every outside change (cancel), so a tick's late answer is discarded.
    generation: u64,
    /// A tick is working on it without the lock.
    busy: bool,
    progress: f64,
    /// Non-transient poll errors in a row (R102); a good answer resets it.
    poll_errors: u32,
}

/// The runner's state, in [`super::RgnState`]. Memory only (spec §8).
#[derive(Debug, Default)]
pub struct Runner {
    jobs: BTreeMap<Ulid, Tracked>,
    generation: u64,
    pub wake: Arc<Wake>,
}

impl Runner {
    /// Starts driving a job (submit, resume). Due at once.
    pub fn track(&mut self, project_id: Ulid, project_dir: PathBuf, job: JobFile, now: Timestamp) {
        self.generation += 1;
        let generation = self.generation;
        self.jobs
            .entry(job.id)
            .and_modify(|t| {
                t.project_dir = project_dir.clone();
            })
            .or_insert(Tracked {
                project_id,
                project_dir,
                job,
                next_at: now,
                last_error: None,
                generation,
                busy: false,
                progress: 0.0,
                poll_errors: 0,
            });
        self.wake.notify();
    }

    /// Stops driving a job; a tick in flight discards its answer.
    pub fn untrack(&mut self, job_id: Ulid) {
        self.jobs.remove(&job_id);
    }

    /// The runner's copy of a job, if it drives it.
    pub fn job(&self, job_id: Ulid) -> Option<&JobFile> {
        self.jobs.get(&job_id).map(|t| &t.job)
    }

    /// The project a tracked job belongs to.
    pub fn project_of(&self, job_id: Ulid) -> Option<(Ulid, PathBuf)> {
        self.jobs
            .get(&job_id)
            .map(|t| (t.project_id, t.project_dir.clone()))
    }

    /// Replaces the runner's copy after an outside commit (cancel) and stops driving it if done.
    pub fn replace(&mut self, job: JobFile) {
        self.generation += 1;
        let generation = self.generation;
        if job.state.is_terminal() || job.state == JobState::Fetched {
            self.jobs.remove(&job.id);
        } else if let Some(t) = self.jobs.get_mut(&job.id) {
            t.job = job;
            t.generation = generation;
        }
    }

    /// Jobs the runner drives (for tests and the tracker).
    pub fn tracked(&self) -> Vec<Ulid> {
        self.jobs.keys().copied().collect()
    }
}

// ---------------------------------------------------------------- helpers

/// A provider call with the connectivity event (like `provider_call`), keeping the raw error.
pub(crate) fn provider_raw<T>(
    core: &Core,
    fut: impl std::future::Future<Output = ProviderResult<T>>,
) -> ProviderResult<T> {
    let before = core.ctx.provider.connectivity();
    let out = pf_provider::block_on(fut);
    let after = core.ctx.provider.connectivity();
    if after != before {
        core.ctx.emit(
            crate::provider_commands::STATUS_CHANGED,
            json!({ "connectivity": after }),
        );
    }
    out
}

/// The job record's `error{code, hint}` for a normalised provider error (ADR 0001).
pub fn error_value(e: &ProviderError) -> Value {
    let hint = match e {
        ProviderError::ProviderFailure { hint } => hint.clone(),
        ProviderError::InsufficientBalance => "Top up the provider balance, then retry.".into(),
        ProviderError::Offline => "The provider can't be reached; polling continues.".into(),
        other => other.to_string(),
    };
    json!({ "code": e.code(), "hint": hint })
}

fn cost_value(c: &Cost) -> Value {
    serde_json::to_value(c).unwrap_or(Value::Null)
}

/// A job's cost for the UI: the actual one when known, else the estimate.
pub fn shown_cost(job: &JobFile) -> Value {
    let known = |v: &Value| v.get("amount").is_some_and(|a| a.is_number());
    if known(&job.cost) {
        job.cost.clone()
    } else if !job.estimate.is_null() {
        job.estimate.clone()
    } else {
        Value::Null
    }
}

fn elapsed(job: &JobFile, now: Timestamp) -> Duration {
    let start = job
        .state_times
        .get(JobState::Submitted.as_str())
        .copied()
        .unwrap_or(job.created_at);
    Duration::from_millis(now.millis().saturating_sub(start.millis()).max(0) as u64)
}

/// Emits `rgn:job-state`.
pub fn emit_state(ctx: &Context, job: &JobFile, progress: Option<f64>) {
    let now = ctx.now();
    ctx.emit(
        EVENT_JOB_STATE,
        json!({
            "job_id": job.id,
            "state": job.state,
            "progress": progress,
            "cost": shown_cost(job),
            "error": job.error,
            "long_running": !job.state.is_terminal()
                && job.state != JobState::Fetched
                && long_running(elapsed(job, now)),
        }),
    );
}

/// Applies an update to a job copy, as PRJ's `update_job` does.
fn apply(job: &mut JobFile, u: &SpoolUpdate, now: Timestamp) -> Result<(), String> {
    match u.state {
        Some(s) => job.transition(s, now)?,
        None => job.updated_at = now,
    }
    if let Some(p) = &u.provider {
        job.provider = p.clone();
    }
    if let Some(s) = u.seed_returned {
        job.seed.returned = Some(s);
    }
    if let Some(c) = &u.cost {
        job.cost = c.clone();
    }
    if let Some(e) = &u.error {
        job.error = (!e.is_null()).then(|| e.clone());
    }
    Ok(())
}

/// The open project, if it's the one the job belongs to.
fn open_project(state: &mut State, project_id: Ulid) -> Option<&mut OpenProject> {
    state
        .project
        .as_mut()
        .filter(|p| p.manifest.project_id == project_id)
}

/// Writes `job.json` straight into a closed project's folder (schema §3: the job file is the
/// commit point for job state).
fn write_closed(ctx: &Context, dir: &std::path::Path, job: &JobFile) -> CoreResult<()> {
    let path = dir.join(job_dir(job.id)).join(JOB_FILE);
    if !dir.is_dir() {
        return Err(CoreError::new(
            crate::error::ErrorCode::NotFound,
            "the project folder is gone",
        ));
    }
    fsx::write_json_atomic(ctx, &path, job)
}

/// Commits one update of a tracked job: store, closed-project file or spool (FR-RGN-37).
/// Returns the job after the update.
/// [`commit_update`] for a job the caller holds (submit, cancel).
pub(crate) fn commit_job_update(
    ctx: &Context,
    state: &mut State,
    project_id: Ulid,
    project_dir: PathBuf,
    job: &JobFile,
    u: SpoolUpdate,
) -> Result<JobFile, String> {
    let t = Tracked {
        project_id,
        project_dir,
        job: job.clone(),
        next_at: ctx.now(),
        last_error: None,
        generation: 0,
        busy: false,
        progress: 0.0,
        poll_errors: 0,
    };
    commit_update(ctx, state, &t, u)
}

fn commit_update(
    ctx: &Context,
    state: &mut State,
    t: &Tracked,
    u: SpoolUpdate,
) -> Result<JobFile, String> {
    let now = ctx.now();
    let mut next = t.job.clone();
    apply(&mut next, &u, now)?;
    let pending = spool::has_pending(ctx, t.project_id);
    match open_project(state, t.project_id) {
        Some(p) if !pending => match store::update_job(ctx, p, t.job.id, u.clone().into()) {
            Ok(j) => return Ok(j),
            Err(e) if spool::spoolable(&e) => {
                if let Err(e) = spool::push_update(ctx, t.project_id, t.job.id, &u) {
                    tracing::error!(job = %t.job.id, error = %e, "job update not spooled");
                }
            }
            Err(e) => {
                tracing::warn!(job = %t.job.id, error = %e, "job update refused by the store");
                // The store's copy differs (e.g. a conflict resolution); trust the file.
                if let Ok(j) = store::job(p, t.job.id) {
                    return Ok(j.clone());
                }
            }
        },
        Some(_) => {
            if let Err(e) = spool::push_update(ctx, t.project_id, t.job.id, &u) {
                tracing::error!(job = %t.job.id, error = %e, "job update not spooled");
            }
        }
        None => {
            if let Err(e) = write_closed(ctx, &t.project_dir, &next) {
                tracing::warn!(job = %t.job.id, error = %e, "job update not written; kept in memory");
            }
        }
    }
    Ok(next)
}

// ---------------------------------------------------------------- the loop

/// One provider step's outcome, applied under the lock.
enum Outcome {
    /// Updates in order (one per transition), the progress, and the error that sets the delay.
    Updates(Vec<SpoolUpdate>, f64, Option<ProviderError>),
    /// Candidates to store (FR-RGN-24); `seed_returned` from the first one.
    Candidates(Vec<NewCandidate>, Option<i64>),
    /// Nothing to commit; retry after the delay.
    Retry(ProviderError),
    /// A non-transient poll error (auth, a stale handle, a provider failure): counted, and the
    /// job goes `lost` after [`MAX_POLL_ERRORS`] in a row (R102).
    PollError(ProviderError),
    /// Not a state the runner drives.
    Done,
}

fn job_state_of(s: ProviderJobState) -> JobState {
    match s {
        ProviderJobState::Queued => JobState::Queued,
        ProviderJobState::Running => JobState::Running,
        ProviderJobState::Succeeded => JobState::Succeeded,
        ProviderJobState::Failed => JobState::Failed,
        ProviderJobState::Canceled => JobState::Canceled,
        ProviderJobState::Unknown => JobState::Unknown,
    }
}

/// The transitions from `from` to `to`, inserting `running` where the provider skipped it
/// (e.g. `submitted → succeeded` is not an edge).
fn path(from: JobState, to: JobState) -> Vec<JobState> {
    if from == to {
        return vec![];
    }
    if from.can_become(to) {
        return vec![to];
    }
    if from.can_become(JobState::Running) && JobState::Running.can_become(to) {
        return vec![JobState::Running, to];
    }
    vec![]
}

fn poll(core: &Core, t: &Tracked) -> Outcome {
    let record = ProviderRecord(t.job.provider.clone());
    match provider_raw(core, core.ctx.provider.status(&record)) {
        Ok(st) => {
            let target = job_state_of(st.state);
            let steps = path(t.job.state, target);
            let mut updates: Vec<SpoolUpdate> = steps
                .iter()
                .map(|&s| SpoolUpdate {
                    state: Some(s),
                    ..Default::default()
                })
                .collect();
            let error = match (target, &st.error) {
                (JobState::Failed | JobState::Unknown, Some(e)) => Some(error_value(e)),
                (JobState::Failed, None) => {
                    Some(json!({"code": "provider_failure", "hint": "the job failed"}))
                }
                _ if t.job.error.is_some() => Some(Value::Null),
                _ => None,
            };
            let last = SpoolUpdate {
                state: None,
                provider: Some(st.provider.0.clone()),
                cost: st.cost.as_ref().map(cost_value),
                error,
                seed_returned: None,
            };
            match updates.last_mut() {
                Some(u) => {
                    u.provider = last.provider;
                    u.cost = last.cost;
                    u.error = last.error;
                }
                None => updates.push(last),
            }
            let delay_error = (target == JobState::Unknown).then_some(ProviderError::Offline);
            Outcome::Updates(updates, st.progress, delay_error)
        }
        Err(e @ (ProviderError::Offline | ProviderError::Timeout)) => {
            // Offline mid-job → `unknown`; the next status decides the exit (FR-RGN-23).
            let u = SpoolUpdate {
                state: t
                    .job
                    .state
                    .can_become(JobState::Unknown)
                    .then_some(JobState::Unknown),
                error: Some(error_value(&ProviderError::Offline)),
                ..Default::default()
            };
            let _ = e;
            Outcome::Updates(vec![u], t.progress, Some(ProviderError::Offline))
        }
        Err(e @ ProviderError::RateLimited { .. }) => Outcome::Retry(e),
        Err(e) => Outcome::PollError(e),
    }
}

/// `lost` after [`MAX_POLL_ERRORS`] poll errors in a row: the handle stays in `provider{}`, the
/// possible-charge notice applies (R102).
fn poll_gave_up(e: &ProviderError) -> SpoolUpdate {
    SpoolUpdate {
        state: Some(JobState::Lost),
        error: Some(json!({
            "code": e.code(),
            "hint": format!(
                "Polling stopped after {MAX_POLL_ERRORS} errors in a row ({e}). The job may still run and be charged; check the provider before retrying."
            ),
        })),
        ..Default::default()
    }
}

fn decodes(bytes: &[u8]) -> Option<(u32, u32)> {
    image::load_from_memory(bytes)
        .ok()
        .map(|i| (i.width(), i.height()))
}

/// What a returned candidate is checked against (FR-RGN-24, FR-GEN-24).
#[derive(Debug, Clone, Copy)]
enum SentSize {
    /// A region edit's `fit{sent_w, sent_h}`: the aspect must match.
    Aspect(f64),
    /// A generate job's exact `output{w, h}`: both sides must match.
    Exact(f64, f64),
}

impl SentSize {
    fn of(job: &JobFile) -> Option<SentSize> {
        if job.is_generate() {
            let o = job.extra.get("output")?;
            return match o.get("mode")?.as_str()? {
                "wh" => {
                    let (w, h) = (o.get("w")?.as_f64()?, o.get("h")?.as_f64()?);
                    (w > 0.0 && h > 0.0).then_some(SentSize::Exact(w, h))
                }
                _ => crate::generate::size::ratio_value(o.get("aspect")?.as_str()?)
                    .map(SentSize::Aspect),
            };
        }
        let w = job.fit.get("sent_w")?.as_f64()?;
        let h = job.fit.get("sent_h")?.as_f64()?;
        (w > 0.0 && h > 0.0).then_some(SentSize::Aspect(w / h))
    }

    /// Off by more than [`ASPECT_TOLERANCE`].
    fn differs(self, w: u32, h: u32) -> bool {
        let off = |a: f64, b: f64| (a / b - 1.0).abs() > ASPECT_TOLERANCE;
        let (w, h) = (f64::from(w), f64::from(h));
        match self {
            SentSize::Aspect(a) => off(w / h, a),
            SentSize::Exact(sw, sh) => off(w, sw) || off(h, sh),
        }
    }
}

fn fetch(core: &Core, t: &Tracked) -> Outcome {
    let record = ProviderRecord(t.job.provider.clone());
    match provider_raw(core, core.ctx.provider.fetch_candidates(&record)) {
        Ok(c) => {
            let sent = SentSize::of(&t.job);
            let mut seed_returned = None;
            let cands: Vec<NewCandidate> = c
                .images
                .into_iter()
                .map(|img| {
                    let size = decodes(&img.bytes);
                    let mut extra = Extra::new();
                    if let (Some((w, h)), Some(sent)) = (size, sent)
                        && sent.differs(w, h)
                    {
                        // Kept as returned, never resized (FR-RGN-24, FR-GEN-24).
                        extra.insert("aspect_warning".into(), json!(true));
                    }
                    let seed = img.seed.map(|s| s as i64);
                    seed_returned = seed_returned.or(seed);
                    NewCandidate {
                        bytes: img.bytes,
                        seed,
                        provider_meta: img.provider_meta,
                        invalid: size.is_none(),
                        extra,
                    }
                })
                .collect();
            Outcome::Candidates(cands, seed_returned)
        }
        Err(
            e @ (ProviderError::Offline
            | ProviderError::Timeout
            | ProviderError::RateLimited { .. }),
        ) => Outcome::Retry(e),
        Err(e) => Outcome::Updates(
            vec![SpoolUpdate {
                state: Some(JobState::Failed),
                error: Some(error_value(&e)),
                ..Default::default()
            }],
            1.0,
            None,
        ),
    }
}

fn unreadable() -> SpoolUpdate {
    SpoolUpdate {
        state: Some(JobState::Failed),
        error: Some(json!({
            "code": "unreadable_candidates",
            "hint": "No returned candidate could be decoded. Retry the job.",
        })),
        ..Default::default()
    }
}

/// Stores the candidates (FR-RGN-24): PRJ `add_candidates` while the project is open and
/// writable, else the spool. Returns the job after the commit (still `succeeded` when spooled),
/// or `None` when neither took them (the runner fetches again later).
fn store_candidates(
    core: &Core,
    state: &mut State,
    t: &Tracked,
    cands: Vec<NewCandidate>,
    seed_returned: Option<i64>,
) -> Option<JobFile> {
    let ctx = &core.ctx;
    let install_id = state.settings.install_id;
    let mut job = t.job.clone();
    if seed_returned.is_some() {
        let u = SpoolUpdate {
            seed_returned,
            ..Default::default()
        };
        if let Ok(j) = commit_update(ctx, state, t, u) {
            job = j;
        }
    }
    let pending = spool::has_pending(ctx, t.project_id);
    let stored = match open_project(state, t.project_id) {
        Some(p) if !pending => {
            match store::add_candidates(ctx, install_id, p, job.id, cands.clone()) {
                Ok(records) => {
                    for c in &records {
                        ctx.emit(
                        EVENT_CANDIDATE_READY,
                        json!({
                            "job_id": job.id,
                            "candidate_id": c.id,
                            "status": if c.status == crate::manifest::CandidateStatus::Invalid { "invalid" } else { "ready" },
                        }),
                    );
                    }
                    store::job(p, job.id).ok().cloned()
                }
                Err(e) => {
                    if !spool::spoolable(&e) {
                        tracing::warn!(job = %job.id, error = %e, "add_candidates refused; spooled");
                    }
                    None
                }
            }
        }
        _ => None,
    };
    match stored {
        Some(j) => Some(j),
        None => match spool::push_candidates(ctx, t.project_id, job.id, &cands) {
            Ok(()) => Some(job),
            Err(e) => {
                tracing::error!(job = %job.id, error = %e, "candidates not spooled; the job will fetch again");
                None
            }
        },
    }
}

/// Works one due job.
fn step(core: &Core, t: Tracked) {
    let outcome = match t.job.state {
        JobState::Submitted | JobState::Queued | JobState::Running | JobState::Unknown => {
            poll(core, &t)
        }
        JobState::Succeeded => fetch(core, &t),
        _ => Outcome::Done,
    };
    let ctx = &core.ctx;
    let mut state = core.state();
    let fresh = state
        .rgn
        .runner
        .jobs
        .get(&t.job.id)
        .is_some_and(|cur| cur.generation == t.generation);
    if !fresh {
        // Canceled or replaced meanwhile: the answer is stale.
        if let Some(cur) = state.rgn.runner.jobs.get_mut(&t.job.id) {
            cur.busy = false;
        }
        return;
    }
    let now = ctx.now();
    let mut cur = t.clone();
    let mut delay_error = None;
    match outcome {
        Outcome::Done => {
            state.rgn.runner.untrack(cur.job.id);
            return;
        }
        Outcome::Retry(e) => delay_error = Some(e),
        Outcome::PollError(e) => {
            cur.poll_errors += 1;
            tracing::warn!(job = %cur.job.id, errors = cur.poll_errors, error = %e, "poll failed");
            if cur.poll_errors >= MAX_POLL_ERRORS {
                match commit_update(ctx, &mut state, &cur, poll_gave_up(&e)) {
                    Ok(j) => cur.job = j,
                    Err(e) => tracing::warn!(job = %cur.job.id, error = %e, "lost not committed"),
                }
                emit_state(ctx, &cur.job, Some(cur.progress));
            }
            delay_error = Some(e);
        }
        Outcome::Updates(updates, progress, err) => {
            cur.poll_errors = 0;
            cur.progress = progress;
            delay_error = err;
            for u in updates {
                match commit_update(ctx, &mut state, &cur, u) {
                    Ok(j) => cur.job = j,
                    Err(e) => {
                        tracing::warn!(job = %cur.job.id, error = %e, "skipped a job transition");
                    }
                }
            }
            emit_state(ctx, &cur.job, Some(cur.progress));
        }
        Outcome::Candidates(cands, seed) => {
            if cands.is_empty() || cands.iter().all(|c| c.invalid) {
                if let Ok(j) = commit_update(ctx, &mut state, &cur, unreadable()) {
                    cur.job = j;
                }
            } else {
                let Some(job) = store_candidates(core, &mut state, &cur, cands, seed) else {
                    cur.busy = false;
                    cur.next_at = now.plus(poll_delay(elapsed(&cur.job, now), None));
                    state.rgn.runner.jobs.insert(cur.job.id, cur);
                    return;
                };
                cur.job = job;
                if cur.job.state == JobState::Succeeded {
                    // Spooled: the job becomes `fetched` when the spool is replayed.
                    state.rgn.runner.untrack(cur.job.id);
                    emit_state(ctx, &cur.job, Some(1.0));
                    return;
                }
            }
            emit_state(ctx, &cur.job, Some(1.0));
        }
    }
    if let Some(c) = cur.job.cost.get("amount")
        && let Ok(amount) = serde_json::from_value::<CostAmount>(c.clone())
    {
        state.rgn.ledger.record_actual(cur.job.id, amount);
    }
    if cur.job.state.is_terminal() || cur.job.state == JobState::Fetched {
        state.rgn.runner.untrack(cur.job.id);
        emit_spend(ctx, &state);
        return;
    }
    cur.busy = false;
    cur.last_error = delay_error;
    cur.next_at = now.plus(poll_delay(elapsed(&cur.job, now), cur.last_error.as_ref()));
    state.rgn.runner.jobs.insert(cur.job.id, cur);
}

pub(crate) fn emit_spend(ctx: &Context, state: &State) {
    ctx.emit(
        super::EVENT_SPEND,
        json!({ "spend": state.rgn.ledger.total(), "cap": state.settings.spend.cap }),
    );
}

/// Does every due job (`force`: every job, whatever its time; tests) and retries the open
/// project's spool. Returns the time until the next job is due, if any.
pub fn tick(core: &Core, force: bool) -> Option<Duration> {
    let now = core.ctx.now();
    let due: Vec<Tracked> = {
        let mut state = core.state();
        replay_open(&core.ctx, &mut state);
        state
            .rgn
            .runner
            .jobs
            .values_mut()
            .filter(|t| !t.busy && (force || t.next_at <= now))
            .map(|t| {
                t.busy = true;
                t.clone()
            })
            .collect()
    };
    for t in due {
        step(core, t);
    }
    let state = core.state();
    let now = core.ctx.now();
    state
        .rgn
        .runner
        .jobs
        .values()
        .map(|t| {
            Duration::from_millis(t.next_at.millis().saturating_sub(now.millis()).max(0) as u64)
        })
        .min()
}

/// Ticks with `force` until no job is tracked or `max` ticks ran (tests, fixtures).
pub fn drive(core: &Core, max: usize) -> usize {
    for i in 0..max {
        if core.state().rgn.runner.jobs.is_empty() {
            return i;
        }
        tick(core, true);
    }
    max
}

/// The app's pump: ticks forever on its own thread (FR-RGN-35). Quitting the app stops it.
pub fn run_pump(core: &Core) -> ! {
    loop {
        let d = tick(core, false).unwrap_or(IDLE).min(IDLE);
        let wake = core.state().rgn.runner.wake.clone();
        wake.wait(d.max(Duration::from_millis(50)));
    }
}

/// Replays the open project's spool if it has entries and is writable (after a conflict or
/// lock is resolved, FR-RGN-37).
fn replay_open(ctx: &Context, state: &mut State) {
    let install_id = state.settings.install_id;
    let Some(p) = state.project.as_mut() else {
        return;
    };
    if p.read_only.is_some() || !spool::has_pending(ctx, p.manifest.project_id) {
        return;
    }
    match spool::replay(ctx, install_id, p) {
        Ok(r) => {
            for id in r.fetched {
                if let Ok(j) = store::job(p, id) {
                    emit_state(ctx, j, Some(1.0));
                }
                for c in p.manifest.candidates.iter().filter(|c| c.job_id == id) {
                    ctx.emit(
                        EVENT_CANDIDATE_READY,
                        json!({"job_id": id, "candidate_id": c.id, "status": if c.status == crate::manifest::CandidateStatus::Invalid { "invalid" } else { "ready" }}),
                    );
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "spool replay failed; retried later"),
    }
}

/// FR-RGN-22, called by `project_open` before its snapshot: replays the spool, then resumes
/// every non-terminal job from its job file (`submitted`, `queued`, `running`, `unknown` →
/// poll; `succeeded` → fetch). PRJ's open sweep already turned `requested` into `lost` with
/// the possible-charge warning.
pub fn on_project_open(
    ctx: &Context,
    install_id: Ulid,
    rgn: &mut super::RgnState,
    p: &mut OpenProject,
) {
    if p.read_only.is_none() && spool::has_pending(ctx, p.manifest.project_id) {
        match spool::replay(ctx, install_id, p) {
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "spool replay on open failed; retried later"),
        }
    }
    let now = ctx.now();
    let resumable: Vec<JobFile> = p
        .jobs
        .values()
        .filter(|j| {
            matches!(
                j.state,
                JobState::Submitted
                    | JobState::Queued
                    | JobState::Running
                    | JobState::Unknown
                    | JobState::Succeeded
            )
        })
        .cloned()
        .collect();
    for job in resumable {
        let busy = rgn.runner.jobs.get(&job.id).is_some_and(|t| t.busy);
        if busy {
            continue;
        }
        match rgn.runner.jobs.get_mut(&job.id) {
            // Still driven since before the close: its copy is the newest.
            Some(t) => t.project_dir = p.dir.clone(),
            None => rgn
                .runner
                .track(p.manifest.project_id, p.dir.clone(), job, now),
        }
    }
    rgn.runner.wake.notify();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_rgn_35_path_inserts_running_where_the_provider_skipped_it() {
        use JobState::*;
        assert_eq!(path(Submitted, Succeeded), vec![Running, Succeeded]);
        assert_eq!(path(Queued, Succeeded), vec![Running, Succeeded]);
        assert_eq!(path(Running, Succeeded), vec![Succeeded]);
        assert_eq!(path(Unknown, Succeeded), vec![Succeeded]);
        assert_eq!(path(Running, Running), Vec::<JobState>::new());
        assert_eq!(path(Running, Queued), Vec::<JobState>::new());
    }
}

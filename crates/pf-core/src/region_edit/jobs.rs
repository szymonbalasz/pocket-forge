//! RGN slice 3 commands (spec §9): submit, cancel, retry draft, dismiss. The poll loop is in
//! [`super::runner`]. Inputs are validated at the boundary; a failed command leaves the asset
//! project unchanged (FR-RGN-31), except that a submit the provider refused or timed out keeps
//! its `failed` / `lost` job record (FR-RGN-17/23).

use pf_provider::{CancelAck, CostAmount, Provider, ProviderError, ProviderRecord};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use ulid::Ulid;

use super::commands::{DraftRegion, JobDraft, prepare_job};
use super::fit::RectIn;
use super::runner::{self, commit_job_update, emit_spend, emit_state, provider_raw};
use super::spend::{self, Confirmations, Need};
use super::spool::SpoolUpdate;
use crate::core::{Command, Core, State, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::job::{JobFile, JobState};
use crate::manifest::{CandidateStatus, Extra};
use crate::project::OpenProject;
use crate::project::store::{self, NewJob, ResolveReason};

/// A job blocks the next submit until it's `fetched` or terminal (#40).
pub fn is_active(state: JobState) -> bool {
    !state.is_terminal() && state != JobState::Fetched
}

fn no_project() -> CoreError {
    CoreError::new(ErrorCode::NoProject, "no asset project is open")
}

/// The open project; `project_id`, when given, must be it.
fn open_project(state: &mut State, project_id: Option<Ulid>) -> CoreResult<&mut OpenProject> {
    let p = state.project.as_mut().ok_or_else(no_project)?;
    if project_id.is_some_and(|id| id != p.manifest.project_id) {
        return Err(
            CoreError::new(ErrorCode::UnknownId, "that asset project is not open")
                .with_field("project_id"),
        );
    }
    Ok(p)
}

/// The newest copy of a job: the runner's while it drives it, else the open project's.
fn find_job(state: &State, job_id: Ulid) -> CoreResult<(Ulid, std::path::PathBuf, JobFile)> {
    if let (Some(j), Some((pid, dir))) = (
        state.rgn.runner.job(job_id),
        state.rgn.runner.project_of(job_id),
    ) {
        return Ok((pid, dir, j.clone()));
    }
    let p = state.project.as_ref().ok_or_else(no_project)?;
    let j = store::job(p, job_id)?;
    Ok((p.manifest.project_id, p.dir.clone(), j.clone()))
}

// ---------------------------------------------------------------- rgn_job_submit

/// The confirmations a submit carries: the `needs` the user confirmed (`["threshold",
/// "unknown_cost"]`, the UI's shape) or `{threshold, unknown_cost}`.
fn confirmations<'de, D: Deserializer<'de>>(d: D) -> Result<Confirmations, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Shape {
        List(Vec<Need>),
        Flags(Confirmations),
    }
    Ok(match Shape::deserialize(d)? {
        Shape::Flags(c) => c,
        Shape::List(needs) => Confirmations {
            threshold: needs.contains(&Need::Threshold),
            unknown_cost: needs.contains(&Need::UnknownCost),
        },
    })
}

/// `rgn_job_submit` (FR-RGN-13–17, 23, 34).
pub struct RgnJobSubmit;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobSubmitInput {
    #[serde(default)]
    pub project_id: Option<Ulid>,
    pub draft: JobDraft,
    pub estimate_id: String,
    #[serde(default, deserialize_with = "confirmations")]
    pub confirmations: Confirmations,
}

impl Validate for JobSubmitInput {
    fn validate(&self) -> CoreResult<()> {
        self.draft.validate()?;
        if self.estimate_id.is_empty() || self.estimate_id.len() > 200 {
            return Err(CoreError::invalid("estimate_id", "not an estimate id"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobSubmitOutput {
    pub job_id: Ulid,
}

/// A random seed in 0..2³² (JSON-safe; the UI shows it), from the randomness seam.
fn random_seed(core: &Core) -> u64 {
    core.ctx.rng.next_u64() & 0xFFFF_FFFF
}

fn with_job_id(e: CoreError, job_id: Ulid, possible_charge: bool) -> CoreError {
    let mut detail = match e.detail.clone() {
        Value::Object(o) => o,
        _ => serde_json::Map::new(),
    };
    detail.insert("job_id".into(), json!(job_id));
    if possible_charge {
        detail.insert("possible_charge".into(), json!(true));
    }
    e.with_detail(Value::Object(detail))
}

impl Command for RgnJobSubmit {
    const NAME: &'static str = "rgn_job_submit";
    type Input = JobSubmitInput;
    type Output = JobSubmitOutput;

    fn run(core: &Core, input: JobSubmitInput) -> CoreResult<JobSubmitOutput> {
        let draft = &input.draft;
        let prepared = prepare_job(core, input.project_id, draft, true)?;
        if prepared.estimate_id != input.estimate_id {
            return Err(CoreError::new(
                ErrorCode::EstimateStale,
                "the job changed since its estimate; estimate again",
            ));
        }
        let now = core.ctx.now();
        let seed = prepared
            .sent
            .seed
            .or_else(|| prepared.caps.seed.then(|| random_seed(core)));
        let mut request = prepared.request(&draft.model_id, &draft.final_prompt);
        request.seed = seed;

        // Spend guard, active-job check and the `requested` record: one lock (FR-RGN-14–17).
        let (job, project_id, project_dir, estimate) = {
            let mut state = core.state();
            let estimate = state
                .rgn
                .estimates
                .get(&prepared.estimate_id, now)
                .ok_or_else(|| {
                    CoreError::new(
                        ErrorCode::EstimateStale,
                        "the estimate expired; estimate again",
                    )
                })?;
            let view = spend::assess(
                state.rgn.ledger.total(),
                &state.settings.spend,
                estimate.amount,
            );
            spend::guard(&view, estimate.amount, input.confirmations)?;
            let install_id = state.settings.install_id;
            let p = open_project(&mut state, input.project_id)?;
            p.require_writable()?;
            if let Some(active) = p.jobs.values().find(|j| is_active(j.state)) {
                return Err(CoreError::new(
                    ErrorCode::JobActive,
                    "this asset project already has a job running",
                )
                .with_detail(json!({ "job_id": active.id })));
            }
            let fit = &prepared.fit;
            let mut extra = Extra::new();
            extra.insert("seed_pinned".into(), json!(draft.seed.is_some()));
            let new = NewJob {
                operation: "edit_masked".into(),
                model_id: draft.model_id.trim().to_owned(),
                capability_snapshot: serde_json::to_value(&prepared.caps)
                    .map_err(|e| CoreError::internal(e.to_string()))?,
                final_prompt: draft.final_prompt.clone(),
                controls: prepared.sent.controls.clone(),
                candidate_count: prepared.sent.candidate_count,
                seed_requested: seed.map(|s| s as i64),
                region: json!({ "inner": fit.inner, "outer": fit.outer, "margin": fit.margin }),
                fit: json!({ "sent_w": fit.sent_w, "sent_h": fit.sent_h, "scale": fit.scale }),
                mask_id: Some(draft.mask_id),
                estimate: serde_json::to_value(&estimate).unwrap_or(Value::Null),
                provider: Value::Null,
                // FR-RGN-43: REF fills these in REF slice 6; `[]` = no references sent.
                references: vec![],
                extra,
            };
            let job = store::record_job(
                &core.ctx,
                install_id,
                p,
                new,
                prepared.images.region_png.clone(),
                prepared.images.mask_png.clone(),
            )?;
            (job, p.manifest.project_id, p.dir.clone(), estimate)
        };
        emit_state(&core.ctx, &job, None);

        // The provider call, without the lock. Submit is never retried (ADR 0001).
        let result = provider_raw(core, core.ctx.provider.submit(&request));

        let ctx = &core.ctx;
        let mut state = core.state();
        let (update, err, charged) = match &result {
            Ok(record) => (
                SpoolUpdate {
                    state: Some(JobState::Submitted),
                    provider: Some(record.0.clone()),
                    ..Default::default()
                },
                None,
                true,
            ),
            // FR-RGN-23: a timeout may still have started (and charged) the job.
            Err(e @ ProviderError::Timeout) => (
                SpoolUpdate {
                    state: Some(JobState::Lost),
                    error: Some(json!({
                        "code": "timeout",
                        "hint": "The provider didn't answer the submit. The job may still run and be charged; check the provider before retrying.",
                    })),
                    ..Default::default()
                },
                Some(with_job_id(CoreError::from(e.clone()), job.id, true)),
                true,
            ),
            Err(e) => (
                SpoolUpdate {
                    state: Some(JobState::Failed),
                    error: Some(runner::error_value(e)),
                    ..Default::default()
                },
                Some(with_job_id(CoreError::from(e.clone()), job.id, false)),
                false,
            ),
        };
        let job = commit_job_update(
            ctx,
            &mut state,
            project_id,
            project_dir.clone(),
            &job,
            update,
        )
        .unwrap_or(job);
        if charged {
            // A lost job counts its estimate (FR-RGN-14).
            state.rgn.ledger.record_submit(job.id, estimate.amount);
        }
        if job.state == JobState::Submitted {
            // The next submit resolves the earlier jobs (FR-RGN-28).
            let install_id = state.settings.install_id;
            if let Some(p) = state
                .project
                .as_mut()
                .filter(|p| p.manifest.project_id == project_id)
            {
                let earlier: Vec<Ulid> = p
                    .jobs
                    .values()
                    .filter(|j| {
                        j.id != job.id
                            && j.resolved_at.is_none()
                            && (j.state == JobState::Fetched || j.state.is_terminal())
                    })
                    .map(|j| j.id)
                    .collect();
                for id in earlier {
                    if let Err(e) =
                        store::resolve_job(ctx, install_id, p, id, ResolveReason::NextSubmit)
                    {
                        tracing::warn!(job = %id, error = %e, "earlier job not resolved");
                    }
                }
            }
            state
                .rgn
                .runner
                .track(project_id, project_dir, job.clone(), ctx.now());
        }
        emit_state(ctx, &job, None);
        emit_spend(ctx, &state);
        match err {
            Some(e) => Err(e),
            None => Ok(JobSubmitOutput { job_id: job.id }),
        }
    }
}

// ---------------------------------------------------------------- rgn_job_cancel

/// `rgn_job_cancel` (FR-RGN-19).
pub struct RgnJobCancel;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobIdInput {
    pub job_id: Ulid,
}

impl Validate for JobIdInput {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CancelOutput {
    pub result: CancelAck,
}

impl Command for RgnJobCancel {
    const NAME: &'static str = "rgn_job_cancel";
    type Input = JobIdInput;
    type Output = CancelOutput;

    fn run(core: &Core, input: JobIdInput) -> CoreResult<CancelOutput> {
        let (project_id, dir, job) = find_job(&core.state(), input.job_id)?;
        if !matches!(
            job.state,
            JobState::Submitted | JobState::Queued | JobState::Running
        ) {
            return Err(CoreError::invalid(
                "job_id",
                format!("a {} job can't be canceled", job.state.as_str()),
            ));
        }
        let ack = provider_raw(
            core,
            core.ctx
                .provider
                .cancel(&ProviderRecord(job.provider.clone())),
        )?;
        if ack == CancelAck::NotCancellable {
            return Ok(CancelOutput { result: ack });
        }
        let ctx = &core.ctx;
        let mut state = core.state();
        // The runner may have moved the job on meanwhile.
        let (_, _, current) = find_job(&state, input.job_id)?;
        if !current.state.can_become(JobState::Canceled) {
            return Ok(CancelOutput {
                result: CancelAck::NotCancellable,
            });
        }
        let u = SpoolUpdate {
            state: Some(JobState::Canceled),
            ..Default::default()
        };
        let done = commit_job_update(ctx, &mut state, project_id, dir, &current, u)
            .map_err(CoreError::internal)?;
        state.rgn.runner.replace(done.clone());
        emit_state(ctx, &done, None);
        emit_spend(ctx, &state);
        Ok(CancelOutput { result: ack })
    }
}

// ---------------------------------------------------------------- rgn_job_retry_draft

/// `rgn_job_retry_draft` (FR-RGN-21, 44): the job's draft, a new random seed unless pinned.
/// The panel estimates and confirms again; nothing is submitted.
pub struct RgnJobRetryDraft;

/// The job's region as a draft region.
fn draft_region(job: &JobFile) -> CoreResult<DraftRegion> {
    let inner: RectIn = serde_json::from_value(job.region["inner"].clone())
        .map_err(|_| CoreError::new(ErrorCode::Corrupt, "the job has no region"))?;
    let margin = job.region["margin"].as_u64().unwrap_or(0) as u32;
    Ok(DraftRegion { inner, margin })
}

impl Command for RgnJobRetryDraft {
    const NAME: &'static str = "rgn_job_retry_draft";
    type Input = JobIdInput;
    type Output = JobDraft;

    fn run(core: &Core, input: JobIdInput) -> CoreResult<JobDraft> {
        let state = core.state();
        let (_, _, job) = find_job(&state, input.job_id)?;
        let p = state.project.as_ref().ok_or_else(no_project)?;
        let mask_id = job
            .mask_id
            .filter(|id| p.manifest.masks.iter().any(|m| m.id == *id))
            .ok_or_else(|| {
                CoreError::new(
                    ErrorCode::SourceMissing,
                    "the job's mask is gone; paint the mask again",
                )
            })?;
        let pinned = job.extra.get("seed_pinned").and_then(Value::as_bool) == Some(true);
        Ok(JobDraft {
            region: draft_region(&job)?,
            mask_id,
            model_id: job.model_id.clone(),
            final_prompt: job.final_prompt.clone(),
            controls: job.controls.clone(),
            seed: if pinned {
                job.seed.requested.map(|s| s as u64)
            } else {
                None
            },
            // FR-RGN-44: REF restores the job's references in REF slice 6.
            references: None,
        })
    }
}

// ---------------------------------------------------------------- rgn_job_dismiss

/// `rgn_job_dismiss` (FR-RGN-28, R9): ends a fetched or terminal job with nothing accepted.
pub struct RgnJobDismiss;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DismissOutput {
    /// Candidates this dismiss sent to the trash.
    pub trashed: usize,
}

impl Command for RgnJobDismiss {
    const NAME: &'static str = "rgn_job_dismiss";
    type Input = JobIdInput;
    type Output = DismissOutput;

    fn run(core: &Core, input: JobIdInput) -> CoreResult<DismissOutput> {
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let p = open_project(&mut state, None)?;
        p.require_writable()?;
        let job = store::job(p, input.job_id)?.clone();
        if is_active(job.state) {
            return Err(CoreError::new(
                ErrorCode::JobActive,
                "the job is still running; cancel it first",
            ));
        }
        let in_trash = |p: &OpenProject| {
            p.manifest
                .candidates
                .iter()
                .filter(|c| c.job_id == job.id && c.trash_id.is_some())
                .count()
        };
        let before = in_trash(p);
        store::resolve_job(&core.ctx, install_id, p, job.id, ResolveReason::Dismiss)?;
        let trashed = in_trash(p).saturating_sub(before);
        if let Ok(j) = store::job(p, job.id) {
            emit_state(&core.ctx, j, None);
        }
        Ok(DismissOutput { trashed })
    }
}

/// Candidates of a job that are kept (not in the trash, not invalid).
pub fn kept(p: &OpenProject, job_id: Ulid) -> usize {
    p.manifest
        .candidates
        .iter()
        .filter(|c| {
            c.job_id == job_id && c.trash_id.is_none() && c.status != CandidateStatus::Invalid
        })
        .count()
}

/// The estimate amount of a job record (`estimate{amount, unit}`).
pub fn estimate_amount(job: &JobFile) -> CostAmount {
    serde_json::from_value(job.estimate["amount"].clone()).unwrap_or(CostAmount::Unknown)
}

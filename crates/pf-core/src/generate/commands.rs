//! GEN core commands, slice 3 (new-asset-generation §9): size fit, controls, estimate, submit
//! and the retry draft. Estimate and submit go through RGN's estimate cache, spend guard and
//! job runner unchanged (FR-GEN-20–23); references and the sketch through REF's prepare
//! (FR-GEN-13/15); the final prompt through PRM's `compose` (FR-GEN-16). Inputs are validated
//! at the boundary; a failed command leaves the project unchanged (FR-GEN-35), except that a
//! submit the provider refused or timed out keeps its `failed` / `lost` record (FR-RGN-17/23).

use std::collections::BTreeMap;

use pf_provider::{Capabilities, Cost, JobRequest, Operation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ulid::Ulid;

use super::size::{self, GenOutput, GenSize, GenSizeFit};
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::job::{JobFile, OPERATION_GENERATE};
use crate::manifest::{Extra, Kind};
use crate::project::store::NewJob;
use crate::prompt::{self, FinalPrompt, helpers};
use crate::references::prepare::{self as ref_prepare, JobReference, STRUCTURE_DIR};
use crate::references::resolve::{Notice, Source};
use crate::references::{DraftReferences, Prepared};
use crate::region_edit::commands::{
    MAX_PROMPT_CHARS, capabilities_for, check_controls_map, check_model_id, estimate_cost,
};
use crate::region_edit::controls::{self, ControlsEvaluation, SentControls};
use crate::region_edit::estimate::generate_estimate_id;
use crate::region_edit::jobs::{
    self, JobIdInput, JobSubmitOutput, SubmitJob, find_job, random_seed, retry_controls,
};
use crate::region_edit::spend::{Confirmations, SpendView};

const MAX_HELPER_TICKS: usize = 200;

/// The model's `generate` capabilities (R76). A model without a generate row → `unknown_model`.
pub fn capabilities(core: &Core, model_id: &str) -> CoreResult<Capabilities> {
    capabilities_for(core, model_id, Operation::Generate)
}

fn output_size(caps: &Capabilities) -> CoreResult<&pf_provider::OutputSize> {
    caps.output_size.as_ref().ok_or_else(|| {
        CoreError::new(
            ErrorCode::UnknownModel,
            format!("{} can't generate a new image", caps.model_id),
        )
        .with_field("model_id")
    })
}

// ---------------------------------------------------------------- gen_size_fit

/// `gen_size_fit` (FR-GEN-07–10): what the Size block sends to this model.
pub struct GenSizeFitCmd;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SizeFitInput {
    pub model_id: String,
    /// The project's kind (the UI sends it; the fit itself depends only on `size`).
    #[serde(default)]
    pub kind: Option<Kind>,
    pub size: GenSize,
}

impl Validate for SizeFitInput {
    fn validate(&self) -> CoreResult<()> {
        check_model_id("model_id", &self.model_id)?;
        self.size.validate("size")
    }
}

impl Command for GenSizeFitCmd {
    const NAME: &'static str = "gen_size_fit";
    type Input = SizeFitInput;
    type Output = GenSizeFit;

    fn run(core: &Core, input: SizeFitInput) -> CoreResult<GenSizeFit> {
        let caps = capabilities(core, &input.model_id)?;
        size::fit(output_size(&caps)?, &input.size)
    }
}

// ---------------------------------------------------------------- gen_controls_evaluate

/// `gen_controls_evaluate` (FR-GEN-06): as FR-RGN-33, on the `generate` capabilities.
pub struct GenControlsEvaluate;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlsEvaluateInput {
    pub model_id: String,
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
}

impl Validate for ControlsEvaluateInput {
    fn validate(&self) -> CoreResult<()> {
        check_model_id("model_id", &self.model_id)?;
        check_controls_map("values", &self.values)
    }
}

impl Command for GenControlsEvaluate {
    const NAME: &'static str = "gen_controls_evaluate";
    type Input = ControlsEvaluateInput;
    type Output = ControlsEvaluation;

    fn run(core: &Core, input: ControlsEvaluateInput) -> CoreResult<ControlsEvaluation> {
        let caps = capabilities(core, &input.model_id)?;
        Ok(controls::evaluate_controls(&caps, &input.values))
    }
}

// ---------------------------------------------------------------- the draft

/// The chosen sketch (`draft.generate.structure`). A retry draft's flags, if sent back, are
/// ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructureChoice {
    pub file: String,
}

/// The draft estimate and submit act on (`GenJobDraft`, R77): `draft.generate` plus the final
/// prompt the panel shows.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GenJobDraft {
    pub model_id: String,
    /// Control name → value, as the user set them (FR-RGN-34).
    #[serde(default)]
    pub controls: BTreeMap<String, Value>,
    /// Pinned seed, or `null` = random (FR-GEN-18).
    #[serde(default)]
    pub seed: Option<u64>,
    pub size: GenSize,
    /// The sketch in `structure/`, or `null` = None (FR-GEN-14).
    #[serde(default)]
    pub structure: Option<StructureChoice>,
    /// REF's selection (`draft.generate.references`, FR-GEN-13); `null` = REF's defaults.
    #[serde(default)]
    pub references: Option<DraftReferences>,
    #[serde(default)]
    pub typed_prompt: String,
    /// PRM's helper ticks (FR-PRM-05); absent = the defaults.
    #[serde(default)]
    pub prompt_helpers: Option<Vec<String>>,
    /// The panel's preview of the final prompt. Informational: the core composes the prompt it
    /// sends from `typed_prompt` and `prompt_helpers` (FR-GEN-16).
    #[serde(default)]
    pub final_prompt: Option<String>,
}

impl Validate for GenJobDraft {
    fn validate(&self) -> CoreResult<()> {
        check_model_id("draft.model_id", &self.model_id)?;
        check_controls_map("draft.controls", &self.controls)?;
        self.size.validate("draft.size")?;
        for (f, text) in [
            ("draft.typed_prompt", Some(&self.typed_prompt)),
            ("draft.final_prompt", self.final_prompt.as_ref()),
        ] {
            if text.is_some_and(|t| t.chars().count() > MAX_PROMPT_CHARS) {
                return Err(CoreError::invalid(f, "the prompt is too long"));
            }
        }
        if let Some(s) = &self.structure {
            ref_prepare::check_structure_file("draft.structure.file", &s.file)?;
        }
        if let Some(t) = &self.prompt_helpers
            && (t.len() > MAX_HELPER_TICKS || t.iter().any(|i| i.is_empty() || i.len() > 200))
        {
            return Err(CoreError::invalid(
                "draft.prompt_helpers",
                "not a list of helper ids",
            ));
        }
        if let Some(r) = &self.references
            && (r.packs.len() > 1000
                || r.items.len() > 10_000
                || r.items
                    .iter()
                    .any(|i| i.key.is_empty() || i.key.len() > 2400))
        {
            return Err(CoreError::invalid(
                "draft.references",
                "not a valid references selection",
            ));
        }
        Ok(())
    }
}

/// Everything a generate job would send, after every local check (estimate and submit).
#[derive(Debug, Clone)]
pub struct PreparedGen {
    pub caps: Capabilities,
    pub fit: GenSizeFit,
    pub output: GenOutput,
    pub prompt: FinalPrompt,
    pub sent: SentControls,
    /// The sketch row first, then REF's references (FR-GEN-15).
    pub references: Prepared,
    pub estimate_id: String,
}

impl PreparedGen {
    /// PRV's request: no source, no mask, the output size (FR-PRV-36/37).
    pub fn request(&self, model_id: &str) -> JobRequest {
        JobRequest {
            operation: Operation::Generate,
            model_id: model_id.trim().to_owned(),
            final_prompt: self.prompt.text.clone(),
            controls: self.sent.controls.clone(),
            references: self.references.provider_references(),
            region_png: vec![],
            mask_png: vec![],
            candidate_count: self.sent.candidate_count,
            seed: self.sent.seed,
            output: Some(self.fit.request()),
        }
    }

    /// References the confirm counts ("with N references"): the sketch is not one.
    pub fn reference_count(&self) -> usize {
        self.references
            .sent
            .iter()
            .filter(|r| r.row.source != Source::Structure)
            .count()
    }
}

/// The local half of estimate and submit: the open project (writable, folder present), the
/// generate capabilities, the size fit, PRM's compose, the controls check, REF's prepare with
/// the sketch. No provider call except `get_capabilities`. Runs without the state lock.
pub fn prepare_gen(
    core: &Core,
    project_id: Option<Ulid>,
    draft: &GenJobDraft,
    for_submit: bool,
) -> CoreResult<PreparedGen> {
    let (kind, list) = {
        let mut state = core.state();
        let list = helpers::effective(&state.settings);
        let p = state
            .project
            .as_mut()
            .ok_or_else(|| CoreError::new(ErrorCode::NoProject, "no asset project is open"))?;
        crate::project::require_folder(p)?;
        if project_id.is_some_and(|id| id != p.manifest.project_id) {
            return Err(
                CoreError::new(ErrorCode::UnknownId, "that asset project is not open")
                    .with_field("project_id"),
            );
        }
        if p.read_only.is_some() {
            return Err(CoreError::new(
                ErrorCode::ReadOnly,
                "this asset project is read-only: no estimates or jobs",
            ));
        }
        (p.manifest.kind, list)
    };
    let caps = capabilities(core, &draft.model_id)?;
    let fit = size::fit(output_size(&caps)?, &draft.size)?;
    let output = fit.output(&draft.size);
    // FR-GEN-16: typed prompt + ticked helpers, never the blend instruction.
    let prompt = prompt::compose(
        kind,
        &draft.typed_prompt,
        draft.prompt_helpers.as_deref(),
        &list,
        None,
    );
    let sent =
        controls::check_controls(&caps, &draft.controls, &prompt.text, draft.seed, for_submit)?;
    let structure = draft.structure.as_ref().map(|s| s.file.as_str());
    let references =
        ref_prepare::prepare_for_send_generate(core, &caps, draft.references.as_ref(), structure)?;
    let refs = ref_prepare::estimate_inputs(&references.sent, &caps.slots);
    let estimate_id = generate_estimate_id(
        draft.model_id.trim(),
        &fit.request(),
        &sent.cost_inputs,
        &refs,
    );
    Ok(PreparedGen {
        caps,
        fit,
        output,
        prompt,
        sent,
        references,
        estimate_id,
    })
}

// ---------------------------------------------------------------- gen_job_estimate

/// `gen_job_estimate` (FR-GEN-20/21).
pub struct GenJobEstimate;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobEstimateInput {
    #[serde(default)]
    pub project_id: Option<Ulid>,
    pub draft: GenJobDraft,
}

impl Validate for JobEstimateInput {
    fn validate(&self) -> CoreResult<()> {
        self.draft.validate()
    }
}

/// `GenEstimate` (R77): RGN's estimate without the region fit, plus the output.
#[derive(Debug, Serialize)]
pub struct JobEstimateOutput {
    pub estimate_id: String,
    pub cost: Cost,
    pub candidate_count: u32,
    /// References sent, after dedupe; the sketch is not counted (the confirm text).
    pub references: usize,
    pub reference_notices: Vec<Notice>,
    pub output: GenOutput,
    #[serde(flatten)]
    pub spend: SpendView,
}

impl Command for GenJobEstimate {
    const NAME: &'static str = "gen_job_estimate";
    type Input = JobEstimateInput;
    type Output = JobEstimateOutput;

    fn run(core: &Core, input: JobEstimateInput) -> CoreResult<JobEstimateOutput> {
        let job = prepare_gen(core, input.project_id, &input.draft, false)?;
        let (cost, spend) =
            estimate_cost(core, &job.estimate_id, job.request(&input.draft.model_id))?;
        Ok(JobEstimateOutput {
            references: job.reference_count(),
            estimate_id: job.estimate_id,
            cost,
            candidate_count: job.sent.candidate_count,
            reference_notices: job.references.notices,
            output: job.output,
            spend,
        })
    }
}

// ---------------------------------------------------------------- gen_job_submit

/// `gen_job_submit` (FR-GEN-21–23): the job record, then RGN's runner.
pub struct GenJobSubmit;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobSubmitInput {
    #[serde(default)]
    pub project_id: Option<Ulid>,
    pub draft: GenJobDraft,
    pub estimate_id: String,
    #[serde(default, deserialize_with = "jobs::confirmations")]
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

impl Command for GenJobSubmit {
    const NAME: &'static str = "gen_job_submit";
    type Input = JobSubmitInput;
    type Output = JobSubmitOutput;

    fn run(core: &Core, input: JobSubmitInput) -> CoreResult<JobSubmitOutput> {
        let draft = &input.draft;
        let prepared = prepare_gen(core, input.project_id, draft, true)?;
        if prepared.estimate_id != input.estimate_id {
            return Err(CoreError::new(
                ErrorCode::EstimateStale,
                "the job changed since its estimate; estimate again",
            ));
        }
        let seed = prepared
            .sent
            .seed
            .or_else(|| prepared.caps.seed.then(|| random_seed(core)));
        let mut request = prepared.request(&draft.model_id);
        request.seed = seed;
        let mut extra = Extra::new();
        extra.insert("seed_pinned".into(), json!(draft.seed.is_some()));
        // FR-GEN-22: the prompt as typed (slug, pick label, Retry); never the blend instruction.
        extra.insert("typed_prompt".into(), json!(draft.typed_prompt));
        extra.insert("blend_instruction".into(), json!(false));
        prompt::record_on_job(&mut extra, &prepared.prompt.sent);
        extra.insert("output".into(), to_value(&prepared.output)?);
        // The size as asked (Retry restores it; `output` is what was sent).
        extra.insert("size".into(), to_value(&draft.size)?);
        let references = prepared
            .references
            .rows()
            .iter()
            .map(to_value)
            .collect::<CoreResult<Vec<_>>>()?;
        let new = NewJob {
            operation: OPERATION_GENERATE.into(),
            model_id: draft.model_id.trim().to_owned(),
            capability_snapshot: to_value(&prepared.caps)?,
            final_prompt: prepared.prompt.text.clone(),
            controls: prepared.sent.controls.clone(),
            candidate_count: prepared.sent.candidate_count,
            seed_requested: seed.map(|s| s as i64),
            // FR-GEN-22: no region, fit, mask or inputs.
            region: Value::Null,
            fit: Value::Null,
            mask_id: None,
            estimate: Value::Null,
            provider: Value::Null,
            references,
            extra,
        };
        jobs::submit_job(
            core,
            SubmitJob {
                project_id: input.project_id,
                estimate_id: &prepared.estimate_id,
                confirmations: input.confirmations,
                request,
                new,
                inputs: None,
            },
        )
    }
}

/// Serialises a job field (an `internal` error is a bug).
fn to_value<T: Serialize>(v: &T) -> CoreResult<Value> {
    serde_json::to_value(v).map_err(|e| CoreError::internal(e.to_string()))
}

// ---------------------------------------------------------------- gen_job_retry_draft

/// `gen_job_retry_draft` (FR-GEN-32): the job's draft for the New image tab. Nothing is
/// submitted; the tab estimates and confirms again.
pub struct GenJobRetryDraft;

/// The sketch as the retry finds it: by path and hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetryStructure {
    pub file: String,
    /// `NNN`: the file's bytes differ from what that job sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_since: Option<String>,
    /// The file is gone from `structure/`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
}

/// `GenRetryDraft` (R77).
#[derive(Debug, Clone, Serialize)]
pub struct RetryDraft {
    pub model_id: String,
    pub controls: BTreeMap<String, Value>,
    /// The job's seed when it was pinned, else `null` (a new random one at submit).
    pub seed: Option<u64>,
    pub size: Option<GenSize>,
    pub structure: Option<RetryStructure>,
    pub references: Option<DraftReferences>,
    pub typed_prompt: String,
    /// FR-PRM-10: the ticks restored from the job's `prompt_helpers[]`.
    pub prompt_helpers: Vec<String>,
    /// Ticked helpers whose text changed since the job (badged `changed since NNN`).
    pub prompt_helpers_changed: Vec<String>,
    /// Recorded helpers that can't be sent any more (listed, unticked).
    pub prompt_helpers_missing: Vec<prompt::SentHelper>,
}

/// `NNN` of the job's iteration, or "the job" before it has one.
fn label(job: &JobFile) -> String {
    match &job.iteration {
        Some(it) => format!("{:03}", it.n),
        None => "the job".to_owned(),
    }
}

/// The job's size as asked; a job without `size` falls back to its grid.
fn retry_size(job: &JobFile) -> Option<GenSize> {
    if let Some(s) = job
        .extra
        .get("size")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
    {
        return Some(s);
    }
    let out: GenOutput = serde_json::from_value(job.extra.get("output")?.clone()).ok()?;
    out.grid.map(|g| GenSize::Grid {
        cols: g.cols,
        rows: g.rows,
        pps: g.pps,
    })
}

fn retry_structure(core: &Core, job: &JobFile, dir: &std::path::Path) -> Option<RetryStructure> {
    let row: JobReference = job
        .references
        .iter()
        .filter_map(|v| serde_json::from_value::<JobReference>(v.clone()).ok())
        .find(|r| r.source == Source::Structure)?;
    let file = row
        .path
        .strip_prefix(&format!("{STRUCTURE_DIR}/"))
        .unwrap_or(&row.path)
        .to_owned();
    let path = dir.join(STRUCTURE_DIR).join(&file);
    if std::fs::symlink_metadata(&path).is_err() {
        return Some(RetryStructure {
            file,
            changed_since: None,
            missing: true,
        });
    }
    // A cloud-only file is never hydrated here; it reads as unchanged until it is sent.
    let changed =
        crate::references::retry::current_hash(core, &path).is_some_and(|h| h != row.content_hash);
    Some(RetryStructure {
        file,
        changed_since: changed.then(|| label(job)),
        missing: false,
    })
}

impl Command for GenJobRetryDraft {
    const NAME: &'static str = "gen_job_retry_draft";
    type Input = JobIdInput;
    type Output = RetryDraft;

    fn run(core: &Core, input: JobIdInput) -> CoreResult<RetryDraft> {
        let (job, dir, settings, kind) = {
            let state = core.state();
            let (_, _, job) = find_job(&state, input.job_id).map_err(|e| {
                if e.code == ErrorCode::UnknownId {
                    CoreError::new(ErrorCode::NotFound, "no such job").with_field("job_id")
                } else {
                    e
                }
            })?;
            let p = state
                .project
                .as_ref()
                .ok_or_else(|| CoreError::new(ErrorCode::NoProject, "no asset project is open"))?;
            (job, p.dir.clone(), state.settings.clone(), p.manifest.kind)
        };
        if !job.is_generate() {
            return Err(CoreError::new(
                ErrorCode::WrongOperation,
                "that job is a region edit; retry it from Region edit",
            )
            .with_field("job_id"));
        }
        let pinned = job.extra.get("seed_pinned").and_then(Value::as_bool) == Some(true);
        let ticks =
            helpers::retry_ticks(kind, &job.prompt_helpers(), &helpers::effective(&settings));
        Ok(RetryDraft {
            model_id: job.model_id.clone(),
            controls: retry_controls(&job),
            seed: if pinned {
                job.seed.requested.map(|s| s as u64)
            } else {
                None
            },
            size: retry_size(&job),
            structure: retry_structure(core, &job, &dir),
            references: jobs::retry_references(core, &job, &dir, &settings),
            typed_prompt: job.typed_prompt().to_owned(),
            prompt_helpers: ticks.ticks,
            prompt_helpers_changed: ticks.changed,
            prompt_helpers_missing: ticks.missing,
        })
    }
}

// ---------------------------------------------------------------- gen_candidate_preview / _pick

/// `{job_id, candidate_id}` of a generate candidate.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateInput {
    pub job_id: Ulid,
    pub candidate_id: Ulid,
}

impl Validate for CandidateInput {}

fn no_project() -> CoreError {
    CoreError::new(ErrorCode::NoProject, "no asset project is open")
}

/// The open project's generate job `job_id` (`not_found` / `wrong_operation`).
fn generate_job(p: &crate::project::OpenProject, job_id: Ulid) -> CoreResult<&JobFile> {
    let job = p
        .jobs
        .get(&job_id)
        .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "no such job").with_field("job_id"))?;
    if !job.is_generate() {
        return Err(CoreError::new(
            ErrorCode::WrongOperation,
            "that job is a region edit; accept it from Region edit",
        )
        .with_field("job_id"));
    }
    Ok(job)
}

/// `gen_candidate_preview` (FR-GEN-26, OQ-GEN-13): the candidate registered as a transient image
/// (FR-EDT-31) over its own size. `same_size` = it matches the working image, so EDT compares
/// (split, flip); otherwise, or with no image, EDT shows it alone, fit to view. A candidate with
/// alpha is shown over the flatten colour, as the canvas will show it after a pick (FR-GEN-12).
pub struct GenCandidatePreview;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreviewOutput {
    pub key: String,
    pub url: String,
    /// The candidate's own rect, image px from the top left.
    pub rect: crate::region_edit::fit::RectPx,
    pub same_size: bool,
}

impl Command for GenCandidatePreview {
    const NAME: &'static str = "gen_candidate_preview";
    type Input = CandidateInput;
    type Output = PreviewOutput;

    fn run(core: &Core, input: CandidateInput) -> CoreResult<PreviewOutput> {
        let (project_id, path, working) = {
            let state = core.state();
            let p = state.project.as_ref().ok_or_else(no_project)?;
            generate_job(p, input.job_id)?;
            let cand = p
                .manifest
                .candidates
                .iter()
                .find(|c| c.id == input.candidate_id)
                .ok_or_else(|| {
                    CoreError::new(ErrorCode::NotFound, "no such candidate")
                        .with_field("candidate_id")
                })?;
            if cand.job_id != input.job_id {
                return Err(CoreError::new(
                    ErrorCode::InvalidCandidate,
                    "the candidate belongs to another job",
                )
                .with_field("candidate_id"));
            }
            if cand.status == crate::manifest::CandidateStatus::Invalid {
                return Err(CoreError::new(
                    ErrorCode::InvalidCandidate,
                    "the candidate won't decode",
                )
                .with_field("candidate_id"));
            }
            let path = p.resolve_image(input.candidate_id)?.path;
            (p.manifest.project_id, path, p.manifest.image_size())
        };
        // The decode and the flatten run without the state lock.
        crate::library::hydrate_if_needed(&core.ctx, &path)?;
        let bytes = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CoreError::new(ErrorCode::NotFound, "the candidate's file is gone")
            } else {
                CoreError::io("reading the candidate", &e)
            }
        })?;
        let info = pf_imaging::probe_bytes(&bytes).map_err(crate::project::probe_error)?;
        let bytes = flattened_if_alpha(bytes)?;
        let reg = crate::tiles::register_transient_image(core, project_id, bytes)?;
        Ok(PreviewOutput {
            key: reg.key,
            url: reg.url,
            rect: crate::region_edit::fit::RectPx {
                x: 0,
                y: 0,
                w: info.width,
                h: info.height,
            },
            same_size: working == Some((info.width, info.height)),
        })
    }
}

/// An image with alpha as an opaque PNG over the flatten colour `#808080` (FR-GEN-12); an
/// opaque one as it is.
pub(crate) fn flattened_if_alpha(bytes: Vec<u8>) -> CoreResult<Vec<u8>> {
    let img = image::load_from_memory(&bytes).map_err(|e| {
        CoreError::new(
            ErrorCode::InvalidCandidate,
            format!("the candidate won't decode: {e}"),
        )
        .with_field("candidate_id")
    })?;
    if !img.color().has_alpha() {
        return Ok(bytes);
    }
    let rgb = pf_imaging::to_rgb8_flattened(img);
    let mut out = Vec::new();
    rgb.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| CoreError::internal(format!("encoding the preview: {e}")))?;
    Ok(out)
}

/// `gen_candidate_pick` (FR-GEN-27–30): PRJ's pick commit. The candidate becomes the base image
/// (stack = `[base layer]`), from any generate iteration, no provider call, no spend; undo is
/// `project_restore_revision(parent)` (FR-GEN-29).
pub struct GenCandidatePick;

impl Command for GenCandidatePick {
    const NAME: &'static str = "gen_candidate_pick";
    type Input = CandidateInput;
    type Output = crate::project::store::BaseCommit;

    fn run(core: &Core, input: CandidateInput) -> CoreResult<crate::project::store::BaseCommit> {
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let p = state.project.as_mut().ok_or_else(no_project)?;
        crate::project::require_folder(p)?;
        p.require_writable()?;
        generate_job(p, input.job_id)?;
        let out = crate::project::store::pick_base(
            &core.ctx,
            install_id,
            p,
            input.job_id,
            input.candidate_id,
        )
        .map_err(|e| {
            if e.code == ErrorCode::UnknownId {
                CoreError::new(ErrorCode::NotFound, e.message).with_field("candidate_id")
            } else {
                e
            }
        })?;
        if let Ok(j) = crate::project::store::job(p, input.job_id) {
            crate::region_edit::runner::emit_state(&core.ctx, j, None);
        }
        Ok(out)
    }
}

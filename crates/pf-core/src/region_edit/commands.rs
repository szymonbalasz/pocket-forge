//! RGN core commands, slices 1–2 (spec §9). Re-exported from [`crate::commands`], which holds
//! the registry. Inputs are validated at the boundary; a failed command changes nothing.

use std::collections::BTreeMap;

use pf_provider::{Capabilities, Cost, JobRequest, Operation, Provider};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use super::controls::{self, ControlsEvaluation};
use super::estimate::estimate_id;
use super::fit::{self, DEFAULT_MARGIN, MAX_MARGIN, RectIn, RegionFit};
use super::spend::{self, SpendView};
use super::{EVENT_SPEND, check_project, pixel_source, prepare};
use crate::commands::NoInput;
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::provider_commands::provider_call;
use crate::settings;

const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_CONTROLS: usize = 64;

fn check_model_id(field: &str, m: &str) -> CoreResult<()> {
    let m = m.trim();
    if m.is_empty() || m.len() > 200 || m.contains(['/', '\\', '\0']) {
        return Err(CoreError::invalid(field, "not a valid model id"));
    }
    Ok(())
}

fn check_rect(field: &str, r: &RectIn) -> CoreResult<()> {
    if !r.is_finite() {
        return Err(CoreError::invalid(field, "the rect must be finite numbers"));
    }
    Ok(())
}

fn check_margin(m: u32) -> CoreResult<()> {
    if m > MAX_MARGIN {
        return Err(CoreError::invalid(
            "margin",
            format!("the context margin is 0 to {MAX_MARGIN} px"),
        ));
    }
    Ok(())
}

fn check_controls_map(field: &str, c: &BTreeMap<String, Value>) -> CoreResult<()> {
    if c.len() > MAX_CONTROLS || c.keys().any(|k| k.is_empty() || k.len() > 200) {
        return Err(CoreError::invalid(
            field,
            "not a valid set of control values",
        ));
    }
    Ok(())
}

/// The model's capabilities through PRV; a model the provider doesn't know → `unknown_model`.
pub fn capabilities(core: &Core, model_id: &str) -> CoreResult<Capabilities> {
    provider_call(core, core.ctx.provider.get_capabilities(model_id.trim())).map_err(|e| {
        let p = &e.detail["provider"];
        if e.code == ErrorCode::Provider
            && p["code"] == "invalid_request"
            && p["field"] == "model_id"
        {
            CoreError::new(
                ErrorCode::UnknownModel,
                format!("unknown model {model_id:?}"),
            )
            .with_field("model_id")
        } else {
            e
        }
    })
}

// ---------------------------------------------------------------- rgn_region_fit

/// `rgn_region_fit` (FR-RGN-01/02/05): validates EDT's `{inner}` and fits it to the model.
pub struct RgnRegionFit;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionFitInput {
    pub project_id: Ulid,
    pub inner: RectIn,
    #[serde(default)]
    pub margin: Option<u32>,
    /// Without a model the outer rect is returned unfitted.
    #[serde(default)]
    pub model_id: Option<String>,
}

impl Validate for RegionFitInput {
    fn validate(&self) -> CoreResult<()> {
        check_rect("inner", &self.inner)?;
        check_margin(self.margin.unwrap_or(DEFAULT_MARGIN))?;
        if let Some(m) = &self.model_id {
            check_model_id("model_id", m)?;
        }
        Ok(())
    }
}

impl Command for RgnRegionFit {
    const NAME: &'static str = "rgn_region_fit";
    type Input = RegionFitInput;
    type Output = RegionFit;

    fn run(core: &Core, input: RegionFitInput) -> CoreResult<RegionFit> {
        let src = pixel_source(&core.state())?;
        check_project(&*src, Some(input.project_id))?;
        let px = match &input.model_id {
            Some(m) => Some(capabilities(core, m)?.input_px),
            None => None,
        };
        let (w, h) = src.image_size();
        fit::region_fit(
            input.inner,
            input.margin.unwrap_or(DEFAULT_MARGIN),
            w,
            h,
            px,
        )
    }
}

// ---------------------------------------------------------------- rgn_controls_evaluate

/// `rgn_controls_evaluate` (FR-RGN-08/11/33): the controls the panel shows for these values.
pub struct RgnControlsEvaluate;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlsEvaluateInput {
    pub model_id: String,
    /// Control name → value as the user set it (the prompt under its control name too).
    #[serde(default)]
    pub values: BTreeMap<String, Value>,
}

impl Validate for ControlsEvaluateInput {
    fn validate(&self) -> CoreResult<()> {
        check_model_id("model_id", &self.model_id)?;
        check_controls_map("values", &self.values)
    }
}

impl Command for RgnControlsEvaluate {
    const NAME: &'static str = "rgn_controls_evaluate";
    type Input = ControlsEvaluateInput;
    type Output = ControlsEvaluation;

    fn run(core: &Core, input: ControlsEvaluateInput) -> CoreResult<ControlsEvaluation> {
        let caps = capabilities(core, &input.model_id)?;
        Ok(controls::evaluate_controls(&caps, &input.values))
    }
}

// ---------------------------------------------------------------- rgn_job_estimate

/// The draft's region: EDT's inner rect and the context margin.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftRegion {
    pub inner: RectIn,
    #[serde(default = "default_margin")]
    pub margin: u32,
}

fn default_margin() -> u32 {
    DEFAULT_MARGIN
}

/// The job draft (spec §9): what estimate and submit act on.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JobDraft {
    pub region: DraftRegion,
    pub mask_id: Ulid,
    pub model_id: String,
    #[serde(default)]
    pub final_prompt: String,
    /// Control name → value, as the user set them (FR-RGN-34).
    #[serde(default)]
    pub controls: BTreeMap<String, Value>,
    /// Pinned seed, or `null` = random (FR-RGN-10).
    #[serde(default)]
    pub seed: Option<u64>,
}

impl Validate for JobDraft {
    fn validate(&self) -> CoreResult<()> {
        check_rect("draft.region.inner", &self.region.inner)?;
        check_margin(self.region.margin)?;
        check_model_id("draft.model_id", &self.model_id)?;
        if self.final_prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(CoreError::invalid(
                "draft.final_prompt",
                "the prompt is too long",
            ));
        }
        check_controls_map("draft.controls", &self.controls)
    }
}

/// Everything a job would send, after every local check (shared by estimate and submit).
#[derive(Debug, Clone)]
pub struct PreparedJob {
    pub caps: Capabilities,
    pub fit: RegionFit,
    pub sent: controls::SentControls,
    pub images: prepare::SentImages,
    pub estimate_id: String,
}

impl PreparedJob {
    pub fn request(&self, model_id: &str, final_prompt: &str) -> JobRequest {
        JobRequest {
            operation: Operation::EditMasked,
            model_id: model_id.trim().to_owned(),
            final_prompt: final_prompt.to_owned(),
            controls: self.sent.controls.clone(),
            references: vec![],
            region_png: self.images.region_png.clone(),
            mask_png: self.images.mask_png.clone(),
            candidate_count: self.sent.candidate_count,
            seed: self.sent.seed,
        }
    }
}

/// The local half of estimate and submit: read-only check, capabilities, fit, the controls
/// boundary check (FR-RGN-33), mask pipeline, extraction and the limit pre-check (FR-RGN-06).
/// No provider call except `get_capabilities`. Runs without the state lock.
pub fn prepare_job(
    core: &Core,
    project_id: Option<Ulid>,
    draft: &JobDraft,
    for_submit: bool,
) -> CoreResult<PreparedJob> {
    let src = pixel_source(&core.state())?;
    check_project(&*src, project_id)?;
    if src.read_only() {
        return Err(CoreError::new(
            ErrorCode::ReadOnly,
            "this asset project is read-only: no estimates or jobs",
        ));
    }
    let caps = capabilities(core, &draft.model_id)?;
    let (w, h) = src.image_size();
    let fit = fit::region_fit(
        draft.region.inner,
        draft.region.margin,
        w,
        h,
        Some(caps.input_px),
    )?;
    let sent = controls::check_controls(
        &caps,
        &draft.controls,
        &draft.final_prompt,
        draft.seed,
        for_submit,
    )?;
    let mask = src.mask(draft.mask_id)?;
    let images = prepare::prepare(&*src, &fit, &mask, &caps)?;
    let estimate_id = estimate_id(draft.model_id.trim(), &fit, &sent.cost_inputs);
    Ok(PreparedJob {
        caps,
        fit,
        sent,
        images,
        estimate_id,
    })
}

/// `rgn_job_estimate` (FR-RGN-12/13/14/15/16).
pub struct RgnJobEstimate;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobEstimateInput {
    #[serde(default)]
    pub project_id: Option<Ulid>,
    pub draft: JobDraft,
}

impl Validate for JobEstimateInput {
    fn validate(&self) -> CoreResult<()> {
        self.draft.validate()
    }
}

#[derive(Debug, Serialize)]
pub struct JobEstimateOutput {
    pub estimate_id: String,
    /// `{amount: number | "unknown", unit}`.
    pub cost: Cost,
    pub fit: RegionFit,
    pub candidate_count: u32,
    /// `spend`, `cap`, `threshold`, `needs[threshold|unknown_cost|set_cap]`, `over_cap`.
    #[serde(flatten)]
    pub spend: SpendView,
}

impl Command for RgnJobEstimate {
    const NAME: &'static str = "rgn_job_estimate";
    type Input = JobEstimateInput;
    type Output = JobEstimateOutput;

    fn run(core: &Core, input: JobEstimateInput) -> CoreResult<JobEstimateOutput> {
        let job = prepare_job(core, input.project_id, &input.draft, false)?;
        let now = core.ctx.now();
        let cached = core.state().rgn.estimates.get(&job.estimate_id, now);
        let cost = match cached {
            Some(c) => c,
            None => {
                let request = job.request(&input.draft.model_id, &input.draft.final_prompt);
                let request = if request.final_prompt.is_empty() {
                    JobRequest {
                        final_prompt: controls::ESTIMATE_PROMPT.to_owned(),
                        ..request
                    }
                } else {
                    request
                };
                let est = provider_call(core, core.ctx.provider.estimate(&request))?;
                core.state()
                    .rgn
                    .estimates
                    .put(job.estimate_id.clone(), est.cost.clone(), now);
                est.cost
            }
        };
        let state = core.state();
        let view = spend::assess(state.rgn.ledger.total(), &state.settings.spend, cost.amount);
        Ok(JobEstimateOutput {
            estimate_id: job.estimate_id,
            cost,
            fit: job.fit,
            candidate_count: job.sent.candidate_count,
            spend: view,
        })
    }
}

// ---------------------------------------------------------------- rgn_spend_get / _set

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpendOutput {
    /// Session spend in provider units (since app start).
    pub spend: f64,
    pub cap: Option<f64>,
    pub threshold: Option<f64>,
}

fn spend_output(state: &crate::core::State) -> SpendOutput {
    SpendOutput {
        spend: state.rgn.ledger.total(),
        cap: state.settings.spend.cap,
        threshold: state.settings.spend.threshold,
    }
}

/// `rgn_spend_get`.
pub struct RgnSpendGet;

impl Command for RgnSpendGet {
    const NAME: &'static str = "rgn_spend_get";
    type Input = NoInput;
    type Output = SpendOutput;

    fn run(core: &Core, _: NoInput) -> CoreResult<SpendOutput> {
        Ok(spend_output(&core.state()))
    }
}

/// `rgn_spend_set` (FR-RGN-14/16): stores the cap and threshold in `settings.json` `spend` (R8).
pub struct RgnSpendSet;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpendSetInput {
    /// Missing = unchanged, `null` = not set.
    #[serde(default, deserialize_with = "some_value")]
    pub cap: Option<Option<f64>>,
    #[serde(default, deserialize_with = "some_value")]
    pub threshold: Option<Option<f64>>,
}

/// Tells a missing field (`None`) from an explicit `null` (`Some(None)`).
fn some_value<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<f64>>, D::Error> {
    Option::<f64>::deserialize(d).map(Some)
}

impl Validate for SpendSetInput {
    fn validate(&self) -> CoreResult<()> {
        spend::check_amount("cap", self.cap.flatten())?;
        spend::check_amount("threshold", self.threshold.flatten())
    }
}

impl Command for RgnSpendSet {
    const NAME: &'static str = "rgn_spend_set";
    type Input = SpendSetInput;
    type Output = SpendOutput;

    fn run(core: &Core, input: SpendSetInput) -> CoreResult<SpendOutput> {
        let mut state = core.state();
        let mut next = state.settings.clone();
        if let Some(cap) = input.cap {
            next.spend.cap = cap;
        }
        if let Some(threshold) = input.threshold {
            next.spend.threshold = threshold;
        }
        settings::save(&core.ctx, &next)?;
        state.settings = next;
        let out = spend_output(&state);
        core.ctx.emit(
            EVENT_SPEND,
            serde_json::json!({ "spend": out.spend, "cap": out.cap }),
        );
        Ok(out)
    }
}

//! RGN core commands, slice 1 (spec §9). Re-exported from [`crate::commands`], which holds
//! the registry. Inputs are validated at the boundary; a failed command changes nothing.

use pf_provider::{Capabilities, Provider};
use serde::Deserialize;
use ulid::Ulid;

use super::fit::{self, DEFAULT_MARGIN, MAX_MARGIN, RectIn, RegionFit};
use super::{check_project, pixel_source};
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::provider_commands::provider_call;

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

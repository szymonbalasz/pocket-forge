//! Capability-driven controls (FR-RGN-08/09/10/11/33/34, Appendix A, #37). Rules act only
//! through PRV's pure `Capabilities::evaluate` (FR-PRV-30); the source, mask and prompt are bound
//! by `role`, never by input name (R28).

use std::collections::BTreeMap;

use pf_provider::{
    Capabilities, ControlError, ControlErrorCode, ControlRole, ControlType, EffectiveControl,
};
use serde::Serialize;
use serde_json::Value;

use crate::error::{CoreError, CoreResult, ErrorCode};

/// One control as the panel renders it (`rgn_controls_evaluate`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ControlState {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ControlType,
    pub role: ControlRole,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub default: Option<Value>,
    pub cost_affecting: bool,
    /// False when a rule hides it: neither shown nor sent (FR-RGN-33).
    pub visible: bool,
    /// The effective allowed values (empty = any value in range).
    pub allowed: Vec<Value>,
    pub required: bool,
    /// The value to show: the user's, or the default after a reset; `null` = unset.
    pub value: Option<Value>,
    /// The user's value was rejected and reset to the default; the panel shows a notice.
    pub reset: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ControlError>,
}

/// `rgn_controls_evaluate` result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ControlsEvaluation {
    pub model_id: String,
    /// Every capability control except the image inputs (source, mask, reference: never shown,
    /// Appendix A), in capability order.
    pub controls: Vec<ControlState>,
    /// Required controls without a value; submit is disabled, naming them (FR-RGN-33).
    pub required_missing: Vec<String>,
    /// Given names this model has no control for (a remembered value from another model,
    /// FR-RGN-11); dropped with a notice.
    pub unknown: Vec<String>,
    /// The capability `warnings[]` for the panel warning (FR-RGN-08).
    pub warnings: Vec<String>,
    pub prompt_required: bool,
    pub seed: bool,
}

/// The image roles: filled with the region and mask PNGs or not sent in tier S (Appendix A).
fn is_image_role(role: ControlRole) -> bool {
    matches!(
        role,
        ControlRole::Source | ControlRole::Mask | ControlRole::Reference
    )
}

/// Codes after which a remembered value resets to the default (FR-RGN-11, FR-RGN-33).
fn resets(code: ControlErrorCode) -> bool {
    matches!(
        code,
        ControlErrorCode::WrongType
            | ControlErrorCode::OutOfRange
            | ControlErrorCode::Step
            | ControlErrorCode::NotAllowed
    )
}

/// Loose equality, as PRV's `evaluate` compares: numbers by value (`2` == `2.0`).
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

fn required_error(name: &str) -> ControlError {
    ControlError {
        control: name.to_owned(),
        code: ControlErrorCode::Required,
        message: "required".into(),
    }
}

/// The prompt-role control, if the model has one.
pub fn prompt_control(caps: &Capabilities) -> Option<&pf_provider::Control> {
    caps.controls.iter().find(|c| c.role == ControlRole::Prompt)
}

/// FR-RGN-33 / FR-RGN-11: evaluates `values` (control name → value, as the user set them) and
/// resets rejected values to their defaults, then evaluates again.
pub fn evaluate_controls(
    caps: &Capabilities,
    values: &BTreeMap<String, Value>,
) -> ControlsEvaluation {
    let mut values: BTreeMap<String, Value> = values
        .iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let first = caps.evaluate(&values);
    let unknown: Vec<String> = first
        .errors
        .iter()
        .filter(|e| e.code == ControlErrorCode::UnknownControl)
        .map(|e| e.control.clone())
        .collect();
    let mut reset = vec![];
    for e in first.errors.iter().filter(|e| resets(e.code)) {
        values.remove(&e.control);
        reset.push(e.control.clone());
    }
    for name in &unknown {
        values.remove(name);
    }
    let eval = caps.evaluate(&values);
    let visible: BTreeMap<&str, &EffectiveControl> =
        eval.controls.iter().map(|c| (c.name.as_str(), c)).collect();

    let mut controls = vec![];
    let mut required_missing = vec![];
    for c in caps.controls.iter().filter(|c| !is_image_role(c.role)) {
        let eff = visible.get(c.name.as_str()).copied();
        let mut value = values.get(&c.name).cloned().or_else(|| c.default.clone());
        let mut was_reset = reset.contains(&c.name);
        // A default outside the effective allowed set falls back to the first allowed value.
        if let (Some(e), Some(v)) = (eff, &value)
            && !e.allowed.is_empty()
            && !e.allowed.iter().any(|a| same(a, v))
        {
            value = e.allowed.first().cloned();
            was_reset = true;
        }
        let mut error = eval.errors.iter().find(|e| e.control == c.name).cloned();
        let required = eff.is_some_and(|e| e.required)
            || (c.role == ControlRole::Prompt && caps.prompt_required);
        // An empty prompt counts as missing when one is required (FR-RGN-09).
        let empty_prompt = c.role == ControlRole::Prompt
            && value
                .as_ref()
                .is_none_or(|v| v.as_str().is_some_and(|s| s.trim().is_empty()));
        if eff.is_some() && required && empty_prompt && error.is_none() {
            error = Some(required_error(&c.name));
        }
        if eff.is_some()
            && error
                .as_ref()
                .is_some_and(|e| e.code == ControlErrorCode::Required)
        {
            required_missing.push(c.name.clone());
        }
        controls.push(ControlState {
            name: c.name.clone(),
            kind: c.kind,
            role: c.role,
            min: c.min,
            max: c.max,
            step: c.step,
            default: c.default.clone(),
            cost_affecting: c.cost_affecting,
            visible: eff.is_some(),
            allowed: eff.map_or_else(|| c.allowed.clone(), |e| e.allowed.clone()),
            required,
            value: eff.and(value),
            reset: was_reset,
            error: eff.and(error),
        });
    }
    ControlsEvaluation {
        model_id: caps.model_id.clone(),
        controls,
        required_missing,
        unknown,
        warnings: caps.warnings.clone(),
        prompt_required: caps.prompt_required,
        seed: caps.seed,
    }
}

fn invalid_control(name: &str, reason: &str, message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InvalidControl, message)
        .with_field(format!("controls.{name}"))
        .with_detail(serde_json::json!({ "name": name, "reason": reason }))
}

fn reason_of(code: ControlErrorCode) -> &'static str {
    match code {
        ControlErrorCode::Required => "required",
        ControlErrorCode::WrongType => "wrong_type",
        ControlErrorCode::OutOfRange => "out_of_range",
        ControlErrorCode::Step => "step",
        ControlErrorCode::NotAllowed => "not_allowed",
        ControlErrorCode::UnknownControl => "unknown_control",
    }
}

/// What a job sends, after the boundary check (FR-RGN-33/34).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SentControls {
    /// Visible `other`- (and `background`-) role controls that are required or were set by the user; untouched
    /// optional ones are omitted so the provider applies its default (FR-RGN-34).
    pub controls: BTreeMap<String, Value>,
    pub candidate_count: u32,
    /// The pinned seed, or `None` = random (FR-RGN-10).
    pub seed: Option<u64>,
    /// Cost-affecting values (by control name) that go into the estimate id (FR-RGN-13).
    pub cost_inputs: BTreeMap<String, Value>,
}

/// The boundary check of estimate and submit (FR-RGN-33): `controls` are the values the user
/// set, keyed by control name; the prompt and seed come from the draft's own fields.
/// `require_prompt` is false for estimates (the prompt is not a cost input, FR-RGN-12).
pub fn check_controls(
    caps: &Capabilities,
    controls: &BTreeMap<String, Value>,
    final_prompt: &str,
    seed: Option<u64>,
    require_prompt: bool,
) -> CoreResult<SentControls> {
    let mut values: BTreeMap<String, Value> = BTreeMap::new();
    for (name, v) in controls.iter().filter(|(_, v)| !v.is_null()) {
        match caps.controls.iter().find(|c| &c.name == name) {
            None => {
                return Err(invalid_control(
                    name,
                    "unknown_control",
                    format!("{} has no control {name:?}", caps.model_id),
                ));
            }
            Some(c)
                if is_image_role(c.role)
                    || matches!(c.role, ControlRole::Prompt | ControlRole::Seed) =>
            {
                return Err(invalid_control(
                    name,
                    "bound_by_role",
                    format!("{name:?} is filled from the draft, not from controls"),
                ));
            }
            Some(_) => {
                values.insert(name.clone(), v.clone());
            }
        }
    }
    let prompt = prompt_control(caps);
    if let Some(p) = prompt {
        // Estimates don't depend on the prompt; a placeholder keeps a required prompt
        // satisfied there. Submit sends the typed prompt byte for byte (FR-RGN-09).
        let text = if final_prompt.is_empty() && !require_prompt {
            ESTIMATE_PROMPT
        } else {
            final_prompt
        };
        values.insert(p.name.clone(), Value::String(text.to_owned()));
    }
    let seed_control = caps.controls.iter().find(|c| c.role == ControlRole::Seed);
    let seed = match (seed, caps.seed) {
        (Some(_), false) => {
            return Err(invalid_control(
                "seed",
                "unknown_control",
                format!("{} has no seed", caps.model_id),
            ));
        }
        (s, _) => s,
    };
    if let (Some(s), Some(c)) = (seed, seed_control) {
        values.insert(c.name.clone(), Value::from(s));
    }
    if require_prompt
        && (caps.prompt_required || prompt.is_some_and(|p| p.required))
        && final_prompt.trim().is_empty()
    {
        let name = prompt.map_or("prompt", |p| p.name.as_str());
        return Err(invalid_control(name, "required", "type a prompt first"));
    }

    let eval = caps.evaluate(&values);
    if let Some(e) = eval.errors.first() {
        return Err(invalid_control(
            &e.control,
            reason_of(e.code),
            format!("{}: {}", e.control, e.message),
        ));
    }
    let mut sent = BTreeMap::new();
    let mut cost_inputs = BTreeMap::new();
    let mut candidate_count = 1u32;
    for c in &eval.controls {
        let user = values
            .get(&c.name)
            .filter(|_| controls.contains_key(&c.name));
        let effective = user.cloned().or_else(|| c.default.clone());
        if c.role == ControlRole::CandidateCount
            && let Some(n) = effective.as_ref().and_then(Value::as_u64)
        {
            candidate_count = u32::try_from(n).unwrap_or(u32::MAX);
        }
        if c.cost_affecting
            && let Some(v) = &effective
        {
            cost_inputs.insert(c.name.clone(), v.clone());
        }
        // `background` (GEN, OQ-GEN-07) is an ordinary control (FR-GEN-12).
        if matches!(c.role, ControlRole::Other | ControlRole::Background)
            && let Some(v) = (if c.required { effective } else { user.cloned() })
        {
            sent.insert(c.name.clone(), v);
        }
    }
    Ok(SentControls {
        controls: sent,
        candidate_count,
        seed,
        cost_inputs,
    })
}

/// The prompt an estimate sends while the prompt field is still empty (pick in the build
/// report: cost doesn't depend on the prompt, and providers reject an empty one).
pub const ESTIMATE_PROMPT: &str = "estimate";

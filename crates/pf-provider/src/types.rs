//! Generic provider shapes (ADR 0001, provider spec Appendix C). No Scenario concepts here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A provider operation (ADR 0001 "Job request").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Generate,
    EditMasked,
    Extend,
    Upscale,
    RemoveBackground,
    Tile,
}

/// `list_models` filter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelFilter {
    pub operation: Option<Operation>,
}

/// One entry of `list_models`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSummary {
    pub id: String,
    pub name: String,
    pub operations: Vec<Operation>,
}

/// Generic control types (FR-PRV-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlType {
    Int,
    Float,
    Enum,
    Bool,
    String,
    Seed,
    Image,
    Mask,
}

/// What a control is bound to (R28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlRole {
    Source,
    Mask,
    Prompt,
    CandidateCount,
    Seed,
    Reference,
    /// The output background (`opaque` / `transparent`, GEN OQ-GEN-07).
    Background,
    Other,
}

/// One model control (Appendix C).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Control {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ControlType,
    pub role: ControlRole,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub allowed: Vec<Value>,
    pub default: Option<Value>,
    pub required: bool,
    pub cost_affecting: bool,
}

/// A rule condition (`if`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleCondition {
    pub control: String,
    pub op: RuleOp,
    pub value: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleOp {
    Eq,
    In,
}

/// A rule effect (`then`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleEffect {
    pub control: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
}

/// A conditional rule between controls (Appendix C `rules[]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    #[serde(rename = "if")]
    pub when: RuleCondition,
    pub then: RuleEffect,
}

/// How a model reads a mask (glossary: mask convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskConvention {
    WhiteEdit,
    BlackEdit,
    AlphaTransparentEdit,
}

/// How soft mask values are passed on (FR-PRV-12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "rule", content = "level")]
pub enum SoftRule {
    PassThrough,
    Threshold(u8),
}

/// Mask support of a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskSupport {
    pub convention: MaskConvention,
    pub soft: SoftRule,
}

/// Accepted sent-image size (R20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputPx {
    pub min: u32,
    pub max: u32,
    pub multiple_of: u32,
}

/// What a reference image may influence (glossary: reference role).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceRole {
    Style,
    Structure,
    Identity,
}

/// A reference image input of a model (FR-PRV-31), derived from a control with
/// `role: reference` (R31).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    /// The control (input) name; `ReferenceImage::slot` names it.
    pub name: String,
    /// From the overlay row's `slot_roles`; `None` when the model doesn't say.
    pub role: Option<ReferenceRole>,
    /// 1 if the input is required, else 0. Meeting it is the caller's job (FR-PRV-32).
    pub min: u32,
    /// `file` → 1; `file_array` → the input's `max`, else the overlay's `slot_max`, else 1.
    pub max: u32,
    /// From `costImpact`: references in this slot change the estimate (FR-REF-24).
    pub cost_affecting: bool,
    /// The input takes a list (`file_array`); otherwise one file. The body sends an array or
    /// one ref accordingly (FR-PRV-33).
    #[serde(default)]
    pub array: bool,
}

/// `list_models` plus whether it came from the local cache while offline (FR-PRV-25).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelList {
    pub models: Vec<ModelSummary>,
    pub stale: bool,
}

/// Normalised model capabilities (Appendix C).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub model_id: String,
    pub operations: Vec<Operation>,
    pub controls: Vec<Control>,
    pub rules: Vec<Rule>,
    pub mask: Option<MaskSupport>,
    pub input_px: InputPx,
    pub max_bytes: u64,
    pub seed: bool,
    pub prompt_required: bool,
    pub slots: Vec<Slot>,
    pub warnings: Vec<String>,
    /// `generate` only: the output size the model takes (GEN FR-GEN-07–11). Its inputs are
    /// never controls.
    #[serde(default)]
    pub output_size: Option<OutputSize>,
    /// `generate` only: the slot that takes the structural sketch, sent first (FR-GEN-14/15).
    /// Null on every real model until a paid check confirms one (OQ-GEN-05).
    #[serde(default)]
    pub structure: Option<StructureSlot>,
}

/// The output size a `generate` model takes: exact width × height, or one of its ratios.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum OutputSize {
    /// Each side within `min`–`max`, a multiple of `step`; with `min_pixels`, width × height
    /// is at least that many px (a minimum area the model's schema doesn't publish; Seedream).
    Wh {
        min: u32,
        max: u32,
        step: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_pixels: Option<u64>,
    },
    /// One of these `w:h` ratios (non-ratio values such as `auto` are left out).
    Aspect { allowed: Vec<String> },
}

/// The reference slot a structural sketch goes to (FR-GEN-15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructureSlot {
    pub slot: String,
}

/// The output size a `generate` request asks for (matches the model's [`OutputSize`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum OutputRequest {
    Wh { w: u32, h: u32 },
    Aspect { aspect: String },
}

/// Purpose of an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadPurpose {
    Source,
    Mask,
    Reference,
}

/// The provider's id for an uploaded image (glossary: provider asset ref).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AssetRef(pub String);

/// A reference image sent with a job (FR-PRV-33), prepared by REF (FR-REF-21/22).
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceImage {
    /// The [`Slot::name`] it goes to.
    pub slot: String,
    /// The role the user picked, recorded only; the slot decides the input.
    pub role: Option<ReferenceRole>,
    /// The bytes as sent: PNG, JPEG or WebP.
    pub bytes: Vec<u8>,
    /// SHA-256 of `bytes`: `sha256:<hex>` (pf-core's form) or bare hex. A mismatch is
    /// `invalid_request(reference)`.
    pub hash: String,
}

/// A generic job request (ADR 0001).
///
/// `region_png` is the outer rect as sent (after fit); `mask_png` is the canonical mask
/// (8-bit grey, white = editable) at exactly the same size (FR-PRV-12). A `generate` request
/// sends neither (both empty) and sets `output` instead.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRequest {
    pub operation: Operation,
    pub model_id: String,
    pub final_prompt: String,
    pub controls: BTreeMap<String, Value>,
    /// In request order; each slot's refs go to its input in this order (FR-PRV-33).
    pub references: Vec<ReferenceImage>,
    pub region_png: Vec<u8>,
    pub mask_png: Vec<u8>,
    pub candidate_count: u32,
    pub seed: Option<u64>,
    /// `generate` only: the output size asked for (`invalid_request(output)` if it doesn't
    /// match the model's `output_size`).
    pub output: Option<OutputRequest>,
}

/// A cost amount: a number of provider units, or unknown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CostAmount {
    Known(f64),
    Unknown,
}

impl Serialize for CostAmount {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            CostAmount::Known(v) => s.serialize_f64(*v),
            CostAmount::Unknown => s.serialize_str("unknown"),
        }
    }
}

impl<'de> Deserialize<'de> for CostAmount {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            Value::Number(n) => n
                .as_f64()
                .map(CostAmount::Known)
                .ok_or_else(|| serde::de::Error::custom("cost amount out of range")),
            Value::String(s) if s == "unknown" => Ok(CostAmount::Unknown),
            other => Err(serde::de::Error::custom(format!(
                "cost amount must be a number or \"unknown\", not {other}"
            ))),
        }
    }
}

/// The provider-reported charge of a job (glossary: cost).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    pub amount: CostAmount,
    pub unit: String,
}

/// The opaque `provider{}` object persisted verbatim on the job record (Appendix B, FR-PRV-26).
/// Only `pf-provider` reads its fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderRecord(pub Value);

/// True if a persisted `provider{}` object carries a job handle (`submit` returned; R102). The
/// one fact about the record the core may ask for: whether polling can go on without a new
/// submit.
pub fn has_job_handle(provider: &Value) -> bool {
    provider.get("job_handle").is_some_and(|h| !h.is_null())
}

/// Result of `estimate`.
#[derive(Debug, Clone, PartialEq)]
pub struct Estimate {
    pub cost: Cost,
    pub provider: ProviderRecord,
}

/// Generic job state from `status` (ADR 0001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderJobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Unknown,
}

/// Result of `status`.
#[derive(Debug, Clone, PartialEq)]
pub struct JobStatus {
    pub state: ProviderJobState,
    /// 0–1.
    pub progress: f64,
    pub cost: Option<Cost>,
    /// Set when `state` is `failed`.
    pub error: Option<crate::ProviderError>,
    /// The updated record to persist.
    pub provider: ProviderRecord,
}

/// Result of `cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelAck {
    Canceled,
    NotCancellable,
}

/// One candidate image, untouched as returned.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateImage {
    pub bytes: Vec<u8>,
    pub seed: Option<u64>,
    /// Opaque provider metadata (asset id, seed, inputs).
    pub provider_meta: Value,
}

/// Result of `fetch_candidates`.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidates {
    pub images: Vec<CandidateImage>,
    pub provider: ProviderRecord,
}

/// Result of `balance`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Balance {
    Known {
        remaining: f64,
        unit: String,
        period_end: Option<String>,
    },
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_gen_08_wh_min_pixels_is_add_only() {
        // Capabilities serialised before `min_pixels` existed still load (no minimum).
        let old: OutputSize =
            serde_json::from_str(r#"{"mode":"wh","min":1024,"max":6240,"step":16}"#).unwrap();
        assert_eq!(
            old,
            OutputSize::Wh {
                min: 1024,
                max: 6240,
                step: 16,
                min_pixels: None
            }
        );
        // No minimum → the field is left out, so older readers see the same JSON.
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            r#"{"mode":"wh","min":1024,"max":6240,"step":16}"#
        );
        let seedream = OutputSize::Wh {
            min: 1024,
            max: 6240,
            step: 16,
            min_pixels: Some(3_981_312),
        };
        let text = serde_json::to_string(&seedream).unwrap();
        assert!(text.contains(r#""min_pixels":3981312"#), "{text}");
        assert_eq!(serde_json::from_str::<OutputSize>(&text).unwrap(), seedream);
    }

    #[test]
    fn cost_amount_serialises_as_number_or_unknown() {
        let known = Cost {
            amount: CostAmount::Known(12.5),
            unit: "CU".into(),
        };
        let unknown = Cost {
            amount: CostAmount::Unknown,
            unit: "CU".into(),
        };
        assert_eq!(
            serde_json::to_string(&known).unwrap(),
            r#"{"amount":12.5,"unit":"CU"}"#
        );
        assert_eq!(
            serde_json::to_string(&unknown).unwrap(),
            r#"{"amount":"unknown","unit":"CU"}"#
        );
        let back: Cost = serde_json::from_str(r#"{"amount":"unknown","unit":"CU"}"#).unwrap();
        assert_eq!(back, unknown);
        assert!(serde_json::from_str::<Cost>(r#"{"amount":"lots","unit":"CU"}"#).is_err());
    }
}

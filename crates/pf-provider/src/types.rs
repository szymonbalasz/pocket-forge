//! Generic provider shapes (ADR 0001, provider spec Appendix C). No Scenario concepts here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A provider operation (ADR 0001 "Job request").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// An extra image input of a model (none are sent in tier S).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub name: String,
    pub role: Option<ReferenceRole>,
    pub required: bool,
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

/// A reference image sent with a job.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceImage {
    pub role: ReferenceRole,
    pub png: Vec<u8>,
}

/// A generic job request (ADR 0001).
///
/// `region_png` is the outer rect as sent (after fit); `mask_png` is the canonical mask
/// (8-bit grey, white = editable) at exactly the same size (FR-PRV-12).
#[derive(Debug, Clone, PartialEq)]
pub struct JobRequest {
    pub operation: Operation,
    pub model_id: String,
    pub final_prompt: String,
    pub controls: BTreeMap<String, Value>,
    pub references: Vec<ReferenceImage>,
    pub region_png: Vec<u8>,
    pub mask_png: Vec<u8>,
    pub candidate_count: u32,
    pub seed: Option<u64>,
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

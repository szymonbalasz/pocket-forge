//! The model overlay (#15, OQ-PRV-18: a static Rust table): what Scenario's capabilities don't
//! say. Each row gives a model its mask convention and soft rule, the input name per role
//! where Scenario doesn't mark it (R28), the accepted sent-image size, whether a dry run needs
//! the uploaded assets, and where it was checked. Masked edit is offered only for a model with
//! a row **and** a mask input in its capabilities (FR-PRV-11).
//!
//! Model ids, `dry_run_assets` and the `soft` picks are confirmed by free `GET /models/{id}`
//! and dry-run recordings (PRV slice 5, owner); until then they follow the research doc.

use crate::{ControlRole, InputPx, MaskConvention, SoftRule};

/// One overlay row (spec Appendix C).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayRow {
    pub model_id: &'static str,
    pub convention: MaskConvention,
    pub soft: SoftRule,
    /// Input name → role, for roles Scenario doesn't mark (source, mask, prompt, count).
    pub roles: &'static [(&'static str, ControlRole)],
    pub input_px: InputPx,
    /// Largest converted file per image (bytes).
    pub max_bytes: u64,
    /// Does a dry run need the uploaded image assets (R15)? `true` until a recorded free dry
    /// run shows otherwise: estimate then uploads, and FR-PRV-14 dedupes for submit.
    pub dry_run_assets: bool,
    pub doc_url: &'static str,
    pub checked: &'static str,
}

const MB: u64 = 1024 * 1024;

const GPT_ROLES: &[(&str, ControlRole)] = &[
    ("image", ControlRole::Source),
    ("mask", ControlRole::Mask),
    ("prompt", ControlRole::Prompt),
    ("numOutputs", ControlRole::CandidateCount),
];

const IDEOGRAM_ROLES: &[(&str, ControlRole)] = &[
    ("image", ControlRole::Source),
    ("mask", ControlRole::Mask),
    ("prompt", ControlRole::Prompt),
    ("numOutputs", ControlRole::CandidateCount),
    ("seed", ControlRole::Seed),
];

const OPENAI_DOC: &str = "https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-openai";
const IDEOGRAM_DOC: &str = "https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-ideogram";
const CHECKED: &str = "2026-11-06";

/// GPT Image: PNG, transparent alpha = edit, same size as the image, ≤ 3840 px, ≤ 50 MB.
const fn gpt(model_id: &'static str) -> OverlayRow {
    OverlayRow {
        model_id,
        convention: MaskConvention::AlphaTransparentEdit,
        soft: SoftRule::PassThrough,
        roles: GPT_ROLES,
        input_px: InputPx {
            min: 256,
            max: 3840,
            multiple_of: 16,
        },
        max_bytes: 50 * MB,
        dry_run_assets: true,
        doc_url: OPENAI_DOC,
        checked: CHECKED,
    }
}

/// Ideogram: `white` = edit for 4.5 / 4.5 Precise Edit, `black` = inpaint for V3 / Character.
const fn ideogram(model_id: &'static str, convention: MaskConvention, max: u32) -> OverlayRow {
    OverlayRow {
        model_id,
        convention,
        soft: SoftRule::Threshold(128),
        roles: IDEOGRAM_ROLES,
        input_px: InputPx {
            min: 256,
            max,
            multiple_of: 8,
        },
        max_bytes: 50 * MB,
        dry_run_assets: true,
        doc_url: IDEOGRAM_DOC,
        checked: CHECKED,
    }
}

/// The Scenario rows (spec Appendix C).
pub static SCENARIO_ROWS: [OverlayRow; 6] = [
    gpt("model_openai-gpt-image-2"),
    gpt("model_openai-gpt-image-2-5"),
    ideogram("model_ideogram-v4-5", MaskConvention::WhiteEdit, 4096),
    // Precise Edit takes inputs up to 67 MP and keeps the input size (research doc).
    ideogram(
        "model_ideogram-v4-5-precise-edit",
        MaskConvention::WhiteEdit,
        8192,
    ),
    ideogram("model_ideogram-v3", MaskConvention::BlackEdit, 4096),
    ideogram("model_ideogram-character", MaskConvention::BlackEdit, 4096),
];

const FAKE_ROLES: &[(&str, ControlRole)] = &[
    ("image", ControlRole::Source),
    ("mask", ControlRole::Mask),
    ("prompt", ControlRole::Prompt),
    ("num_samples", ControlRole::CandidateCount),
];

const fn fake(model_id: &'static str, convention: MaskConvention, soft: SoftRule) -> OverlayRow {
    OverlayRow {
        model_id,
        convention,
        soft,
        roles: FAKE_ROLES,
        input_px: InputPx {
            min: 64,
            max: 4096,
            multiple_of: 8,
        },
        max_bytes: 50 * MB,
        dry_run_assets: false,
        doc_url: "docs/specs/provider.md#appendix-d-fake-cases",
        checked: CHECKED,
    }
}

/// The fake provider's rows: one model per mask convention (FR-PRV-03).
pub static FAKE_ROWS: [OverlayRow; 3] = [
    fake(
        "fake-white-edit",
        MaskConvention::WhiteEdit,
        SoftRule::Threshold(128),
    ),
    fake(
        "fake-black-edit",
        MaskConvention::BlackEdit,
        SoftRule::Threshold(128),
    ),
    fake(
        "fake-alpha-edit",
        MaskConvention::AlphaTransparentEdit,
        SoftRule::PassThrough,
    ),
];

/// The row for a model, from a table.
pub fn row_in<'a>(rows: &'a [OverlayRow], model_id: &str) -> Option<&'a OverlayRow> {
    rows.iter().find(|r| r.model_id == model_id)
}

impl OverlayRow {
    /// The role the overlay gives an input name, if any.
    pub fn role_of(&self, input: &str) -> Option<ControlRole> {
        self.roles
            .iter()
            .find(|(n, _)| *n == input)
            .map(|(_, r)| *r)
    }
}

//! The model overlay (#15, OQ-PRV-18: a static Rust table): what Scenario's capabilities don't
//! say. Each row gives a model its mask convention and soft rule, the input name per role
//! where Scenario doesn't mark it (R28), the accepted sent-image size, whether a dry run needs
//! the uploaded assets, the role and fallback count of each reference slot (FR-PRV-31), the
//! inputs the adapter fills itself (fixed values, the sent size), and where it was checked.
//! Masked edit is offered only for a model with a row **and** a mask input in its
//! capabilities (FR-PRV-11).
//!
//! Model ids and input names follow Scenario's model docs (checked 2026-11-08). The owner's
//! free `get-model` and dry-run recordings (PRV slice 5) confirm them, plus `dry_run_assets`
//! and the `soft` picks.

use crate::{ControlRole, InputPx, MaskConvention, ReferenceRole, SoftRule};

/// One overlay row (spec Appendix C).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayRow {
    pub model_id: &'static str,
    pub convention: MaskConvention,
    pub soft: SoftRule,
    /// Input name → role, for roles Scenario doesn't mark (source, mask, prompt, count).
    /// A name the model doesn't list is skipped.
    pub roles: &'static [(&'static str, ControlRole)],
    pub input_px: InputPx,
    /// Largest converted file per image (bytes).
    pub max_bytes: u64,
    /// Does a dry run need the uploaded image assets (R15)? `true` until a recorded free dry
    /// run shows otherwise: estimate then uploads, and FR-PRV-14 dedupes for submit.
    pub dry_run_assets: bool,
    /// Reference input name → role (#51). An input not listed has no role.
    pub slot_roles: &'static [(&'static str, ReferenceRole)],
    /// Reference input name → max count, for a `file_array` input that lists no `max`.
    pub slot_max: &'static [(&'static str, u32)],
    /// Inputs always sent with this value and never shown as controls (a masked edit needs
    /// it, e.g. Ideogram 4.5's `aspectRatio: "auto"`).
    pub fixed: &'static [(&'static str, &'static str)],
    /// `(width, height)` inputs set to the sent image's size and never shown as controls, so
    /// the candidate comes back at the sent size (GPT Image).
    pub size_inputs: Option<(&'static str, &'static str)>,
    pub doc_url: &'static str,
    pub checked: &'static str,
}

const MB: u64 = 1024 * 1024;

/// GPT Image takes the source as the first of `referenceImages`; the mask applies to it.
const GPT_ROLES: &[(&str, ControlRole)] = &[
    ("referenceImages", ControlRole::Source),
    ("mask", ControlRole::Mask),
    ("prompt", ControlRole::Prompt),
    ("numOutputs", ControlRole::CandidateCount),
];

/// Ideogram 4.5 also edits the first of `referenceImages`.
const IDEOGRAM_45_ROLES: &[(&str, ControlRole)] = &[
    ("referenceImages", ControlRole::Source),
    ("mask", ControlRole::Mask),
    ("prompt", ControlRole::Prompt),
    ("numOutputs", ControlRole::CandidateCount),
    ("seed", ControlRole::Seed),
];

/// Ideogram Precise Edit, V3 and Character take `image` + `mask`.
const IDEOGRAM_ROLES: &[(&str, ControlRole)] = &[
    ("image", ControlRole::Source),
    ("mask", ControlRole::Mask),
    ("prompt", ControlRole::Prompt),
    ("numOutputs", ControlRole::CandidateCount),
    ("seed", ControlRole::Seed),
];

/// Ideogram V3 / Character reference inputs (spec Appendix C). The docs name Character's
/// input `characterReferenceImage` (one file); both spellings map until it is recorded.
const IDEOGRAM_SLOTS: &[(&str, ReferenceRole)] = &[
    ("styleReferenceImages", ReferenceRole::Style),
    ("characterReferenceImage", ReferenceRole::Identity),
    ("characterReferenceImages", ReferenceRole::Identity),
];

/// Precise Edit's extra images are style references.
const PRECISE_SLOTS: &[(&str, ReferenceRole)] = &[("referenceImages", ReferenceRole::Style)];

const OPENAI_DOC: &str = "https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-openai";
const IDEOGRAM_DOC: &str = "https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-ideogram";
const CHECKED: &str = "2026-11-08";

/// GPT Image: PNG, transparent alpha = edit, same size as the first reference image,
/// 16–3840 px in steps of 16, ≤ 50 MB.
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
        // None recorded (Appendix C).
        slot_roles: &[],
        slot_max: &[],
        fixed: &[],
        size_inputs: Some(("width", "height")),
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
        slot_roles: IDEOGRAM_SLOTS,
        slot_max: &[],
        fixed: &[],
        size_inputs: None,
        doc_url: IDEOGRAM_DOC,
        checked: CHECKED,
    }
}

/// The Scenario rows (spec Appendix C).
pub static SCENARIO_ROWS: [OverlayRow; 9] = [
    gpt("model_openai-gpt-image-2"),
    gpt("model_openai-gpt-image-2-5-flare"),
    gpt("model_openai-gpt-image-2-5-sunburst"),
    // 4.5 edits the first reference image; a mask needs `aspectRatio: "auto"` (docs).
    OverlayRow {
        roles: IDEOGRAM_45_ROLES,
        slot_roles: &[],
        fixed: &[("aspectRatio", "auto")],
        ..ideogram("model_ideogram-v4-5", MaskConvention::WhiteEdit, 4096)
    },
    // Precise Edit takes inputs up to 67 MP and keeps the input size (research doc).
    OverlayRow {
        slot_roles: PRECISE_SLOTS,
        ..ideogram(
            "model_ideogram-v4-5-precise-edit",
            MaskConvention::WhiteEdit,
            8192,
        )
    },
    ideogram("model_ideogram-v3-balanced", MaskConvention::BlackEdit, 4096),
    ideogram("model_ideogram-v3-quality", MaskConvention::BlackEdit, 4096),
    ideogram("model_ideogram-v3-turbo", MaskConvention::BlackEdit, 4096),
    ideogram("model_ideogram-character", MaskConvention::BlackEdit, 4096),
];

/// The Scenario model ids with an overlay row (the recorder's `--model all`).
pub fn scenario_model_ids() -> impl Iterator<Item = &'static str> {
    SCENARIO_ROWS.iter().map(|r| r.model_id)
}

const FAKE_ROLES: &[(&str, ControlRole)] = &[
    ("image", ControlRole::Source),
    ("mask", ControlRole::Mask),
    ("prompt", ControlRole::Prompt),
    ("num_samples", ControlRole::CandidateCount),
];

/// The fake models' reference inputs (FR-PRV-35).
const FAKE_SLOTS: &[(&str, ReferenceRole)] = &[
    ("style_refs", ReferenceRole::Style),
    ("identity_ref", ReferenceRole::Identity),
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
        slot_roles: FAKE_SLOTS,
        slot_max: &[],
        fixed: &[],
        size_inputs: None,
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

    /// The role of a reference input (FR-PRV-31).
    pub fn slot_role(&self, input: &str) -> Option<ReferenceRole> {
        self.slot_roles
            .iter()
            .find(|(n, _)| *n == input)
            .map(|(_, r)| *r)
    }

    /// The fallback max count of a reference input with no `max` (FR-PRV-31).
    pub fn slot_max(&self, input: &str) -> Option<u32> {
        self.slot_max
            .iter()
            .find(|(n, _)| *n == input)
            .map(|(_, m)| *m)
    }

    /// True if the adapter fills this input itself (never a control).
    pub fn fills(&self, input: &str) -> bool {
        self.fixed.iter().any(|(n, _)| *n == input)
            || self
                .size_inputs
                .is_some_and(|(w, h)| w == input || h == input)
    }
}

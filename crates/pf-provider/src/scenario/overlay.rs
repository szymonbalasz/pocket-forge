//! The model overlay (#15, OQ-PRV-18: a static Rust table): what Scenario's capabilities don't
//! say. Each row gives a model its mask convention and soft rule, the input name per role
//! where Scenario doesn't mark it (R28), the accepted sent-image size, whether a dry run needs
//! the uploaded assets, the role and fallback count of each reference slot (FR-PRV-31), the
//! inputs the adapter fills itself (fixed values, the sent size), and where it was checked.
//! Masked edit is offered only for a model with a row **and** a mask input in its
//! capabilities (FR-PRV-11). Rows are per operation (GEN OQ-GEN-01): a model may have an
//! `edit_masked` row and a `generate` row with different roles, filled inputs and slots.
//! `generate` is offered for a model with a generate row whose `capabilities[]` lists
//! `txt2img`.
//!
//! Model ids and input names follow Scenario's model docs (checked 2026-11-08). The owner's
//! free `get-model` and dry-run recordings (PRV slice 5) confirm them, plus `dry_run_assets`
//! and the `soft` picks.

use crate::{ControlRole, InputPx, MaskConvention, Operation, ReferenceRole, SoftRule};

/// One overlay row (spec Appendix C).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayRow {
    pub model_id: &'static str,
    /// The operation this row describes (one row per model and operation).
    pub operation: Operation,
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
    /// `generate`: the input that takes the output ratio (`output_size.mode = aspect`); never
    /// a control. With `size_inputs` instead, the output is exact width × height (`wh`).
    pub aspect_input: Option<&'static str>,
    /// `generate`: ratio → value of an `aspect_input` that takes named sizes instead of `w:h`
    /// values (`imageSize: landscape_16_9`). Empty = the input's values are ratios.
    pub aspect_names: &'static [(&'static str, &'static str)],
    /// Inputs never shown as controls and never sent (outputs the app can't use: GPT Image's
    /// SVG / GIF inputs, Nano Banana's video). The model's default applies.
    pub ignored: &'static [&'static str],
    /// `(input, value)`: an allowed value dropped from a control (`resolution:
    /// match_input_image` when generating).
    pub drop_values: &'static [(&'static str, &'static str)],
    /// `generate`: the reference slot that takes the structural sketch (FR-GEN-15). `None` on
    /// every real model until a paid check confirms one (OQ-GEN-05).
    pub structure_slot: Option<&'static str>,
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
/// Scenario's third-party model docs (the BFL, Google, ByteDance, Krea and Recraft rows).
const THIRD_PARTY_DOC: &str =
    "https://docs.scenario.com/get-started/generation/third-party-model-generation";
const CHECKED: &str = "2026-11-08";
/// The rows added from the free `get-model` / dry-run recordings of 2026-11-11.
const CHECKED_11: &str = "2026-11-11";

/// GPT Image 2.6 inputs for outputs the app can't use (SVG, GIF frames) and its markup image
/// (not a reference): never controls, never sent; the model's defaults (`png`) apply.
const GPT_IGNORED: &[&str] = &[
    "markup",
    "outputFormat",
    "frameCount",
    "frameDurationMs",
    "svgModel",
    "complexity",
];

/// GPT Image: PNG, transparent alpha = edit, same size as the first reference image,
/// 16–3840 px in steps of 16, ≤ 50 MB.
const fn gpt(model_id: &'static str) -> OverlayRow {
    OverlayRow {
        model_id,
        operation: Operation::EditMasked,
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
        aspect_input: None,
        aspect_names: &[],
        ignored: GPT_IGNORED,
        drop_values: &[],
        structure_slot: None,
        doc_url: OPENAI_DOC,
        checked: CHECKED,
    }
}

/// Ideogram: `white` = edit for 4.5 / 4.5 Precise Edit, `black` = inpaint for V3 / Character.
const fn ideogram(model_id: &'static str, convention: MaskConvention, max: u32) -> OverlayRow {
    OverlayRow {
        model_id,
        operation: Operation::EditMasked,
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
        aspect_input: None,
        aspect_names: &[],
        ignored: &[],
        drop_values: &[],
        structure_slot: None,
        doc_url: IDEOGRAM_DOC,
        checked: CHECKED,
    }
}

/// FLUX.1 Pro Fill: `image` + `mask`, both required. Its mask has no `maskFrom`; the overlay
/// role marks it (FR-PRV-11).
const FLUX_FILL_ROLES: &[(&str, ControlRole)] = &[
    ("image", ControlRole::Source),
    ("mask", ControlRole::Mask),
    ("prompt", ControlRole::Prompt),
    ("seed", ControlRole::Seed),
];

/// GPT Image generate (GEN §10): no source; `referenceImages` are plain references (no role,
/// cost-affecting); the output is exact `width` × `height`; `background` has the background
/// role. The mask is listed only so it is dropped.
const GPT_GEN_ROLES: &[(&str, ControlRole)] = &[
    ("mask", ControlRole::Mask),
    ("prompt", ControlRole::Prompt),
    ("numOutputs", ControlRole::CandidateCount),
    ("background", ControlRole::Background),
];

/// Ideogram generate: the edit inputs (`image`, `mask`) are listed only so they are dropped.
const IDEOGRAM_GEN_ROLES: &[(&str, ControlRole)] = &[
    ("image", ControlRole::Source),
    ("mask", ControlRole::Mask),
    ("prompt", ControlRole::Prompt),
    ("numOutputs", ControlRole::CandidateCount),
    ("seed", ControlRole::Seed),
];

/// A generate-only model with no count input (1 per job, R72); `seed` is recognised by name.
const PROMPT_ROLES: &[(&str, ControlRole)] = &[("prompt", ControlRole::Prompt)];

/// A generate-only model with `numOutputs`.
const COUNT_ROLES: &[(&str, ControlRole)] = &[
    ("prompt", ControlRole::Prompt),
    ("numOutputs", ControlRole::CandidateCount),
];

/// Named output sizes (`imageSize`; integrator ruling 2026-11-11): offered as ratios
/// (`output_size.mode = aspect`), sent as the name. `square` (the smaller square) and Krea's
/// `auto` are not offered.
pub const NAMED_SIZES: &[(&str, &str)] = &[
    ("1:1", "square_hd"),
    ("4:3", "landscape_4_3"),
    ("16:9", "landscape_16_9"),
    ("3:4", "portrait_4_3"),
    ("9:16", "portrait_16_9"),
];

/// FLUX 2 also takes `match_input_image` as a resolution; a generate job has no input image.
const MATCH_INPUT_RESOLUTION: &[(&str, &str)] = &[("resolution", "match_input_image")];

/// The `generate` row from a model's edit row: same model and limits (the mask convention is
/// unused: a generate job sends no mask).
const fn generate(edit: OverlayRow) -> OverlayRow {
    OverlayRow {
        operation: Operation::Generate,
        fixed: &[],
        size_inputs: None,
        aspect_input: None,
        structure_slot: None,
        ..edit
    }
}

/// A `generate` row for a model with no edit row to derive it from. The caller sets its size
/// input (`size_inputs` or `aspect_input`). The mask convention is unused.
const fn generate_only(
    model_id: &'static str,
    roles: &'static [(&'static str, ControlRole)],
) -> OverlayRow {
    OverlayRow {
        model_id,
        operation: Operation::Generate,
        convention: MaskConvention::WhiteEdit,
        soft: SoftRule::PassThrough,
        roles,
        input_px: InputPx {
            min: 256,
            max: 4096,
            multiple_of: 8,
        },
        max_bytes: 50 * MB,
        dry_run_assets: true,
        slot_roles: &[],
        slot_max: &[],
        fixed: &[],
        size_inputs: None,
        aspect_input: None,
        aspect_names: &[],
        ignored: &[],
        drop_values: &[],
        structure_slot: None,
        doc_url: THIRD_PARTY_DOC,
        checked: CHECKED_11,
    }
}

/// A generate-only model whose output ratio is `aspectRatio` (`w:h` values).
const fn aspect_only(
    model_id: &'static str,
    roles: &'static [(&'static str, ControlRole)],
) -> OverlayRow {
    OverlayRow {
        aspect_input: Some("aspectRatio"),
        ..generate_only(model_id, roles)
    }
}

/// A generate-only model whose output is a named size (`imageSize`, [`NAMED_SIZES`]).
const fn named_size_only(
    model_id: &'static str,
    roles: &'static [(&'static str, ControlRole)],
) -> OverlayRow {
    OverlayRow {
        aspect_input: Some("imageSize"),
        aspect_names: NAMED_SIZES,
        ..generate_only(model_id, roles)
    }
}

/// GPT Image generate rows: `wh` 16–3840, step 16, from `width` / `height` (recorded).
const fn gpt_generate(model_id: &'static str) -> OverlayRow {
    OverlayRow {
        roles: GPT_GEN_ROLES,
        size_inputs: Some(("width", "height")),
        ..generate(gpt(model_id))
    }
}

/// Ideogram generate rows: `aspect` from `aspectRatio` (recorded).
const fn ideogram_generate(edit: OverlayRow) -> OverlayRow {
    OverlayRow {
        roles: IDEOGRAM_GEN_ROLES,
        aspect_input: Some("aspectRatio"),
        ..generate(edit)
    }
}

/// GPT Image 2.6 (Flare / Sunburst) generate: the 2.5 shape (recorded 2026-11-11). No edit
/// row: the paid checks of 2026-11-11 restyled the whole image and filled the masked area
/// with the mask's own pixels (black with the alpha mask, white with a white-is-edit mask), so
/// masked edit is withheld until the owner decides (spec Appendix C).
const fn gpt26_generate(model_id: &'static str) -> OverlayRow {
    OverlayRow {
        checked: CHECKED_11,
        ..gpt_generate(model_id)
    }
}

/// FLUX.1 Pro Fill (masked edit only, no `txt2img`). White = edit (BFL's convention; the paid
/// check of 2026-11-11 changed only the masked area). Its `image` and `mask` take ≤ 10 MB (`maxSize`); 256–2048 px
/// in steps of 32 (FLUX works in multiples of 32).
const FLUX_FILL: OverlayRow = OverlayRow {
    roles: FLUX_FILL_ROLES,
    slot_roles: &[],
    input_px: InputPx {
        min: 256,
        max: 2048,
        multiple_of: 32,
    },
    max_bytes: 10 * MB,
    doc_url: THIRD_PARTY_DOC,
    checked: CHECKED_11,
    ..ideogram("model_bfl-flux-1-pro-fill", MaskConvention::WhiteEdit, 2048)
};

/// Ideogram V4 Tiling generate: a named size; `strength` only applies to an input image.
const TILING_GENERATE: OverlayRow = OverlayRow {
    aspect_input: Some("imageSize"),
    aspect_names: NAMED_SIZES,
    ignored: &["strength"],
    slot_roles: &[],
    checked: CHECKED_11,
    ..ideogram_generate(ideogram(
        "model_ideogram-v4-tiling",
        MaskConvention::WhiteEdit,
        4096,
    ))
};

/// The Scenario rows (spec Appendix C; generate rows GEN §10). Edit rows first.
/// GPT Image 2 (`model_openai-gpt-image-2`) was removed on 2026-11-11: Scenario lists it as
/// `unlisted` (superseded by 2.6). Old revisions that name it still open; a saved model that
/// names it falls back (FR-RGN-11, FR-GEN-19).
pub static SCENARIO_ROWS: [OverlayRow; 26] = [
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
    ideogram(
        "model_ideogram-v3-balanced",
        MaskConvention::BlackEdit,
        4096,
    ),
    ideogram("model_ideogram-v3-quality", MaskConvention::BlackEdit, 4096),
    ideogram("model_ideogram-v3-turbo", MaskConvention::BlackEdit, 4096),
    ideogram("model_ideogram-character", MaskConvention::BlackEdit, 4096),
    FLUX_FILL,
    // Generate (GEN §10, from the recorded get-model files of 2026-11-08 and 2026-11-11).
    // Character, 4.5 Precise Edit and FLUX.1 Pro Fill list only `img2img`: no generate row.
    // GPT Image 2.6, Ideogram V4 Tiling (edit dry run 32 CU) and the rest: generate only.
    gpt_generate("model_openai-gpt-image-2-5-flare"),
    gpt_generate("model_openai-gpt-image-2-5-sunburst"),
    gpt26_generate("model_openai-gpt-image-2-6-flare"),
    gpt26_generate("model_openai-gpt-image-2-6-sunburst"),
    // 4.5's `referenceImages` are plain references when generating (no role).
    ideogram_generate(OverlayRow {
        slot_roles: &[],
        ..ideogram("model_ideogram-v4-5", MaskConvention::WhiteEdit, 4096)
    }),
    ideogram_generate(ideogram(
        "model_ideogram-v3-balanced",
        MaskConvention::BlackEdit,
        4096,
    )),
    ideogram_generate(ideogram(
        "model_ideogram-v3-quality",
        MaskConvention::BlackEdit,
        4096,
    )),
    ideogram_generate(ideogram(
        "model_ideogram-v3-turbo",
        MaskConvention::BlackEdit,
        4096,
    )),
    TILING_GENERATE,
    named_size_only("model_ideogram-v4", COUNT_ROLES),
    // FLUX 2: plain references; `match_input_image` is dropped from both size inputs.
    OverlayRow {
        drop_values: MATCH_INPUT_RESOLUTION,
        ..aspect_only("model_bfl-flux-2-pro-editing", PROMPT_ROLES)
    },
    OverlayRow {
        drop_values: MATCH_INPUT_RESOLUTION,
        ..aspect_only("model_bfl-flux-2-max-editing", PROMPT_ROLES)
    },
    aspect_only("model_bfl-flux-3-image", PROMPT_ROLES),
    // Nano Banana's `video` input is not a reference image.
    OverlayRow {
        ignored: &["video", "videoFps"],
        ..aspect_only("model_google-gemini-nano-banana-2-1", COUNT_ROLES)
    },
    // Seedream: exact `width` × `height`; its sequential multi-image mode is not offered.
    OverlayRow {
        size_inputs: Some(("width", "height")),
        ignored: &["sequentialImageGeneration", "maxImages"],
        input_px: InputPx {
            min: 1024,
            max: 6240,
            multiple_of: 16,
        },
        ..generate_only("model_bytedance-seedream-5-0-lite", PROMPT_ROLES)
    },
    named_size_only("model_krea-3", PROMPT_ROLES),
    // Recraft's `colors` palette (`inputs_array`) has no generic control.
    OverlayRow {
        ignored: &["colors"],
        ..named_size_only("model_recraft-v4-1", PROMPT_ROLES)
    },
];

// The Scenario model ids with an overlay row, each once (the recorder's `--model all`).
pub fn scenario_model_ids() -> impl Iterator<Item = &'static str> {
    unique_ids(&SCENARIO_ROWS).into_iter()
}

/// The Scenario model ids with a row for one operation (the recorder's generate dry runs).
pub fn scenario_model_ids_for(operation: Operation) -> impl Iterator<Item = &'static str> {
    SCENARIO_ROWS
        .iter()
        .filter(move |r| r.operation == operation)
        .map(|r| r.model_id)
}

/// The model ids of a table, each once, in table order.
pub fn unique_ids(rows: &[OverlayRow]) -> Vec<&'static str> {
    let mut ids: Vec<&'static str> = vec![];
    for r in rows {
        if !ids.contains(&r.model_id) {
            ids.push(r.model_id);
        }
    }
    ids
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
        operation: Operation::EditMasked,
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
        aspect_input: None,
        aspect_names: &[],
        ignored: &[],
        drop_values: &[],
        structure_slot: None,
        doc_url: "docs/specs/provider.md#appendix-d-fake-cases",
        checked: CHECKED,
    }
}

const FAKE_GEN_ROLES: &[(&str, ControlRole)] = &[
    ("prompt", ControlRole::Prompt),
    ("num_samples", ControlRole::CandidateCount),
    ("background", ControlRole::Background),
];

/// `fake-gen-wh`'s `refs` slot (no role) also takes the sketch; `fake-gen-aspect` has a style
/// slot.
const FAKE_GEN_SLOTS: &[(&str, ReferenceRole)] = &[("style_refs", ReferenceRole::Style)];

/// A fake generate model (GEN §10): `wh` 64–2048 step 16, or a few ratios.
const fn fake_generate(model_id: &'static str, wh: bool) -> OverlayRow {
    OverlayRow {
        operation: Operation::Generate,
        roles: FAKE_GEN_ROLES,
        input_px: InputPx {
            min: 64,
            max: 2048,
            multiple_of: 16,
        },
        slot_roles: FAKE_GEN_SLOTS,
        size_inputs: if wh { Some(("width", "height")) } else { None },
        aspect_input: if wh { None } else { Some("aspect_ratio") },
        structure_slot: if wh { Some("refs") } else { None },
        doc_url: "docs/specs/new-asset-generation.md#10-provider-interaction",
        ..fake(model_id, MaskConvention::WhiteEdit, SoftRule::PassThrough)
    }
}

/// The fake provider's rows: one edit model per mask convention (FR-PRV-03), then the three
/// generate models (GEN §10).
pub static FAKE_ROWS: [OverlayRow; 6] = [
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
    fake_generate("fake-gen-wh", true),
    fake_generate("fake-gen-aspect", false),
    fake_generate("fake-gen-refs", true),
];

/// The masked-edit row for a model, from a table.
#[cfg(test)]
pub fn row_in<'a>(rows: &'a [OverlayRow], model_id: &str) -> Option<&'a OverlayRow> {
    row_for(rows, model_id, Operation::EditMasked)
}

/// The row for a model and an operation, from a table.
pub fn row_for<'a>(
    rows: &'a [OverlayRow],
    model_id: &str,
    operation: Operation,
) -> Option<&'a OverlayRow> {
    rows.iter()
        .find(|r| r.model_id == model_id && r.operation == operation)
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
            || self.aspect_input == Some(input)
            || self
                .size_inputs
                .is_some_and(|(w, h)| w == input || h == input)
    }

    /// True if this input is never a control and never sent ([`OverlayRow::ignored`]).
    pub fn ignores(&self, input: &str) -> bool {
        self.ignored.contains(&input)
    }

    /// True if this input is never a control: filled by the adapter or ignored.
    pub fn skips(&self, input: &str) -> bool {
        self.fills(input) || self.ignores(input)
    }

    /// True if `value` is dropped from `input`'s allowed values.
    pub fn drops(&self, input: &str, value: &serde_json::Value) -> bool {
        self.drop_values
            .iter()
            .any(|(n, v)| *n == input && value.as_str() == Some(*v))
    }

    /// The value sent for an output ratio: its named size, or the ratio itself.
    pub fn aspect_value<'a>(&self, ratio: &'a str) -> Option<&'a str> {
        if self.aspect_names.is_empty() {
            return Some(ratio);
        }
        self.aspect_names
            .iter()
            .find(|(r, _)| *r == ratio)
            .map(|(_, n)| *n)
    }
}

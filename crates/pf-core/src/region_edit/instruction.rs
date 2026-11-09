//! The blend instruction (#55): an editable, app-wide text appended to the typed prompt of a
//! region edit at submit, when the draft's `blend_instruction` setting is on (the default). It
//! asks the model to continue the surrounding map across the mask edge. It is a content
//! instruction, not an alpha fade (the feather is separate), and never affects the estimate.

use serde::{Deserialize, Serialize};

use super::EVENT_BLEND_INSTRUCTION;
use crate::commands::NoInput;
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult};
use crate::manifest::Kind;
use crate::prompt;
use crate::settings::{self, RegionEditSettings};

/// The built-in instruction (owner, #55) for Maps. The kind sentence is added per asset kind.
pub const DEFAULT_BLEND_INSTRUCTION: &str = "Keep everything near the edges of the edited area \
consistent with the surrounding map: continue the existing walls, floor tiles, paths, lighting, \
scale and line style across the boundary so there is no visible seam. Place the new content \
toward the centre of the edited area.";

/// R94 / R95: the built-in instruction for Tokens and Items (R95's exact text).
pub const DEFAULT_BLEND_INSTRUCTION_IMAGE: &str = "Keep everything near the edges of the edited \
area consistent with the surrounding image: continue the existing shapes, colours, lighting, \
scale and line style across the boundary so there is no visible seam. Place the new content \
toward the centre of the edited area.";

/// The built-in text for an asset kind (R94, R95): Maps keep #55's text; Tokens and Items get
/// R95's "surrounding image" text.
pub fn default_text(kind: Kind) -> &'static str {
    match kind {
        Kind::Map => DEFAULT_BLEND_INSTRUCTION,
        Kind::Token | Kind::Item => DEFAULT_BLEND_INSTRUCTION_IMAGE,
    }
}

fn is_a_default(text: &str) -> bool {
    text == DEFAULT_BLEND_INSTRUCTION || text == DEFAULT_BLEND_INSTRUCTION_IMAGE
}

/// Longest instruction text accepted (characters).
pub const MAX_INSTRUCTION_CHARS: usize = 2000;

/// The sentence added for the asset kind: Maps get the view; Tokens and Items get none.
pub fn kind_sentence(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Map => Some("Top-down battlemap view."),
        Kind::Token | Kind::Item => None,
    }
}

/// The instruction text (the setting, or the kind's default when unset, R94).
pub fn base_text(s: &RegionEditSettings, kind: Kind) -> &str {
    s.blend_instruction
        .as_deref()
        .unwrap_or_else(|| default_text(kind))
}

/// The instruction appended for this kind: the text plus the kind sentence. Empty when the text
/// is blank (nothing is appended then).
pub fn instruction_for(s: &RegionEditSettings, kind: Kind) -> String {
    let text = base_text(s, kind).trim();
    if text.is_empty() {
        return String::new();
    }
    match kind_sentence(kind) {
        Some(k) => format!("{text} {k}"),
        None => text.to_owned(),
    }
}

/// The prompt as sent: the typed prompt, then `"\n\n"` and the instruction when `on`. Off, or a
/// blank instruction, sends the typed prompt byte for byte. Appended once: a typed prompt that
/// already ends with this instruction is sent as is.
pub fn compose(typed: &str, on: bool, instruction: &str) -> String {
    if !on || instruction.is_empty() {
        return typed.to_owned();
    }
    let suffix = format!("\n\n{instruction}");
    if typed.ends_with(&suffix) {
        return typed.to_owned();
    }
    if typed.is_empty() {
        return instruction.to_owned();
    }
    format!("{typed}{suffix}")
}

/// The final prompt a job sends for this draft (FR-RGN-09 as amended by PRM, FR-PRM-06): PRM's
/// [`prompt::compose`] of the typed prompt, the ticked helpers of the open project's kind and the
/// open project's instruction when the draft has it on. Without an open project (test seam):
/// no helpers and no kind sentence.
///
/// #55 "appended once" still holds: a typed prompt that already ends with the instruction is
/// sent without a second copy.
pub fn final_prompt(
    core: &Core,
    typed: &str,
    on: bool,
    ticks: Option<&[String]>,
) -> prompt::FinalPrompt {
    let state = core.state();
    let s = &state.settings.region_edit;
    let (kind, helpers, instruction) = match state.project.as_ref() {
        Some(p) => (
            p.manifest.kind,
            prompt::helpers::effective(&state.settings),
            instruction_for(s, p.manifest.kind),
        ),
        None => (
            Kind::Map,
            Vec::new(),
            base_text(s, Kind::Map).trim().to_owned(),
        ),
    };
    drop(state);
    let blend = (on && !instruction.is_empty()).then_some(instruction.as_str());
    let typed = match blend {
        Some(b) => typed
            .strip_suffix(b)
            .and_then(|t| t.strip_suffix("\n\n"))
            .unwrap_or(typed),
        None => typed,
    };
    prompt::compose(kind, typed, ticks, &helpers, blend)
}

/// `rgn_blend_instruction_get` / `_set` output.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BlendInstructionOutput {
    /// The instruction text in use (the setting, or the open project kind's default).
    pub text: String,
    /// The built-in default text for the open project's kind (Maps without one), for "Reset to
    /// default" (R94).
    pub default_text: String,
    /// True while the text is the default (no custom text stored).
    pub is_default: bool,
    /// What is appended for the open project's kind (text + kind sentence); `null` without one.
    pub for_project: Option<String>,
}

fn output(core: &Core) -> BlendInstructionOutput {
    let state = core.state();
    let s = &state.settings.region_edit;
    let kind = state
        .project
        .as_ref()
        .map_or(Kind::Map, |p| p.manifest.kind);
    BlendInstructionOutput {
        text: base_text(s, kind).to_owned(),
        default_text: default_text(kind).to_owned(),
        is_default: s.blend_instruction.is_none(),
        for_project: state
            .project
            .as_ref()
            .map(|p| instruction_for(s, p.manifest.kind)),
    }
}

/// `rgn_blend_instruction_get` (#55): the app-wide text, its default, and what the open project
/// appends.
pub struct RgnBlendInstructionGet;

impl Command for RgnBlendInstructionGet {
    const NAME: &'static str = "rgn_blend_instruction_get";
    type Input = NoInput;
    type Output = BlendInstructionOutput;

    fn run(core: &Core, _: NoInput) -> CoreResult<BlendInstructionOutput> {
        Ok(output(core))
    }
}

/// `rgn_blend_instruction_set` (#55): stores the text in `settings.json` `region_edit`; `null`
/// resets to the default. Emits `rgn:blend-instruction`.
pub struct RgnBlendInstructionSet;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlendInstructionSetInput {
    /// `null` = the default.
    pub text: Option<String>,
}

impl Validate for BlendInstructionSetInput {
    fn validate(&self) -> CoreResult<()> {
        if let Some(t) = &self.text
            && t.chars().count() > MAX_INSTRUCTION_CHARS
        {
            return Err(CoreError::invalid(
                "text",
                format!("the instruction is at most {MAX_INSTRUCTION_CHARS} characters"),
            ));
        }
        Ok(())
    }
}

impl Command for RgnBlendInstructionSet {
    const NAME: &'static str = "rgn_blend_instruction_set";
    type Input = BlendInstructionSetInput;
    type Output = BlendInstructionOutput;

    fn run(core: &Core, input: BlendInstructionSetInput) -> CoreResult<BlendInstructionOutput> {
        {
            let mut state = core.state();
            let mut next = state.settings.clone();
            // A default text itself is stored as "default", so a later default change (and the
            // per-kind default, R94) applies.
            next.region_edit.blend_instruction = input.text.filter(|t| !is_a_default(t));
            settings::save(&core.ctx, &next)?;
            state.settings = next;
        }
        let out = output(core);
        core.ctx.emit(
            EVENT_BLEND_INSTRUCTION,
            serde_json::to_value(&out).unwrap_or_default(),
        );
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_55_compose_appends_once_and_off_is_byte_for_byte() {
        let i = "Blend it.";
        assert_eq!(compose("a well", true, i), "a well\n\nBlend it.");
        assert_eq!(
            compose("a well\n\nBlend it.", true, i),
            "a well\n\nBlend it."
        );
        assert_eq!(compose("  a well \n", false, i), "  a well \n");
        assert_eq!(compose("a well", true, ""), "a well");
        assert_eq!(compose("", true, i), "Blend it.");
    }

    #[test]
    fn owner_55_kind_sentence_for_maps_only() {
        let s = RegionEditSettings::default();
        let map = instruction_for(&s, Kind::Map);
        assert!(map.starts_with(DEFAULT_BLEND_INSTRUCTION));
        assert!(map.ends_with(" Top-down battlemap view."));
        assert_eq!(
            instruction_for(&s, Kind::Token),
            DEFAULT_BLEND_INSTRUCTION_IMAGE
        );
        assert_eq!(
            instruction_for(&s, Kind::Item),
            DEFAULT_BLEND_INSTRUCTION_IMAGE
        );
        let custom = RegionEditSettings {
            blend_instruction: Some("  Match the ink.  ".into()),
            ..Default::default()
        };
        assert_eq!(
            instruction_for(&custom, Kind::Map),
            "Match the ink. Top-down battlemap view."
        );
        let blank = RegionEditSettings {
            blend_instruction: Some("   ".into()),
            ..Default::default()
        };
        assert_eq!(instruction_for(&blank, Kind::Map), "");
    }

    /// R94 / R95: Maps keep #55's text; Tokens and Items get R95's text exactly; a custom text
    /// is used as is for every kind.
    #[test]
    fn owner_r95_default_blend_text_per_kind() {
        let s = RegionEditSettings::default();
        assert!(instruction_for(&s, Kind::Map).starts_with(DEFAULT_BLEND_INSTRUCTION));
        assert!(DEFAULT_BLEND_INSTRUCTION.contains("surrounding map: continue the existing walls"));
        let r95 = "Keep everything near the edges of the edited area consistent with the \
surrounding image: continue the existing shapes, colours, lighting, scale and line style across \
the boundary so there is no visible seam. Place the new content toward the centre of the edited \
area.";
        for k in [Kind::Token, Kind::Item] {
            assert_eq!(instruction_for(&s, k), r95);
        }
        assert!(is_a_default(DEFAULT_BLEND_INSTRUCTION_IMAGE));
        let custom = RegionEditSettings {
            blend_instruction: Some("Match the ink.".into()),
            ..Default::default()
        };
        assert_eq!(instruction_for(&custom, Kind::Token), "Match the ink.");
    }
}

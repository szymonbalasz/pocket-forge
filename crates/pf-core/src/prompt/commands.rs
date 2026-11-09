//! Helper commands (spec §9): `prm_helpers_list`, `prm_helpers_save`, `prm_helpers_reset`,
//! `prm_compose_preview`. Settings writes are temp → rename; a failed command leaves settings
//! unchanged (FR-PRM-40). Settings editing works while the project is read-only (FR-PRM-12).

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::helpers::{self, Helper, MAX_HELPERS};
use super::{EVENT_HELPERS_CHANGED, PromptPart, Skipped, compose};
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::manifest::Kind;
use crate::settings::{self, Settings};

/// Longest typed prompt the preview takes (the draft's limit).
const MAX_PROMPT_CHARS: usize = 20_000;
/// Longest helper id accepted (a built-in slug or a ULID).
const MAX_ID_CHARS: usize = 100;

fn list_for(helpers: Vec<Helper>, kind: Option<Kind>) -> Vec<Helper> {
    match kind {
        Some(k) => helpers
            .into_iter()
            .filter(|h| h.kinds.contains(&k))
            .collect(),
        None => helpers,
    }
}

/// The stored section's unknown fields (kept across a save).
fn section_extra(s: &Settings) -> BTreeMap<String, Value> {
    s.prompt_helpers
        .as_ref()
        .and_then(|v| helpers::parse_section(v).ok())
        .map(|(_, sec)| sec.extra)
        .unwrap_or_default()
}

/// Stores `list` as the section, emits `prm:helpers-changed` and returns the stored list.
fn store(core: &Core, list: &[Helper]) -> CoreResult<Vec<Helper>> {
    let out = {
        let mut state = core.state();
        let mut next = state.settings.clone();
        let section = helpers::to_stored(list, section_extra(&next));
        let value = serde_json::to_value(&section)
            .map_err(|e| CoreError::internal(format!("serialising prompt helpers: {e}")))?;
        next.prompt_helpers = Some(value);
        let out = helpers::effective(&next);
        settings::save(&core.ctx, &next)?;
        state.settings = next;
        out
    };
    core.ctx.emit(
        EVENT_HELPERS_CHANGED,
        serde_json::to_value(&out).unwrap_or_default(),
    );
    Ok(out)
}

// ---------------------------------------------------------------- list

/// `prm_helpers_list` (§9): every helper in settings order, hidden ones included (Settings shows
/// them); `kind` keeps those for that asset type.
pub struct PrmHelpersList;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelpersListInput {
    #[serde(default)]
    pub kind: Option<Kind>,
}

impl Validate for HelpersListInput {}

impl Command for PrmHelpersList {
    const NAME: &'static str = "prm_helpers_list";
    type Input = HelpersListInput;
    type Output = Vec<Helper>;

    fn run(core: &Core, input: HelpersListInput) -> CoreResult<Vec<Helper>> {
        let all = helpers::effective(&core.state().settings);
        Ok(list_for(all, input.kind))
    }
}

// ---------------------------------------------------------------- save

/// `prm_helpers_save` (FR-PRM-02/03): the full ordered list. Every built-in must be listed
/// (hide it instead). A helper without an `id` is new and gets a ULID.
pub struct PrmHelpersSave;

/// One helper as Settings sends it. A `prm_helpers_list` row can be sent back as is: `builtin`
/// and `modified` are derived, so they are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct HelperInput {
    #[serde(default)]
    pub id: Option<String>,
    pub label: String,
    pub text: String,
    pub kinds: Vec<Kind>,
    #[serde(default)]
    pub default_on: bool,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelpersSaveInput {
    pub helpers: Vec<HelperInput>,
}

impl Validate for HelpersSaveInput {
    fn validate(&self) -> CoreResult<()> {
        if self.helpers.len() > MAX_HELPERS {
            return Err(helpers::too_many());
        }
        let mut seen = HashSet::new();
        for (i, h) in self.helpers.iter().enumerate() {
            if let Some(id) = &h.id {
                if id.chars().count() > MAX_ID_CHARS {
                    return Err(CoreError::invalid(
                        format!("helpers[{i}].id"),
                        "id too long",
                    ));
                }
                helpers::check_id(i, id)?;
                if !seen.insert(id.as_str()) {
                    return Err(CoreError::invalid(
                        format!("helpers[{i}].id"),
                        "the same helper is listed twice",
                    ));
                }
            }
            helpers::check_fields(i, &h.label, &h.text, &h.kinds)?;
        }
        if let Some(st) = helpers::STARTER.iter().find(|st| !seen.contains(st.id)) {
            return Err(CoreError::invalid(
                "helpers",
                format!(
                    "the built-in helper {:?} can't be deleted; hide it instead",
                    st.label
                ),
            )
            .with_detail(serde_json::json!({ "id": st.id })));
        }
        Ok(())
    }
}

impl Command for PrmHelpersSave {
    const NAME: &'static str = "prm_helpers_save";
    type Input = HelpersSaveInput;
    type Output = Vec<Helper>;

    fn run(core: &Core, input: HelpersSaveInput) -> CoreResult<Vec<Helper>> {
        let list: Vec<Helper> = input
            .helpers
            .into_iter()
            .map(|h| {
                let id = h.id.unwrap_or_else(|| core.ctx.new_id().to_string());
                let builtin = helpers::starter(&id).is_some();
                Helper {
                    id,
                    label: h.label.trim().to_owned(),
                    text: h.text.trim().to_owned(),
                    kinds: helpers::canonical_kinds(&h.kinds),
                    default_on: h.default_on,
                    builtin,
                    hidden: h.hidden,
                    modified: false,
                }
            })
            .collect();
        store(core, &list)
    }
}

// ---------------------------------------------------------------- reset

/// `prm_helpers_reset` (FR-PRM-03): `{id}` restores one built-in's starter label, text, kinds
/// and `default_on` and unhides it, in place; `{all: true}` restores the starter set (in its
/// order) and keeps the user-made helpers after it.
pub struct PrmHelpersReset;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelpersResetInput {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub all: bool,
}

impl Validate for HelpersResetInput {
    fn validate(&self) -> CoreResult<()> {
        match (&self.id, self.all) {
            (Some(_), true) | (None, false) => Err(CoreError::invalid(
                "id",
                "give either a helper id or all: true",
            )),
            (Some(id), false) if id.chars().count() > MAX_ID_CHARS => {
                Err(CoreError::invalid("id", "id too long"))
            }
            _ => Ok(()),
        }
    }
}

impl Command for PrmHelpersReset {
    const NAME: &'static str = "prm_helpers_reset";
    type Input = HelpersResetInput;
    type Output = Vec<Helper>;

    fn run(core: &Core, input: HelpersResetInput) -> CoreResult<Vec<Helper>> {
        let current = helpers::effective(&core.state().settings);
        let next: Vec<Helper> = match input.id {
            None => helpers::starter_set()
                .into_iter()
                .chain(current.into_iter().filter(|h| !h.builtin))
                .collect(),
            Some(id) => {
                let at = current.iter().position(|h| h.id == id).ok_or_else(|| {
                    CoreError::new(ErrorCode::NotFound, "no such prompt helper").with_field("id")
                })?;
                let st = helpers::starter(&id).ok_or_else(|| {
                    CoreError::invalid("id", "only a built-in helper can be reset")
                })?;
                let mut next = current;
                next[at] = st.helper();
                next
            }
        };
        store(core, &next)
    }
}

// ---------------------------------------------------------------- compose preview

/// `prm_compose_preview` (FR-PRM-08): the final prompt as it would be sent, with its parts, the
/// skipped helpers and the character count. No provider call; no effect on the estimate.
pub struct PrmComposePreview;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComposePreviewInput {
    pub kind: Kind,
    #[serde(default)]
    pub typed_prompt: String,
    /// The draft's ticks; `null` = the defaults (FR-PRM-05).
    #[serde(default)]
    pub helper_ids: Option<Vec<String>>,
    /// The blend instruction as appended (RGN, with it on); `null` = none.
    #[serde(default)]
    pub blend_instruction: Option<String>,
}

impl Validate for ComposePreviewInput {
    fn validate(&self) -> CoreResult<()> {
        if self.typed_prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(CoreError::invalid("typed_prompt", "the prompt is too long"));
        }
        if let Some(ids) = &self.helper_ids {
            if ids.len() > MAX_HELPERS {
                return Err(CoreError::invalid("helper_ids", "too many helper ids"));
            }
            if ids.iter().any(|i| i.chars().count() > MAX_ID_CHARS) {
                return Err(CoreError::invalid("helper_ids", "not a helper id"));
            }
        }
        if self.blend_instruction.as_ref().is_some_and(|b| {
            b.chars().count() > crate::region_edit::instruction::MAX_INSTRUCTION_CHARS + 100
        }) {
            return Err(CoreError::invalid(
                "blend_instruction",
                "the blend instruction is too long",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComposePreviewOutput {
    pub text: String,
    pub parts: Vec<PromptPart>,
    pub skipped: Vec<Skipped>,
    /// Characters (Unicode scalar values) of `text`.
    pub chars: usize,
}

impl Command for PrmComposePreview {
    const NAME: &'static str = "prm_compose_preview";
    type Input = ComposePreviewInput;
    type Output = ComposePreviewOutput;

    fn run(core: &Core, input: ComposePreviewInput) -> CoreResult<ComposePreviewOutput> {
        let list = helpers::effective(&core.state().settings);
        let f = compose(
            input.kind,
            &input.typed_prompt,
            input.helper_ids.as_deref(),
            &list,
            input.blend_instruction.as_deref(),
        );
        Ok(ComposePreviewOutput {
            chars: f.text.chars().count(),
            text: f.text,
            parts: f.parts,
            skipped: f.skipped,
        })
    }
}

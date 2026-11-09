//! Prompt helpers (FR-PRM-01..05, 10): the starter set, the `prompt_helpers` settings section
//! (built-ins stored as overrides only), validation, reset and the Retry ticks.
//!
//! The section is kept raw in [`Settings`]: an invalid section never resets the whole file. It
//! falls back to the starter set with a warning (§11) and is left on disk until the next save.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::manifest::Kind;
use crate::settings::{Settings, SettingsWarning};

/// At most this many helpers (FR-PRM-01).
pub const MAX_HELPERS: usize = 60;
/// Label length, characters (FR-PRM-01).
pub const MAX_LABEL_CHARS: usize = 40;
/// Text length, characters (FR-PRM-01).
pub const MAX_TEXT_CHARS: usize = 500;
/// Id prefix of the built-ins.
pub const BUILTIN_PREFIX: &str = "builtin:";
/// The settings section (FR-PRM-02).
pub const SECTION: &str = "prompt_helpers";
/// Warning code when the stored section is invalid (§11).
pub const WARNING_INVALID: &str = "prompt_helpers_invalid";

/// A built-in helper as shipped ([Appendix A]; all off by default, #60).
#[derive(Debug, Clone, Copy)]
pub struct StarterHelper {
    pub id: &'static str,
    pub kinds: &'static [Kind],
    pub label: &'static str,
    pub text: &'static str,
}

/// The starter set (spec Appendix A), in its settings order.
pub const STARTER: [StarterHelper; 9] = [
    StarterHelper {
        id: "builtin:top-down-map",
        kinds: &[Kind::Map],
        label: "Top-down battlemap",
        text: "Top-down battlemap view.",
    },
    StarterHelper {
        id: "builtin:match-style",
        kinds: &[Kind::Map],
        label: "Match surrounding style",
        text: "Match the existing art style, palette, line weight and lighting.",
    },
    StarterHelper {
        id: "builtin:no-people",
        kinds: &[Kind::Map],
        label: "No people or tokens",
        text: "No people, creatures or tokens.",
    },
    StarterHelper {
        id: "builtin:no-grid-text",
        kinds: &[Kind::Map],
        label: "No grid or text",
        text: "No grid lines, labels or text.",
    },
    StarterHelper {
        id: "builtin:token-single",
        kinds: &[Kind::Token],
        label: "Single creature",
        text: "A single creature, centred and fully in frame, seen from above.",
    },
    StarterHelper {
        id: "builtin:token-plain-bg",
        kinds: &[Kind::Token],
        label: "Plain background",
        text: "Plain, uncluttered background.",
    },
    StarterHelper {
        id: "builtin:item-single",
        kinds: &[Kind::Item],
        label: "Single item",
        text: "A single item, centred and fully in frame.",
    },
    StarterHelper {
        id: "builtin:item-plain-bg",
        kinds: &[Kind::Item],
        label: "Plain background",
        text: "Plain neutral background, soft even lighting.",
    },
    StarterHelper {
        id: "builtin:no-text",
        kinds: &[Kind::Token, Kind::Item],
        label: "No text",
        text: "No text, frame, border or watermark.",
    },
];

/// The starter entry for a built-in id.
pub fn starter(id: &str) -> Option<&'static StarterHelper> {
    STARTER.iter().find(|s| s.id == id)
}

impl StarterHelper {
    /// As a helper, unmodified and visible.
    pub fn helper(&self) -> Helper {
        Helper {
            id: self.id.to_owned(),
            label: self.label.to_owned(),
            text: self.text.to_owned(),
            kinds: self.kinds.to_vec(),
            default_on: false,
            builtin: true,
            hidden: false,
            modified: false,
        }
    }
}

/// A helper as the app uses it (FR-PRM-01), and the `prm_helpers_list` row (§9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Helper {
    pub id: String,
    pub label: String,
    pub text: String,
    /// In `map, token, item` order, no repeats.
    pub kinds: Vec<Kind>,
    pub default_on: bool,
    pub builtin: bool,
    /// Built-ins only can be hidden in practice (user-made ones are deleted instead).
    pub hidden: bool,
    /// A built-in whose label, text, kinds or `default_on` differ from the starter.
    pub modified: bool,
}

impl Helper {
    /// Shown in the helpers row of a project of this kind (FR-PRM-04).
    pub fn applies_to(&self, kind: Kind) -> bool {
        !self.hidden && self.kinds.contains(&kind)
    }
}

/// The stored section: `prompt_helpers{helpers[]}` (FR-PRM-02). Unknown fields are kept.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PromptHelperSettings {
    #[serde(default)]
    pub helpers: Vec<StoredHelper>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One stored entry. A built-in holds only what differs from its starter (`{id}` = unmodified,
/// kept for the order); a user-made helper holds every field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StoredHelper {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<Kind>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_on: Option<bool>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// `map, token, item` order, no repeats.
pub fn canonical_kinds(kinds: &[Kind]) -> Vec<Kind> {
    Kind::ALL
        .into_iter()
        .filter(|k| kinds.contains(k))
        .collect()
}

/// FR-PRM-01 field checks for helper `i` (`helpers[i].<field>`).
pub fn check_fields(i: usize, label: &str, text: &str, kinds: &[Kind]) -> CoreResult<()> {
    let field = |f: &str| format!("helpers[{i}].{f}");
    let label_n = label.trim().chars().count();
    if label_n == 0 || label_n > MAX_LABEL_CHARS {
        return Err(CoreError::invalid(
            field("label"),
            format!("a helper label is 1 to {MAX_LABEL_CHARS} characters"),
        ));
    }
    let text_n = text.trim().chars().count();
    if text_n == 0 || text_n > MAX_TEXT_CHARS {
        return Err(CoreError::invalid(
            field("text"),
            format!("a helper text is 1 to {MAX_TEXT_CHARS} characters"),
        ));
    }
    if kinds.is_empty() {
        return Err(CoreError::invalid(
            field("kinds"),
            "a helper needs at least one asset type",
        ));
    }
    Ok(())
}

/// FR-PRM-01: `builtin:<slug>` of a known built-in, or a ULID.
pub fn check_id(i: usize, id: &str) -> CoreResult<()> {
    let ok = if id.starts_with(BUILTIN_PREFIX) {
        starter(id).is_some()
    } else {
        Ulid::from_string(id).is_ok()
    };
    if ok {
        Ok(())
    } else {
        Err(CoreError::invalid(
            format!("helpers[{i}].id"),
            "not a known built-in or a user helper id",
        ))
    }
}

pub fn too_many() -> CoreError {
    CoreError::new(
        ErrorCode::TooMany,
        format!("at most {MAX_HELPERS} prompt helpers"),
    )
    .with_field("helpers")
    .with_detail(serde_json::json!({ "max": MAX_HELPERS }))
}

/// The helpers a stored section describes, in settings order. Built-ins missing from the list
/// (added by a later version) follow at the end; a built-in this version doesn't know is
/// ignored. Any rule broken → the error (the caller falls back to the starter set).
pub fn resolve(section: &PromptHelperSettings) -> CoreResult<Vec<Helper>> {
    let mut out = Vec::with_capacity(section.helpers.len().max(STARTER.len()));
    let mut seen = HashSet::new();
    for (i, s) in section.helpers.iter().enumerate() {
        if !seen.insert(s.id.as_str()) {
            return Err(CoreError::invalid(
                format!("helpers[{i}].id"),
                "the same helper is listed twice",
            ));
        }
        let h = if let Some(st) = starter(&s.id) {
            let base = st.helper();
            let mut h = Helper {
                label: s.label.clone().unwrap_or(base.label.clone()),
                text: s.text.clone().unwrap_or(base.text.clone()),
                kinds: s
                    .kinds
                    .as_deref()
                    .map(canonical_kinds)
                    .unwrap_or(base.kinds.clone()),
                default_on: s.default_on.unwrap_or(base.default_on),
                hidden: s.hidden,
                ..base.clone()
            };
            h.modified = differs(&h, &base);
            h
        } else if s.id.starts_with(BUILTIN_PREFIX) {
            continue;
        } else {
            check_id(i, &s.id)?;
            Helper {
                id: s.id.clone(),
                label: s.label.clone().unwrap_or_default(),
                text: s.text.clone().unwrap_or_default(),
                kinds: canonical_kinds(s.kinds.as_deref().unwrap_or_default()),
                default_on: s.default_on.unwrap_or(false),
                builtin: false,
                hidden: s.hidden,
                modified: false,
            }
        };
        check_fields(i, &h.label, &h.text, &h.kinds)?;
        out.push(h);
    }
    out.extend(
        STARTER
            .iter()
            .filter(|st| !seen.contains(st.id))
            .map(StarterHelper::helper),
    );
    if out.len() > MAX_HELPERS {
        return Err(too_many());
    }
    Ok(out)
}

fn differs(h: &Helper, base: &Helper) -> bool {
    h.label != base.label
        || h.text != base.text
        || h.kinds != base.kinds
        || h.default_on != base.default_on
}

/// The stored form of a list: built-ins as overrides only, user-made helpers in full.
pub fn to_stored(helpers: &[Helper], extra: BTreeMap<String, Value>) -> PromptHelperSettings {
    let helpers = helpers
        .iter()
        .map(|h| match starter(&h.id) {
            Some(st) => StoredHelper {
                id: h.id.clone(),
                label: (h.label != st.label).then(|| h.label.clone()),
                text: (h.text != st.text).then(|| h.text.clone()),
                kinds: (h.kinds != st.kinds).then(|| h.kinds.clone()),
                default_on: h.default_on.then_some(true),
                hidden: h.hidden,
            },
            None => StoredHelper {
                id: h.id.clone(),
                label: Some(h.label.clone()),
                text: Some(h.text.clone()),
                kinds: Some(h.kinds.clone()),
                default_on: Some(h.default_on),
                hidden: h.hidden,
            },
        })
        .collect();
    PromptHelperSettings { helpers, extra }
}

/// The starter set (an absent section, FR-PRM-02).
pub fn starter_set() -> Vec<Helper> {
    STARTER.iter().map(StarterHelper::helper).collect()
}

/// Parses and resolves a stored section value.
pub fn parse_section(value: &Value) -> CoreResult<(Vec<Helper>, PromptHelperSettings)> {
    let section: PromptHelperSettings = serde_json::from_value(value.clone())
        .map_err(|e| CoreError::invalid(SECTION, format!("invalid {SECTION} settings: {e}")))?;
    Ok((resolve(&section)?, section))
}

/// The helpers in use: the stored section, or the starter set when it is absent or invalid.
pub fn effective(settings: &Settings) -> Vec<Helper> {
    match &settings.prompt_helpers {
        None => starter_set(),
        Some(v) => parse_section(v)
            .map(|(h, _)| h)
            .unwrap_or_else(|_| starter_set()),
    }
}

/// The load warning for an invalid section (§11, FR-PRJ-39), if any.
pub fn load_warning(settings: &Settings) -> Option<SettingsWarning> {
    let v = settings.prompt_helpers.as_ref()?;
    let e = parse_section(v).err()?;
    tracing::warn!(code = %e.code, "prompt_helpers settings invalid; using the starter helpers");
    Some(SettingsWarning {
        code: WARNING_INVALID,
        message: format!(
            "The prompt helper settings couldn't be read ({}), so the starter helpers are used \
             until you save them again.",
            e.message
        ),
    })
}

/// A draft's ticks (FR-PRM-05) at a command boundary: at most [`MAX_HELPERS`] ids, each 1–100
/// bytes. Unknown ids are allowed (a deleted helper is simply not sent, FR-PRM-06).
pub fn check_ticks(field: &str, ticks: Option<&[String]>) -> CoreResult<()> {
    if let Some(t) = ticks
        && (t.len() > MAX_HELPERS || t.iter().any(|id| id.is_empty() || id.len() > 100))
    {
        return Err(CoreError::invalid(field, "not a list of prompt helper ids"));
    }
    Ok(())
}

/// The ticked helpers: the draft's explicit list, or (`None`) the defaults: every helper shown
/// for this kind with `default_on` (FR-PRM-05).
pub fn ticked_ids(kind: Kind, ticks: Option<&[String]>, helpers: &[Helper]) -> Vec<String> {
    match ticks {
        Some(t) => t.to_vec(),
        None => helpers
            .iter()
            .filter(|h| h.applies_to(kind) && h.default_on)
            .map(|h| h.id.clone())
            .collect(),
    }
}

/// FR-PRM-10: what Retry restores from a job's `prompt_helpers[]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RetryTicks {
    /// The draft ticks (explicit; `[]` for a job without helpers).
    pub ticks: Vec<String>,
    /// Ticked ids whose current text differs from the recorded one (badged `changed since NNN`;
    /// the current text is sent).
    pub changed: Vec<String>,
    /// Recorded helpers that can't be sent any more (deleted, hidden, or no longer for this
    /// kind): listed, unticked, not sent.
    pub missing: Vec<super::SentHelper>,
}

/// FR-PRM-10: what the prompt block badges after a Retry (RGN `rgn_job_retry_draft`, GEN
/// `gen_job_retry_draft`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptRetry {
    /// The job's iteration `NNN` (`changed since NNN`), or "the job" before it has one.
    pub since: String,
    pub changed: Vec<String>,
    pub missing: Vec<super::SentHelper>,
}

impl RetryTicks {
    /// The badges for the prompt block; `None` when nothing changed and nothing is missing.
    pub fn badges(&self, since: &str) -> Option<PromptRetry> {
        (!self.changed.is_empty() || !self.missing.is_empty()).then(|| PromptRetry {
            since: since.to_owned(),
            changed: self.changed.clone(),
            missing: self.missing.clone(),
        })
    }
}

/// FR-PRM-10: the ticks a Retry sets from the job's recorded helpers.
pub fn retry_ticks(kind: Kind, recorded: &[super::SentHelper], helpers: &[Helper]) -> RetryTicks {
    let mut out = RetryTicks::default();
    for r in recorded {
        match helpers.iter().find(|h| h.id == r.id && h.applies_to(kind)) {
            Some(h) => {
                if h.text != r.text {
                    out.changed.push(h.id.clone());
                }
                out.ticks.push(h.id.clone());
            }
            None => out.missing.push(r.clone()),
        }
    }
    out
}

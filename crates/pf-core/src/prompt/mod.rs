//! Prompt helper (PRM, [prompt-helper spec](../../../../docs/specs/prompt-helper.md)).
//!
//! Slice 1, the helpers core: the helper definitions and the `prompt_helpers` settings section
//! ([`helpers`]), [`compose`] (the final prompt = typed prompt + ticked helper texts + the blend
//! instruction, FR-PRM-06/07) for RGN and GEN, the job record entry (FR-PRM-09) and the helper
//! commands ([`commands`]). Plain text only: nothing here calls the provider (FR-PRM-31).

pub mod commands;
pub mod context;
pub mod helpers;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

use crate::manifest::{Extra, Kind};
pub use helpers::{Helper, PromptRetry, RetryTicks, retry_ticks};

/// Event: the helper definitions changed (save or reset); payload = the full list.
pub const EVENT_HELPERS_CHANGED: &str = "prm:helpers-changed";
/// The job field (FR-PRM-09).
pub const JOB_FIELD: &str = "prompt_helpers";

/// Where a part of the final prompt came from (FR-PRM-08).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PartSource {
    Typed,
    Helper,
    Blend,
}

/// One marked part of the final prompt preview. Helper parts carry the helper id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptPart {
    pub source: PartSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub text: String,
}

/// A helper as sent: the job's `prompt_helpers[]` entry (FR-PRM-09).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentHelper {
    pub id: String,
    pub label: String,
    pub text: String,
}

/// Why a ticked helper wasn't sent (FR-PRM-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Duplicate,
    InBlendInstruction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub id: String,
    pub reason: SkipReason,
}

/// What [`compose`] produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FinalPrompt {
    /// The prompt as sent.
    pub text: String,
    /// `text` split by source, in order (the helper parts are joined with one space in `text`).
    pub parts: Vec<PromptPart>,
    /// The helpers actually sent, in order (the job record, FR-PRM-09).
    pub sent: Vec<SentHelper>,
    /// Ticked helpers left out by the dedupe (FR-PRM-07).
    pub skipped: Vec<Skipped>,
}

/// FR-PRM-07 normal form: trimmed, whitespace collapsed, lower case, one trailing `.` dropped.
pub fn normalise(s: &str) -> String {
    let mut n = s.split_whitespace().collect::<Vec<_>>().join(" ");
    n = n.to_lowercase();
    if let Some(stripped) = n.strip_suffix('.') {
        n = stripped.trim_end().to_owned();
    }
    n
}

/// True if `needle` occurs in `hay` on word boundaries (both already normalised).
fn contains_phrase(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    hay.match_indices(needle).any(|(at, _)| {
        !word(hay[..at].chars().next_back()) && !word(hay[at + needle.len()..].chars().next())
    })
}

/// FR-PRM-06/07: the final prompt = the non-empty parts joined with `\n\n`: (1) the typed prompt,
/// trimmed at the end (byte for byte when nothing is added, FR-RGN-09); (2) the ticked helpers'
/// texts in helpers-row (settings) order, joined with one space; (3) the blend instruction (RGN
/// only; `None` or blank = none).
///
/// `ticks` is the draft's list (`None` = the defaults, FR-PRM-05). An unticked, hidden, deleted
/// or wrong-kind helper is never added. A helper whose text is in the blend instruction, or equal
/// to an earlier ticked helper's, is skipped and reported. The typed prompt is never deduplicated
/// against.
pub fn compose(
    kind: Kind,
    typed: &str,
    ticks: Option<&[String]>,
    helpers: &[Helper],
    blend: Option<&str>,
) -> FinalPrompt {
    let ticked = helpers::ticked_ids(kind, ticks, helpers);
    let blend = blend.map(str::trim).filter(|b| !b.is_empty());
    let blend_norm = blend.map(normalise).unwrap_or_default();

    let mut sent = Vec::new();
    let mut skipped = Vec::new();
    let mut earlier: Vec<String> = Vec::new();
    for h in helpers
        .iter()
        .filter(|h| h.applies_to(kind) && ticked.contains(&h.id))
    {
        let norm = normalise(&h.text);
        let reason = if blend.is_some() && contains_phrase(&blend_norm, &norm) {
            Some(SkipReason::InBlendInstruction)
        } else if earlier.contains(&norm) {
            Some(SkipReason::Duplicate)
        } else {
            None
        };
        earlier.push(norm);
        match reason {
            Some(reason) => skipped.push(Skipped {
                id: h.id.clone(),
                reason,
            }),
            None => sent.push(SentHelper {
                id: h.id.clone(),
                label: h.label.clone(),
                text: h.text.trim().to_owned(),
            }),
        }
    }

    let mut parts = Vec::new();
    // Trimmed at the end before an added part; a typed prompt with nothing added is sent byte for
    // byte (FR-RGN-09, blend off), so the preview stays exactly what is sent (FR-PRM-08).
    let raw = typed;
    let typed = if sent.is_empty() && blend.is_none() {
        raw
    } else {
        raw.trim_end()
    };
    if !typed.trim_end().is_empty() {
        parts.push(PromptPart {
            source: PartSource::Typed,
            id: None,
            text: typed.to_owned(),
        });
    }
    parts.extend(sent.iter().map(|s| PromptPart {
        source: PartSource::Helper,
        id: Some(s.id.clone()),
        text: s.text.clone(),
    }));
    if let Some(b) = blend {
        parts.push(PromptPart {
            source: PartSource::Blend,
            id: None,
            text: b.to_owned(),
        });
    }

    let mut blocks: Vec<String> = Vec::new();
    if !typed.trim_end().is_empty() {
        blocks.push(typed.to_owned());
    }
    if !sent.is_empty() {
        blocks.push(
            sent.iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    if let Some(b) = blend {
        blocks.push(b.to_owned());
    }
    FinalPrompt {
        text: blocks.join("\n\n"),
        parts,
        sent,
        skipped,
    }
}

/// FR-PRM-09: writes the job's `prompt_helpers[]` into the new job's fields (with the
/// `requested` commit; never changed afterwards).
pub fn record_on_job(extra: &mut Extra, sent: &[SentHelper]) {
    extra.insert(
        JOB_FIELD.into(),
        serde_json::to_value(sent).unwrap_or_else(|_| serde_json::json!([])),
    );
}

//! New image's chat context (FR-GEN-17, FR-PRM-42): GEN's [`ChatContextSource`]. It reads only
//! the tab's published draft (`GenJobDraft`, the input `gen_job_estimate` takes, FR-PRM-23), the
//! capabilities and model names GEN's commands already fetched this session, and the project's
//! files: no GEN command, no provider call (FR-PRM-31).
//!
//! **Text:** kind; model name and id; every visible control with its value and range or allowed
//! values; the size (grid or ratio, and what is sent); the typed prompt, the ticked helpers and
//! the final prompt (PRM's `compose`, never the blend instruction); each `on` reference; the
//! sketch or "none". **Images**, in this order: the sketch (`structure`), then the `on`
//! references in send order (`reference`), each the bytes REF's prepare would send, downscaled
//! per FR-PRM-25. An image that can't be prepared is dropped and named in the facts. The turn
//! limit (6 images / 4 MiB, [`crate::prompt::context::limit_images`]) drops from the tail, so
//! the sketch goes last; the chat core applies it after its per-session filter.

use std::collections::BTreeMap;

use pf_provider::Capabilities;
use serde_json::Value;
use ulid::Ulid;

use super::commands::GenJobDraft;
use super::size::{self, GenSize};
use crate::core::{Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::prompt::context::{
    ChatContext, ChatContextSource, ChatControl, ChatHost, ChatImage, ChatImageRole, ChatModel,
    chat_jpeg, value_type,
};
use crate::prompt::{self, helpers};
use crate::references::prepare::{self as ref_prepare, STRUCTURE_KEY};
use crate::references::resolve::{self, DraftReferences, Source};
use crate::region_edit::controls;

/// The New image tab's [`ChatContextSource`] (host `gen`).
pub struct GenContextSource;

/// How many unavailable items the build skips before it gives up on images.
const MAX_SKIPS: usize = 32;

impl ChatContextSource for GenContextSource {
    fn host(&self) -> ChatHost {
        ChatHost::Gen
    }

    /// `input` is the tab's published draft (`gen_job_estimate`'s `draft`).
    fn build(&self, core: &Core, project_id: Ulid, input: &Value) -> CoreResult<ChatContext> {
        let draft: GenJobDraft = serde_json::from_value(input.clone())
            .map_err(|e| CoreError::invalid("input", format!("not a New image draft: {e}")))?;
        draft.validate()?;
        let model_id = draft.model_id.trim().to_owned();
        let (kind, list, caps, name, dir, settings) = {
            let state = core.state();
            let p = state
                .project
                .as_ref()
                .filter(|p| p.manifest.project_id == project_id)
                .ok_or_else(|| {
                    CoreError::new(ErrorCode::NoProject, "that asset project is not open")
                })?;
            (
                p.manifest.kind,
                helpers::effective(&state.settings),
                state.generate.caps.get(&model_id).cloned(),
                state.generate.model_names.get(&model_id).cloned(),
                p.dir.clone(),
                state.settings.clone(),
            )
        };
        // FR-GEN-16: typed prompt + ticked helpers, never the blend instruction.
        let fp = prompt::compose(
            kind,
            &draft.typed_prompt,
            draft.prompt_helpers.as_deref(),
            &list,
            None,
        );
        let mut facts = vec![];
        let controls = match &caps {
            Some(c) => visible_controls(c, &draft.controls),
            None => {
                facts.push("the model's controls aren't loaded yet".to_owned());
                raw_controls(&draft.controls)
            }
        };
        facts.push(size_fact(caps.as_ref(), &draft.size));
        if let Some(c) = &caps {
            let n = controls::check_controls(c, &draft.controls, &fp.text, draft.seed, false)
                .map(|s| s.candidate_count)
                .unwrap_or(1);
            facts.push(format!(
                "{n} image{} per job",
                if n == 1 { "" } else { "s" }
            ));
        }
        let images = match &caps {
            Some(c) => images(core, &dir, &settings, c, &draft, &mut facts),
            None => vec![],
        };
        Ok(ChatContext {
            host: ChatHost::Gen,
            kind,
            images,
            typed_prompt: draft.typed_prompt.clone(),
            helpers: fp.sent,
            blend_instruction: None,
            final_prompt: fp.text,
            model: ChatModel {
                name: name.unwrap_or_else(|| model_id.clone()),
                id: model_id,
            },
            controls,
            facts,
        })
    }
}

/// FR-GEN-17: every visible control (not the prompt, seed or image inputs) with its value and
/// its range or allowed values, as the panel shows them.
fn visible_controls(caps: &Capabilities, values: &BTreeMap<String, Value>) -> Vec<ChatControl> {
    let eval = controls::evaluate_controls(caps, values);
    eval.controls
        .into_iter()
        .filter(|c| c.visible)
        .filter(|c| {
            !matches!(
                c.role,
                pf_provider::ControlRole::Prompt | pf_provider::ControlRole::Seed
            )
        })
        .map(|c| {
            let value = c.value.unwrap_or(Value::Null);
            ChatControl {
                name: c.name,
                kind: value_type(&value).to_owned(),
                value,
                min: c.min,
                max: c.max,
                allowed: (!c.allowed.is_empty()).then_some(c.allowed),
            }
        })
        .collect()
}

/// Without capabilities: the values the user set, as they are.
fn raw_controls(values: &BTreeMap<String, Value>) -> Vec<ChatControl> {
    values
        .iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(name, v)| ChatControl {
            name: name.clone(),
            kind: value_type(v).to_owned(),
            value: v.clone(),
            min: None,
            max: None,
            allowed: None,
        })
        .collect()
}

/// "grid 30 × 40 at 70 px per square; sent 1520 × 2032 (51 px per square, model limit)" or
/// "ratio 1:1, long side 1024; sent 1024 × 1024" or "…; sent ratio 2:3 (cells won't be square)".
fn size_fact(caps: Option<&Capabilities>, s: &GenSize) -> String {
    let asked = match s {
        GenSize::Grid { cols, rows, pps } => {
            format!("grid {cols} × {rows} at {pps} px per square")
        }
        GenSize::Ratio { aspect, long_side } => format!("ratio {aspect}, long side {long_side}"),
    };
    let fit = caps
        .and_then(|c| c.output_size.as_ref())
        .and_then(|o| size::fit(o, s).ok());
    let Some(fit) = fit else {
        return format!("size: {asked}");
    };
    let mut sent = match (fit.w, fit.h, &fit.aspect) {
        (Some(w), Some(h), _) => format!("{w} × {h}"),
        (_, _, Some(a)) => format!("ratio {a}"),
        _ => String::new(),
    };
    let mut notes: Vec<String> = vec![];
    if let (GenSize::Grid { .. }, Some(p), Some(_)) = (s, fit.pps, fit.w) {
        notes.push(format!("{p} px per square"));
    }
    notes.extend(fit.notices.iter().map(|n| {
        match n.code {
            "model_limit" => "model limit",
            "model_minimum" => "model minimum",
            "cells_not_square" => "cells won't be square",
            other => other,
        }
        .to_owned()
    }));
    if !notes.is_empty() {
        sent.push_str(&format!(" ({})", notes.join(", ")));
    }
    format!("size: {asked}; sent {sent}")
}

/// The sketch, then the `on` references in send order, as context images. Each item REF's
/// prepare can't send is dropped (turned off for the next attempt) and named in `facts`.
fn images(
    core: &Core,
    dir: &std::path::Path,
    settings: &crate::settings::Settings,
    caps: &Capabilities,
    draft: &GenJobDraft,
    facts: &mut Vec<String>,
) -> Vec<ChatImage> {
    let mut structure = draft.structure.as_ref().map(|s| s.file.clone());
    match (&caps.structure, &structure) {
        (None, _) => {
            structure = None;
            facts.push("this model takes no structural sketch".to_owned());
        }
        (Some(_), None) => facts.push("no sketch".to_owned()),
        (Some(_), Some(f)) => facts.push(format!("sketch: {f}")),
    }
    // REF's reconciled selection: an item that is gone or corrupt is already off; name it.
    let slots = ref_prepare::slots_for(caps, structure.as_deref());
    let resolved = resolve::resolve(
        &core.ctx,
        settings,
        &core.refs.probes,
        dir,
        &slots,
        draft.references.as_ref(),
    );
    for (key, state) in &resolved.lost_on {
        let why = format!("{state:?}").to_lowercase();
        facts.push(format!("reference {} not sent ({why})", file_of(key)));
    }
    let mut refs: DraftReferences = resolved.references;
    for _ in 0..MAX_SKIPS {
        match ref_prepare::prepare_with_structure(
            &core.ctx,
            &core.refs,
            settings,
            dir,
            caps,
            Some(&refs),
            structure.as_deref(),
        ) {
            Ok(prepared) => {
                return prepared
                    .sent
                    .iter()
                    .filter_map(|r| to_image(r, facts))
                    .collect();
            }
            Err(e) if e.code == ErrorCode::ReferenceUnavailable => {
                let key = e.detail["key"].as_str().unwrap_or_default().to_owned();
                facts.push(format!("{} (not sent)", e.message));
                if key.starts_with(STRUCTURE_KEY) {
                    structure = None;
                } else if let Some(i) = refs.items.iter_mut().find(|i| i.key == key && i.on) {
                    i.on = false;
                } else {
                    return vec![];
                }
            }
            Err(e) => {
                facts.push(format!("references not sent: {}", e.message));
                return vec![];
            }
        }
    }
    vec![]
}

/// One prepared item as a context image (FR-PRM-25 JPEG). An image that won't encode is dropped
/// and named.
fn to_image(r: &ref_prepare::SentReference, facts: &mut Vec<String>) -> Option<ChatImage> {
    let file = file_of(&r.row.path).to_owned();
    let (id, role, label) = match r.row.source {
        Source::Structure => (
            format!("structure:{}", r.row.path),
            ChatImageRole::Structure,
            format!("sketch {file}"),
        ),
        Source::Project => (
            format!("ref:{}", r.key),
            ChatImageRole::Reference,
            format!("{file} (project)"),
        ),
        Source::Pack => (
            format!("ref:{}", r.key),
            ChatImageRole::Reference,
            format!(
                "{file} (pack {})",
                r.row.pack_name.as_deref().unwrap_or_default()
            ),
        ),
    };
    match chat_jpeg(&r.bytes) {
        Ok((jpeg, w, h)) => Some(ChatImage {
            id,
            role,
            label,
            w,
            h,
            content_hash: r.row.content_hash.clone(),
            jpeg,
        }),
        Err(e) => {
            facts.push(format!("{label} not sent: {}", e.message));
            None
        }
    }
}

fn file_of(path_or_key: &str) -> &str {
    path_or_key.rsplit(['/', ':']).next().unwrap_or(path_or_key)
}

//! RGN's chat context source (FR-PRM-24, PRM slice 3): a **stub**. It reports the text side of
//! the Region edit job (typed prompt, helpers, blend instruction, final prompt, model, controls,
//! region facts) from the published draft. The `rgn:overlay` image (the sent crop with the mask
//! tinted) and the fit facts ("sent unresampled" / softness) come with PRM slice 5; until then
//! the context has no image. No provider call (FR-PRM-31).

use serde_json::Value;
use ulid::Ulid;

use super::commands::JobDraft;
use super::instruction;
use crate::core::{Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::prompt::context::{
    ChatBlend, ChatContext, ChatContextSource, ChatControl, ChatHost, ChatModel, value_type,
};

/// The Region edit tab's [`ChatContextSource`].
pub struct RgnChatContext;

impl ChatContextSource for RgnChatContext {
    fn host(&self) -> ChatHost {
        ChatHost::Rgn
    }

    /// `input` is the panel's job draft (`rgn_job_estimate`'s `draft`).
    fn build(&self, core: &Core, project_id: Ulid, input: &Value) -> CoreResult<ChatContext> {
        let draft: JobDraft = serde_json::from_value(input.clone())
            .map_err(|e| CoreError::invalid("input", format!("not a region edit draft: {e}")))?;
        draft.validate()?;
        let (kind, blend_text) = {
            let state = core.state();
            let p = state
                .project
                .as_ref()
                .filter(|p| p.manifest.project_id == project_id)
                .ok_or_else(|| {
                    CoreError::new(ErrorCode::NoProject, "that asset project is not open")
                })?;
            let kind = p.manifest.kind;
            (
                kind,
                instruction::instruction_for(&state.settings.region_edit, kind),
            )
        };
        let fp = instruction::final_prompt(
            core,
            &draft.final_prompt,
            draft.blend_instruction,
            draft.prompt_helpers.as_deref(),
        );
        let r = &draft.region;
        Ok(ChatContext {
            host: ChatHost::Rgn,
            kind,
            images: Vec::new(),
            typed_prompt: draft.final_prompt.clone(),
            helpers: fp.sent,
            blend_instruction: Some(ChatBlend {
                on: draft.blend_instruction,
                text: blend_text,
            }),
            final_prompt: fp.text,
            // The display name needs the model list; slice 5 resolves it from the cache.
            model: ChatModel {
                id: draft.model_id.trim().to_owned(),
                name: draft.model_id.trim().to_owned(),
            },
            controls: draft
                .controls
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
                .collect(),
            facts: vec![format!(
                "region {}×{} px, context margin {} px",
                r.inner.w.round(),
                r.inner.h.round(),
                r.margin
            )],
        })
    }
}

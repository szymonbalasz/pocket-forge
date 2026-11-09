//! RGN's chat context source (FR-PRM-24, PRM slice 5; replaces R81's text-only stub). It reads
//! only the Region edit tab's published draft (the input `rgn_job_estimate` takes, FR-PRM-23),
//! the capabilities and model names RGN's commands already fetched this session, and the
//! project's pixels: no RGN command, no provider call (FR-PRM-31).
//!
//! **Image** `rgn:overlay`: the sent crop (FR-RGN-04/05, after the fit) with the mask tinted
//! magenta at 35 % and a 2 px outline, downscaled per FR-PRM-25. **Facts:** the region size, the
//! context margin, "sent unresampled" or the softness factor. With no region or no mask the turn
//! has no image and the facts say so; an overlay that can't be built is dropped and named in
//! the facts (FR-PRM-41).

use std::collections::BTreeMap;

use image::{Rgb, RgbImage};
use serde::Deserialize;
use serde_json::Value;
use ulid::Ulid;

use super::commands::DraftRegion;
use super::fit::{self, RegionFit};
use super::instruction;
use super::pixel_source;
use super::prepare::{sent_mask, sent_region};
use crate::core::Core;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::prompt::context::{
    ChatBlend, ChatContext, ChatContextSource, ChatControl, ChatHost, ChatImage, ChatImageRole,
    ChatModel, chat_jpeg_rgb, value_type,
};
use crate::references::prepare::sha256;

/// The overlay's stable image id (FR-PRM-41).
pub const OVERLAY_ID: &str = "rgn:overlay";
/// FR-PRM-24: the mask tint.
pub const TINT: [u8; 3] = [255, 0, 255];
/// FR-PRM-24: the tint's opacity.
pub const TINT_ALPHA: f32 = 0.35;
/// FR-PRM-24: the outline width in sent px.
pub const OUTLINE_PX: i64 = 2;

/// The Region edit tab's published input: `rgn_job_estimate`'s draft, with the parts that may
/// be missing before a region, mask or model is chosen (FR-PRM-24's "no region yet").
#[derive(Debug, Deserialize)]
struct RgnChatInput {
    #[serde(default)]
    region: Option<DraftRegion>,
    #[serde(default)]
    mask_id: Option<Ulid>,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    final_prompt: String,
    #[serde(default)]
    controls: BTreeMap<String, Value>,
    #[serde(default = "blend_on")]
    blend_instruction: bool,
    #[serde(default)]
    prompt_helpers: Option<Vec<String>>,
}

fn blend_on() -> bool {
    true
}

/// The Region edit tab's [`ChatContextSource`].
pub struct RgnChatContext;

impl ChatContextSource for RgnChatContext {
    fn host(&self) -> ChatHost {
        ChatHost::Rgn
    }

    fn build(&self, core: &Core, project_id: Ulid, input: &Value) -> CoreResult<ChatContext> {
        let draft: RgnChatInput = serde_json::from_value(input.clone())
            .map_err(|e| CoreError::invalid("input", format!("not a region edit draft: {e}")))?;
        let model_id = draft
            .model_id
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let (kind, blend_text, caps, name) = {
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
                state.rgn.caps.get(&model_id).cloned(),
                state.rgn.model_names.get(&model_id).cloned(),
            )
        };
        let fp = instruction::final_prompt(
            core,
            &draft.final_prompt,
            draft.blend_instruction,
            draft.prompt_helpers.as_deref(),
        );
        let mut facts = Vec::new();
        let mut images = Vec::new();
        match (&draft.region, draft.mask_id) {
            (Some(region), Some(mask_id)) => {
                match overlay(core, region, mask_id, caps.as_ref().map(|c| c.input_px)) {
                    Ok((image, fit)) => {
                        facts.push(region_fact(&fit));
                        facts.push(if fit.unresampled() {
                            "sent unresampled".to_owned()
                        } else {
                            format!("the patch is sent {:.1}× softer", 1.0 / fit.scale)
                        });
                        if caps.is_none() {
                            facts.push(
                                "the model's size limits aren't loaded yet; the crop is shown at the region's size"
                                    .to_owned(),
                            );
                        }
                        images.push(image);
                    }
                    Err(e) if e.code == ErrorCode::MaskInvalid => {
                        facts.push(format!(
                            "region {}×{} px, context margin {} px",
                            region.inner.w.round(),
                            region.inner.h.round(),
                            region.margin
                        ));
                        facts.push("no mask painted yet, so no image".to_owned());
                    }
                    Err(e) => facts.push(format!(
                        "the region image couldn't be prepared ({}), so no image",
                        e.code.as_str()
                    )),
                }
            }
            _ => facts.push("no region yet".to_owned()),
        }
        Ok(ChatContext {
            host: ChatHost::Rgn,
            kind,
            images,
            typed_prompt: draft.final_prompt.clone(),
            helpers: fp.sent,
            blend_instruction: Some(ChatBlend {
                on: draft.blend_instruction,
                text: blend_text,
            }),
            final_prompt: fp.text,
            model: ChatModel {
                name: name.unwrap_or_else(|| model_id.clone()),
                id: model_id,
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
            facts,
        })
    }
}

fn region_fact(fit: &RegionFit) -> String {
    format!(
        "region {}×{} px, context margin {} px, sent {}×{} px",
        fit.inner.w, fit.inner.h, fit.margin, fit.sent_w, fit.sent_h
    )
}

/// The overlay image and the fit it was built for.
fn overlay(
    core: &Core,
    region: &DraftRegion,
    mask_id: Ulid,
    input_px: Option<pf_provider::InputPx>,
) -> CoreResult<(ChatImage, RegionFit)> {
    let src = pixel_source(&core.ctx, &core.state())?;
    let (w, h) = src.image_size();
    let fit = fit::region_fit(region.inner, region.margin, w, h, input_px)?;
    let mask = src.mask(mask_id)?;
    let grey = sent_mask(&mask, &fit)?;
    let mut rgb = sent_region(&*src, &fit)?;
    tint(&mut rgb, &grey);
    let (jpeg, jw, jh) = chat_jpeg_rgb(rgb)?;
    Ok((
        ChatImage {
            id: OVERLAY_ID.into(),
            role: ChatImageRole::RegionOverlay,
            label: "region crop with the mask".into(),
            w: jw,
            h: jh,
            content_hash: sha256(&jpeg),
            jpeg,
        },
        fit,
    ))
}

/// FR-PRM-24: the mask (grey, 255 = edit, same size as `rgb`) tinted magenta at 35 %, with a
/// solid 2 px outline on its inside edge.
pub fn tint(rgb: &mut RgbImage, mask: &[u8]) {
    let (w, h) = (i64::from(rgb.width()), i64::from(rgb.height()));
    let on =
        |x: i64, y: i64| x >= 0 && y >= 0 && x < w && y < h && mask[(y * w + x) as usize] >= 128;
    for y in 0..h {
        for x in 0..w {
            let m = mask[(y * w + x) as usize];
            if m == 0 {
                continue;
            }
            let edge = on(x, y)
                && (-OUTLINE_PX..=OUTLINE_PX).any(|dy| {
                    (-OUTLINE_PX..=OUTLINE_PX).any(|dx| (dx != 0 || dy != 0) && !on(x + dx, y + dy))
                });
            let px = rgb.get_pixel_mut(x as u32, y as u32);
            if edge {
                *px = Rgb(TINT);
                continue;
            }
            let a = TINT_ALPHA * f32::from(m) / 255.0;
            for c in 0..3 {
                let v = f32::from(px[c]) * (1.0 - a) + f32::from(TINT[c]) * a;
                px[c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prm_24_tint_and_outline() {
        let mut rgb = RgbImage::from_pixel(8, 8, Rgb([100, 100, 100]));
        let mut mask = vec![0u8; 64];
        for y in 1..7 {
            for x in 1..7 {
                mask[y * 8 + x] = 255;
            }
        }
        tint(&mut rgb, &mask);
        assert_eq!(rgb.get_pixel(0, 0).0, [100, 100, 100], "outside: untouched");
        assert_eq!(rgb.get_pixel(1, 1).0, TINT, "the outline");
        assert_eq!(rgb.get_pixel(2, 2).0, TINT, "2 px wide");
        // 100·0.65 + 255·0.35 = 154.25; 100·0.65 = 65.
        assert_eq!(rgb.get_pixel(3, 3).0, [154, 65, 154], "inside: tinted 35 %");
    }
}

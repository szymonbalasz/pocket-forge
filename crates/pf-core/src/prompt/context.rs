//! The prompt chat's context (FR-PRM-41): what a chat turn tells the backend about the visible
//! tab's job, built by that tab's host through [`ChatContextSource`]. Read-only; no provider call
//! (FR-PRM-31). RGN implements it in `region_edit::chat` (FR-PRM-24), GEN in its slice 6
//! (FR-PRM-42). The chat core (PRM slice 5) calls it at send time.

use serde::Serialize;
use serde_json::Value;
use ulid::Ulid;

use super::SentHelper;
use crate::core::Core;
use crate::error::CoreResult;
use crate::manifest::Kind;

/// The tab a turn is sent from (FR-PRM-43).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatHost {
    Rgn,
    Gen,
}

/// What an image is to the chat (FR-PRM-41).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatImageRole {
    RegionOverlay,
    Structure,
    Reference,
}

/// One context image. `id` is a stable identity (`rgn:overlay`, `structure:<path>`,
/// `ref:<item key>`); the bytes are never serialised (the thread stores only the hash).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatImage {
    pub id: String,
    pub role: ChatImageRole,
    pub label: String,
    pub w: u32,
    pub h: u32,
    pub content_hash: String,
    #[serde(skip)]
    pub jpeg: Vec<u8>,
}

/// The blend instruction as the context reports it (RGN only; GEN has `null`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatBlend {
    pub on: bool,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatModel {
    pub id: String,
    pub name: String,
}

/// A control value as the context reports it. `min` / `max` / `allowed` come from the model's
/// capabilities where the host has them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatControl {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<Value>>,
}

/// FR-PRM-41: the context of one chat turn. `final_prompt` is exactly what submit would send at
/// that moment (FR-PRM-06).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatContext {
    pub host: ChatHost,
    pub kind: Kind,
    pub images: Vec<ChatImage>,
    pub typed_prompt: String,
    pub helpers: Vec<SentHelper>,
    pub blend_instruction: Option<ChatBlend>,
    pub final_prompt: String,
    pub model: ChatModel,
    pub controls: Vec<ChatControl>,
    pub facts: Vec<String>,
}

/// FR-PRM-41: implemented per host. `input` is the host's published draft input (the same
/// inputs its estimate takes, FR-PRM-23). A failure of one image drops only that image and adds
/// it to the facts; an invalid input is `invalid_input`, a project mismatch `no_project`.
pub trait ChatContextSource {
    fn host(&self) -> ChatHost;
    fn build(&self, core: &Core, project_id: Ulid, input: &Value) -> CoreResult<ChatContext>;
}

/// The JSON type name of a control value (`number`, `string`, `boolean`, …).
pub fn value_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ---------------------------------------------------------------- image limits (FR-PRM-25)

/// Long side of a context image, after Lanczos3 (FR-PRM-25).
pub const CHAT_IMAGE_MAX_SIDE: u32 = 1568;
/// JPEG quality of a context image.
pub const CHAT_IMAGE_QUALITY: u8 = 85;
/// Images one turn carries at most.
pub const CHAT_MAX_IMAGES: usize = 6;
/// Encoded bytes one turn carries at most.
pub const CHAT_MAX_BYTES: usize = 4 * 1024 * 1024;

/// FR-PRM-25: an image as a context image: decoded (alpha over the flatten colour, FR-GEN-12),
/// downscaled with Lanczos3 to a long side ≤ 1568 px, encoded as JPEG q85. Returns the JPEG and
/// its size.
pub fn chat_jpeg(bytes: &[u8]) -> CoreResult<(Vec<u8>, u32, u32)> {
    use image::ImageEncoder;
    let img = image::load_from_memory(bytes).map_err(|e| {
        crate::error::CoreError::new(
            crate::error::ErrorCode::Corrupt,
            format!("the image won't decode: {e}"),
        )
    })?;
    let rgb = pf_imaging::to_rgb8_flattened(img);
    let (w, h) = rgb.dimensions();
    let long = w.max(h);
    let rgb = if long > CHAT_IMAGE_MAX_SIDE {
        let scale = f64::from(CHAT_IMAGE_MAX_SIDE) / f64::from(long);
        let nw = ((f64::from(w) * scale).round() as u32).max(1);
        let nh = ((f64::from(h) * scale).round() as u32).max(1);
        pf_imaging::resize_lanczos3(&rgb, nw, nh)
            .map_err(|e| crate::error::CoreError::internal(format!("resizing: {e}")))?
    } else {
        rgb
    };
    let (w, h) = rgb.dimensions();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, CHAT_IMAGE_QUALITY)
        .write_image(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| crate::error::CoreError::internal(format!("encoding: {e}")))?;
    Ok((out, w, h))
}

/// FR-PRM-25: keeps `images` in order up to 6 images and 4 MiB encoded; the tail is dropped and
/// returned, so the context strip and the facts can name it. The chat core calls it after its
/// per-session "already received" filter.
pub fn limit_images(images: &mut Vec<ChatImage>) -> Vec<ChatImage> {
    let mut bytes = 0usize;
    let mut keep = images.len();
    for (i, img) in images.iter().enumerate() {
        if i >= CHAT_MAX_IMAGES || bytes + img.jpeg.len() > CHAT_MAX_BYTES {
            keep = i;
            break;
        }
        bytes += img.jpeg.len();
    }
    images.split_off(keep)
}

/// The fact naming dropped images ("not sent (turn limit): a.png, b.png").
pub fn dropped_fact(dropped: &[ChatImage]) -> Option<String> {
    (!dropped.is_empty()).then(|| {
        let names: Vec<&str> = dropped.iter().map(|i| i.label.as_str()).collect();
        format!("not sent (turn limit): {}", names.join(", "))
    })
}

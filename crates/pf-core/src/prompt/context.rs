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

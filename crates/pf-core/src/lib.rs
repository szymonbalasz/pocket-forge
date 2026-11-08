//! Pocket Forge domain core (AGENTS §9, ADR 0002).
//!
//! Every capability is a core command ([`commands`]); the Tauri layer and the future MCP
//! endpoint (#36) only adapt them. Seams (IDs, clock, randomness, provider, filesystem
//! attributes, faults, events) come from the [`context::Context`]. No Tauri dependency.

mod b64;
pub mod commands;
pub mod context;
mod core;
pub mod create;
pub mod decoded;
pub mod error;
pub mod fsx;
pub mod images;
pub mod job;
pub mod library;
pub mod manifest;
pub mod naming;
pub mod project;
mod provider_commands;
pub mod settings;
pub mod thumb;
pub mod tiles;
pub mod time;

#[cfg(test)]
mod test_support;

pub use crate::core::{Command, Core, State, Validate, dispatch_json};
pub use context::Context;
pub use error::{CoreError, CoreResult, ErrorCode};
pub use pf_provider as provider;

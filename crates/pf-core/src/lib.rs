//! Pocket Forge domain core (AGENTS §9, ADR 0002).
//!
//! Every capability is a core command ([`commands`]); the Tauri layer and the future MCP
//! endpoint (#36) only adapt them. Seams (IDs, clock, randomness, provider, filesystem
//! attributes, faults, events) come from the [`context::Context`]. No Tauri dependency.

pub mod commands;
pub mod context;
mod core;
pub mod error;
pub mod fsx;
pub mod images;
pub mod import;
pub mod library;
pub mod manifest;
pub mod project;
mod provider_commands;
pub mod recent;
pub mod settings;
pub mod time;

pub use crate::core::{Command, Core, State, Validate, dispatch_json};
pub use context::Context;
pub use error::{CoreError, CoreResult, ErrorCode};
pub use pf_provider as provider;

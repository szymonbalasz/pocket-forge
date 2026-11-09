//! New-asset generation (GEN, [new-asset-generation spec](../../../../docs/specs/new-asset-generation.md)).
//!
//! Slice 3, the core: the Size block's fit ([`size`]), controls, estimate, submit through RGN's
//! job runner and the retry draft ([`commands`]). A generate job has no region, fit, mask or
//! input files (FR-GEN-22); its sketch goes through REF's prepare, first in its slot (FR-GEN-15).
//! The project side (no-image projects, the base layer, the pick commit) is PRJ's
//! ([`crate::project::store`], slice 2). Commands are re-exported from [`crate::commands`].
//! Slice 6: the prompt chat's context for the New image tab ([`chat`], FR-GEN-17).

pub mod chat;
pub mod commands;
pub mod size;

use std::collections::BTreeMap;

use pf_provider::Capabilities;

/// GEN's memory-only state (per app session): what the New image tab's commands already fetched,
/// so the chat context (FR-GEN-17) is built without a provider call.
#[derive(Debug, Default)]
pub struct GenState {
    /// The `generate` capabilities by model id, as last fetched.
    pub caps: BTreeMap<String, Capabilities>,
    /// Display names of the `generate` models (`provider_list_models{generate}`).
    pub model_names: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests;

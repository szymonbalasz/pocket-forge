//! New-asset generation (GEN, [new-asset-generation spec](../../../../docs/specs/new-asset-generation.md)).
//!
//! Slice 3, the core: the Size block's fit ([`size`]), controls, estimate, submit through RGN's
//! job runner and the retry draft ([`commands`]). A generate job has no region, fit, mask or
//! input files (FR-GEN-22); its sketch goes through REF's prepare, first in its slot (FR-GEN-15).
//! The project side (no-image projects, the base layer, the pick commit) is PRJ's
//! ([`crate::project::store`], slice 2). Commands are re-exported from [`crate::commands`].

pub mod commands;
pub mod size;

#[cfg(test)]
mod tests;

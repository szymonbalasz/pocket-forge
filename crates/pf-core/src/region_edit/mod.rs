//! Region edit (RGN, [region-edit spec](../../../../docs/specs/region-edit.md)).
//!
//! Slices 1–2: region fit, the mask pipeline and region extraction ([`fit`], [`prepare`]);
//! capability rules, the estimate and the spend guard ([`controls`], [`estimate`], [`spend`]).
//! Slice 3: submit, cancel, retry draft, dismiss ([`jobs`]) and the job runner ([`runner`]),
//! with the interim write spool ([`spool`]). Slices 4–5: candidate preview, accept / re-pick,
//! star, discard, re-tune and the iteration list ([`candidates`]) over the patch bake
//! ([`blend`]). Slice 7 (REF slice 6): references in estimate, submit, the job record and Retry
//! through REF's `prepare_for_send` and `retry`. Commands are re-exported from [`crate::commands`].
//!
//! RGN reads pixels through [`pixels::PixelSource`] (PRJ's decoded original and mask store) and
//! writes only through PRJ's store API (R4).

pub mod blend;
pub mod candidates;
pub mod commands;
pub mod controls;
pub mod estimate;
pub mod fit;
pub mod instruction;
pub mod jobs;
pub mod pixels;
pub mod prepare;
pub mod runner;
pub mod spend;
pub mod spool;

#[cfg(test)]
mod job_tests;
#[cfg(test)]
mod ref_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::core::State;
use crate::error::{CoreError, CoreResult, ErrorCode};

/// Event names (R21, spec §9).
pub const EVENT_SPEND: &str = "rgn:spend";
/// The blend instruction text changed (#55).
pub const EVENT_BLEND_INSTRUCTION: &str = "rgn:blend-instruction";

/// RGN's in-memory state (spec §8 "memory only"): session spend, estimates, the interim
/// decoded original. Reset per app session.
#[derive(Default)]
pub struct RgnState {
    pub ledger: spend::SpendLedger,
    pub estimates: estimate::EstimateCache,
    /// The job runner (FR-RGN-35).
    pub runner: runner::Runner,
    /// Test and fixture seam: replaces the open project's pixels.
    pub source_override: Option<Arc<dyn pixels::PixelSource>>,
}

impl std::fmt::Debug for RgnState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RgnState")
            .field("spend", &self.ledger.total())
            .field("source_override", &self.source_override.is_some())
            .finish_non_exhaustive()
    }
}

/// The open project's pixels, owned so the caller can drop the state lock.
pub fn pixel_source(
    ctx: &crate::context::Context,
    state: &State,
) -> CoreResult<Arc<dyn pixels::PixelSource>> {
    if let Some(s) = &state.rgn.source_override {
        return Ok(s.clone());
    }
    let p = state
        .project
        .as_ref()
        .ok_or_else(|| CoreError::new(ErrorCode::NoProject, "no asset project is open"))?;
    // FR-GEN-03: no region edit before the project has an image.
    p.require_image()?;
    Ok(Arc::new(pixels::OpenProjectSource::new(ctx, p)))
}

/// The id given must be the open project (a late call from a closed project fails).
pub fn check_project(
    src: &dyn pixels::PixelSource,
    project_id: Option<ulid::Ulid>,
) -> CoreResult<()> {
    match project_id {
        Some(id) if id != src.project_id() => Err(CoreError::new(
            ErrorCode::UnknownId,
            "that asset project is not open",
        )
        .with_field("project_id")),
        _ => Ok(()),
    }
}

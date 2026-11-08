//! Region edit (RGN, [region-edit spec](../../../../docs/specs/region-edit.md)).
//!
//! Slice 1: region fit, the mask pipeline and region extraction ([`fit`], [`prepare`]). The
//! commands are in [`commands`] and re-exported from [`crate::commands`].
//!
//! RGN reads the project through [`pixels::PixelSource`] so it doesn't depend on PRJ internals.

pub mod commands;
pub mod fit;
pub mod pixels;
pub mod prepare;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::core::State;
use crate::error::{CoreError, CoreResult, ErrorCode};

/// RGN's in-memory state (spec §8 "memory only"): the interim decoded original. Reset per app
/// session.
#[derive(Default)]
pub struct RgnState {
    pub(crate) original_cache: pixels::OriginalCache,
    /// Test and fixture seam: replaces the open project's pixels.
    pub source_override: Option<Arc<dyn pixels::PixelSource>>,
}

impl std::fmt::Debug for RgnState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RgnState")
            .field("source_override", &self.source_override.is_some())
            .finish_non_exhaustive()
    }
}

/// The open project's pixels, owned so the caller can drop the state lock.
pub fn pixel_source(state: &State) -> CoreResult<Arc<dyn pixels::PixelSource>> {
    if let Some(s) = &state.rgn.source_override {
        return Ok(s.clone());
    }
    let p = state
        .project
        .as_ref()
        .ok_or_else(|| CoreError::new(ErrorCode::NoProject, "no asset project is open"))?;
    Ok(Arc::new(pixels::OpenProjectSource::new(
        p,
        state.rgn.original_cache.clone(),
    )))
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

//! Image operations for Pocket Forge (ADR 0004).
//!
//! The skeleton holds format sniffing and header probing, which import uses to validate files
//! (FR-PRJ-06). Tile pyramid, region extract, blend, flatten and encode land in their own modules.
//! This crate knows nothing about asset projects: it works on bytes, paths and pixel buffers.

mod compose;
mod probe;
pub mod pyramid;
mod rgb_source;
pub mod thumbnail;

pub use compose::{
    COMPOSE_VERSION, ComposeError, Layer, LayerStack, PixelRect, Placement, STRIP_H, Strip,
    compose_rect, flatten_full, flatten_strips,
};
pub use probe::{ImageFormat, ImageInfo, MAX_SIDE, ProbeError, probe_bytes, probe_file, sniff};

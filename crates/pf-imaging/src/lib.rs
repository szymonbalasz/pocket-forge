//! Image operations for Pocket Forge (ADR 0004).
//!
//! The skeleton holds format sniffing and header probing, which import uses to validate files
//! (FR-PRJ-06). Tile pyramid, region extract, blend, flatten and encode land in their own modules.
//! This crate knows nothing about asset projects: it works on bytes, paths and pixel buffers.
//!
//! Export (export spec): `compose` is the only compositor; `encode`, `webp`, `resize` and
//! `sizing` are the export encoders, rescale and size maths. `webp` is the only module allowed
//! `unsafe` (libwebp FFI).

mod compose;
mod encode;
mod probe;
pub mod pyramid;
pub mod reference;
mod region;
mod resize;
mod rgb_source;
mod sizing;
#[cfg(test)]
mod test_support;
pub mod thumbnail;
mod webp;

pub use compose::{
    COMPOSE_VERSION, ComposeError, Layer, LayerStack, PixelRect, Placement, STRIP_H, Strip,
    StripFlattener, compose_rect, flatten_full, flatten_strips,
};
pub use encode::{
    EncodeError, ImageRows, JPEG_MAX_SIDE, JPEG_QUALITY, Phase, Progress, RowSource, encode_jpeg,
    encode_png,
};
pub use probe::{
    ImageFormat, ImageInfo, MAX_SIDE, ProbeError, check_complete, has_image_extension, probe_bytes,
    probe_file, probe_file_complete, sniff,
};
pub use region::{
    GreyDecodeError, box_resample_grey, decode_grey8_png, encode_grey8_png, encode_rgb8_png,
    pad_grey,
};
pub use resize::resize_lanczos3;
pub use rgb_source::decode_rgb8;
pub use sizing::{
    GridInput, InvalidSetting, Limit, MAX_CELLS, MAX_OUTPUT_SIDE, MAX_PX_PER_SQUARE, MAX_SCALE,
    MIN_PX_PER_SQUARE, SizeWarning, Sizing, format_unavailable, plan_size, source_px_per_square,
};
pub use webp::{
    LibWebp, WEBP_MAX_SIDE, WEBP_QUALITY, WebpEncoder, WebpMode, WebpOutput, encode_webp,
    encode_webp_with,
};

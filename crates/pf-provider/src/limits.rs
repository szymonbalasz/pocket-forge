//! The local limit backstop (FR-PRV-13, R20): RGN pre-checks with `input_too_large`; PRV
//! checks again in `estimate` and `submit` before any network call.

use crate::pixels::png_size;
use crate::{Capabilities, JobRequest, ProviderError, ProviderResult};

fn invalid(field: &str) -> ProviderError {
    ProviderError::InvalidRequest {
        field: field.into(),
    }
}

/// The sent image must fit `input_px{min,max,multiple_of}` and `max_bytes`; the canonical
/// mask must be exactly the sent image's size (FR-PRV-12). Returns the image size.
pub fn check(caps: &Capabilities, request: &JobRequest) -> ProviderResult<(u32, u32)> {
    let (w, h) = png_size(&request.region_png).ok_or_else(|| invalid("image"))?;
    let px = caps.input_px;
    let fits = |s: u32| s >= px.min && s <= px.max && s.is_multiple_of(px.multiple_of.max(1));
    if !fits(w) || !fits(h) {
        return Err(invalid("image"));
    }
    if request.region_png.len() as u64 > caps.max_bytes {
        return Err(invalid("image"));
    }
    if png_size(&request.mask_png) != Some((w, h)) {
        return Err(invalid("mask"));
    }
    Ok((w, h))
}

#[allow(dead_code)] // mask conversion lands in PRV slice 4
/// A converted file over `max_bytes` → `invalid_request(field)`.
pub fn check_converted(caps: &Capabilities, bytes: &[u8], field: &str) -> ProviderResult<()> {
    if bytes.len() as u64 > caps.max_bytes {
        return Err(invalid(field));
    }
    Ok(())
}

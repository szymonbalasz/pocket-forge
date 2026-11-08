//! What a job sends (FR-RGN-03/04/06, R1, R20): the region pixels composed from the current
//! layer stack and the mask padded to the outer rect, both resampled to the fitted size, encoded
//! as PNG and checked against the model's limits before any provider call.

use pf_imaging::{GreyDecodeError, compose_rect, resize_lanczos3};
use pf_provider::Capabilities;

use super::fit::{RectPx, RegionFit};
use super::pixels::{MaskRecord, PixelSource};
use crate::error::{CoreError, CoreResult, ErrorCode};

/// The sent images.
#[derive(Debug, Clone, PartialEq)]
pub struct SentImages {
    /// RGB8 PNG of the outer rect, `sent_w × sent_h`.
    pub region_png: Vec<u8>,
    /// Canonical mask (8-bit grey, 255 = edit), `sent_w × sent_h`, before polarity conversion.
    pub mask_png: Vec<u8>,
}

fn mask_invalid(reason: &str, message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::MaskInvalid, message)
        .with_detail(serde_json::json!({ "reason": reason }))
}

fn input_too_large(field: &str, limit: u64, message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorCode::InputTooLarge, message)
        .with_field(field.to_owned())
        .with_detail(serde_json::json!({ "field": field, "limit": limit }))
}

fn imaging(e: impl std::fmt::Display) -> CoreError {
    CoreError::internal(format!("image processing: {e}"))
}

/// FR-RGN-03 (R1): the stored inner-rect mask, padded with 0 to the outer rect (only pixels
/// inside the current region are kept, so context pixels are always 0), box-resampled to the
/// sent size, then validated. Returns `sent_w × sent_h` grey bytes.
pub fn sent_mask(mask: &MaskRecord, fit: &RegionFit) -> CoreResult<Vec<u8>> {
    let (w, h, bytes) = pf_imaging::decode_grey8_png(&mask.png).map_err(|e| match e {
        GreyDecodeError::Depth => mask_invalid("depth", "the mask is not 8-bit greyscale"),
        GreyDecodeError::Corrupt(m) => {
            CoreError::new(ErrorCode::Corrupt, format!("the mask can't be read: {m}"))
        }
    })?;
    if (w, h) != (mask.rect.w, mask.rect.h) {
        return Err(mask_invalid(
            "size",
            format!(
                "the mask is {w}×{h} but its region is {}×{}",
                mask.rect.w, mask.rect.h
            ),
        ));
    }
    let o = fit.outer;
    let keep = RectPx {
        x: fit.inner.x - o.x,
        y: fit.inner.y - o.y,
        w: fit.inner.w,
        h: fit.inner.h,
    };
    let padded = pf_imaging::pad_grey(
        &bytes,
        w,
        h,
        i64::from(mask.rect.x) - i64::from(o.x),
        i64::from(mask.rect.y) - i64::from(o.y),
        o.w,
        o.h,
        keep.to_pixel_rect(),
    )
    .map_err(imaging)?;
    let sent = pf_imaging::box_resample_grey(&padded, o.w, o.h, fit.sent_w, fit.sent_h)
        .map_err(imaging)?;
    if sent.len() != fit.sent_w as usize * fit.sent_h as usize {
        return Err(mask_invalid("size", "the mask doesn't match the sent size"));
    }
    if !sent.iter().any(|&v| v > 0) {
        return Err(mask_invalid(
            "empty",
            "paint the area to change inside the region first",
        ));
    }
    Ok(sent)
}

/// FR-RGN-04: `compose_rect(current layer stack, outer rect)`, Lanczos3-resampled to the sent
/// size when the fit scales (FR-RGN-05).
pub fn sent_region(src: &dyn PixelSource, fit: &RegionFit) -> CoreResult<image::RgbImage> {
    let stack = src.layer_stack()?;
    let pixels = compose_rect(&stack, fit.outer.to_pixel_rect()).map_err(imaging)?;
    if fit.unresampled() {
        return Ok(pixels);
    }
    resize_lanczos3(&pixels, fit.sent_w, fit.sent_h).map_err(imaging)
}

/// FR-RGN-06 (R20): the sent px and encoded bytes against the capabilities.
pub fn check_limits(caps: &Capabilities, fit: &RegionFit, images: &SentImages) -> CoreResult<()> {
    let px = caps.input_px;
    let m = px.multiple_of.max(1);
    for (side, v) in [("width", fit.sent_w), ("height", fit.sent_h)] {
        if v > px.max {
            return Err(input_too_large(
                "image",
                u64::from(px.max),
                format!("the sent {side} {v} px is over the model's {} px", px.max),
            ));
        }
        if v < px.min || !v.is_multiple_of(m) {
            return Err(CoreError::invalid(
                "region",
                format!("the sent {side} {v} px doesn't fit the model's input sizes"),
            ));
        }
    }
    for (field, bytes) in [("image", &images.region_png), ("mask", &images.mask_png)] {
        if bytes.len() as u64 > caps.max_bytes {
            return Err(input_too_large(
                field,
                caps.max_bytes,
                format!(
                    "the {field} is {} bytes, over the model's {} bytes",
                    bytes.len(),
                    caps.max_bytes
                ),
            ));
        }
    }
    Ok(())
}

/// The whole pipeline: mask first (cheap, most likely to fail), then region pixels, encode, and
/// the limit pre-check.
pub fn prepare(
    src: &dyn PixelSource,
    fit: &RegionFit,
    mask: &MaskRecord,
    caps: &Capabilities,
) -> CoreResult<SentImages> {
    let mask_bytes = sent_mask(mask, fit)?;
    let region = sent_region(src, fit)?;
    let images = SentImages {
        region_png: pf_imaging::encode_rgb8_png(&region).map_err(imaging)?,
        mask_png: pf_imaging::encode_grey8_png(fit.sent_w, fit.sent_h, &mask_bytes)
            .map_err(imaging)?,
    };
    check_limits(caps, fit, &images)?;
    Ok(images)
}

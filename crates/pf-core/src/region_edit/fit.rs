//! Region fit (FR-RGN-01/02/05, R1, R20): snap and clamp the region, add the context margin, and
//! fit the outer rect to the model's `input_px{min,max,multiple_of}`. Pure.

use pf_provider::InputPx;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ErrorCode};

/// Smallest region side in image px (FR-RGN-01).
pub const MIN_REGION_SIDE: u32 = 16;
/// Default and largest context margin (FR-RGN-02, #40).
pub const DEFAULT_MARGIN: u32 = 128;
pub const MAX_MARGIN: u32 = 512;
/// Largest outer-rect side before fitting (FR-RGN-02, OQ-CNV-04).
pub const MAX_OUTER_SIDE: u32 = 4096;

/// An integer image-px rect (`{x, y, w, h}`, manifest-schema §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RectPx {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl RectPx {
    pub fn right(&self) -> u32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> u32 {
        self.y + self.h
    }

    pub fn to_pixel_rect(self) -> pf_imaging::PixelRect {
        pf_imaging::PixelRect::new(self.x, self.y, self.w, self.h)
    }
}

/// A rect as EDT's region tool emits it (image px, not yet snapped).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RectIn {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl RectIn {
    pub fn is_finite(&self) -> bool {
        [self.x, self.y, self.w, self.h]
            .iter()
            .all(|v| v.is_finite() && v.abs() < 1e9)
    }
}

impl From<RectPx> for RectIn {
    fn from(r: RectPx) -> Self {
        RectIn {
            x: f64::from(r.x),
            y: f64::from(r.y),
            w: f64::from(r.w),
            h: f64::from(r.h),
        }
    }
}

/// The validated region and what is sent (`rgn_region_fit` result, spec §9).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RegionFit {
    pub inner: RectPx,
    pub outer: RectPx,
    pub margin: u32,
    pub sent_w: u32,
    pub sent_h: u32,
    /// Sent ÷ outer along the outer rect's longest side; 1 = that side is sent unresampled.
    /// The panel's "patch N.N× softer" is `1 / scale` (outer ÷ sent).
    pub scale: f64,
}

impl RegionFit {
    /// True when the outer rect is sent pixel for pixel ("sent unresampled", FR-RGN-05).
    pub fn unresampled(&self) -> bool {
        (self.sent_w, self.sent_h) == (self.outer.w, self.outer.h)
    }
}

/// FR-RGN-01: snaps `inner` to integer px and clamps it to the image.
pub fn snap_region(inner: RectIn, image_w: u32, image_h: u32) -> CoreResult<RectPx> {
    let clamp = |v: f64, max: u32| v.round().clamp(0.0, f64::from(max)) as u32;
    let (x0, x1) = (clamp(inner.x, image_w), clamp(inner.x + inner.w, image_w));
    let (y0, y1) = (clamp(inner.y, image_h), clamp(inner.y + inner.h, image_h));
    let (x0, x1) = (x0.min(x1), x0.max(x1));
    let (y0, y1) = (y0.min(y1), y0.max(y1));
    if x1 <= x0 || y1 <= y0 {
        return Err(CoreError::new(
            ErrorCode::OutOfBounds,
            "the region is outside the image",
        ));
    }
    let r = RectPx {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    };
    if r.w < MIN_REGION_SIDE || r.h < MIN_REGION_SIDE {
        return Err(CoreError::new(
            ErrorCode::RegionTooSmall,
            format!("a region is at least {MIN_REGION_SIDE} × {MIN_REGION_SIDE} px"),
        )
        .with_detail(serde_json::json!({ "min": MIN_REGION_SIDE })));
    }
    Ok(r)
}

/// FR-RGN-02: the outer rect = region + margin, clamped to the image (may be asymmetric).
pub fn outer_rect(inner: RectPx, margin: u32, image_w: u32, image_h: u32) -> CoreResult<RectPx> {
    let x0 = inner.x.saturating_sub(margin);
    let y0 = inner.y.saturating_sub(margin);
    let x1 = inner.right().saturating_add(margin).min(image_w);
    let y1 = inner.bottom().saturating_add(margin).min(image_h);
    let outer = RectPx {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    };
    if outer.w > MAX_OUTER_SIDE || outer.h > MAX_OUTER_SIDE {
        return Err(CoreError::new(
            ErrorCode::RegionTooLarge,
            format!("the region plus its context margin is over {MAX_OUTER_SIDE} px"),
        )
        .with_detail(serde_json::json!({ "limit": MAX_OUTER_SIDE })));
    }
    Ok(outer)
}

fn floor_to(v: u32, m: u32) -> u32 {
    v / m * m
}

fn ceil_to(v: u32, m: u32) -> u32 {
    v.div_ceil(m) * m
}

/// Grows `[start, start + len)` toward `target` within `[0, limit)`, half on each side where
/// there is room. Returns the new `(start, len)`.
fn grow(start: u32, len: u32, target: u32, limit: u32) -> (u32, u32) {
    if target <= len {
        return (start, len);
    }
    let need = target - len;
    let room_lo = start;
    let room_hi = limit.saturating_sub(start + len);
    let mut lo = (need / 2).min(room_lo);
    let hi = (need - lo).min(room_hi);
    lo = (need - hi).min(room_lo);
    (start - lo, len + lo + hi)
}

/// One axis of the fit: returns `(outer start, outer len, sent len)`.
fn fit_axis(start: u32, len: u32, limit: u32, px: InputPx, scale: Option<f64>) -> (u32, u32, u32) {
    let m = px.multiple_of.max(1);
    let lo = ceil_to(px.min.max(1), m);
    let hi = floor_to(px.max, m).max(lo);
    match scale {
        // Downscale branch: grow a side that would land under `min`, then scale and round down.
        Some(s) => {
            let need = (f64::from(lo) / s).ceil() as u32;
            let (start, len) = if ((f64::from(len) * s) as u32) < lo {
                grow(start, len, need, limit)
            } else {
                (start, len)
            };
            let sent = floor_to((f64::from(len) * s) as u32, m).clamp(lo, hi);
            (start, len, sent)
        }
        // Grow the context margin until the side is valid; with no room, resample to the
        // nearest valid size.
        None => {
            let target = ceil_to(len, m).clamp(lo, hi);
            let (start, len) = grow(start, len, target, limit);
            if len == target {
                return (start, len, target);
            }
            let down = floor_to(len, m).clamp(lo, hi);
            let up = ceil_to(len, m).clamp(lo, hi);
            let sent = if len.abs_diff(down) <= len.abs_diff(up) {
                down
            } else {
                up
            };
            (start, len, sent)
        }
    }
}

/// FR-RGN-05: fits the outer rect to `input_px`. Returns the (possibly grown) outer rect and the
/// sent size.
pub fn fit_to_input(outer: RectPx, image_w: u32, image_h: u32, px: InputPx) -> (RectPx, u32, u32) {
    let longest = outer.w.max(outer.h);
    let scale = (longest > px.max).then(|| f64::from(px.max) / f64::from(longest));
    let (x, w, sent_w) = fit_axis(outer.x, outer.w, image_w, px, scale);
    let (y, h, sent_h) = fit_axis(outer.y, outer.h, image_h, px, scale);
    (RectPx { x, y, w, h }, sent_w, sent_h)
}

/// The whole fit (FR-RGN-01/02/05). Without `input_px` (no model picked yet) the outer rect is
/// sent as is.
pub fn region_fit(
    inner: RectIn,
    margin: u32,
    image_w: u32,
    image_h: u32,
    input_px: Option<InputPx>,
) -> CoreResult<RegionFit> {
    if margin > MAX_MARGIN {
        return Err(CoreError::invalid(
            "margin",
            format!("the context margin is 0 to {MAX_MARGIN} px"),
        ));
    }
    let inner = snap_region(inner, image_w, image_h)?;
    let outer = outer_rect(inner, margin, image_w, image_h)?;
    let (outer, sent_w, sent_h) = match input_px {
        Some(px) => fit_to_input(outer, image_w, image_h, px),
        None => (outer, outer.w, outer.h),
    };
    let scale = if outer.w >= outer.h {
        f64::from(sent_w) / f64::from(outer.w)
    } else {
        f64::from(sent_h) / f64::from(outer.h)
    };
    Ok(RegionFit {
        inner,
        outer,
        margin,
        sent_w,
        sent_h,
        scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE_PX: InputPx = InputPx {
        min: 64,
        max: 4096,
        multiple_of: 8,
    };
    const GPT_PX: InputPx = InputPx {
        min: 256,
        max: 3840,
        multiple_of: 16,
    };

    fn rect(x: f64, y: f64, w: f64, h: f64) -> RectIn {
        RectIn { x, y, w, h }
    }

    fn code(e: &CoreError) -> &'static str {
        e.code.as_str()
    }

    #[test]
    fn fr_rgn_01_snaps_and_clamps_to_the_image() {
        let r = snap_region(rect(10.4, 19.6, 100.2, 50.0), 200, 100).unwrap();
        assert_eq!(
            r,
            RectPx {
                x: 10,
                y: 20,
                w: 101,
                h: 50
            }
        );
        let r = snap_region(rect(-50.0, 80.0, 100.0, 100.0), 200, 100).unwrap();
        assert_eq!(
            r,
            RectPx {
                x: 0,
                y: 80,
                w: 50,
                h: 20
            }
        );
        let e = snap_region(rect(300.0, 0.0, 50.0, 50.0), 200, 100).unwrap_err();
        assert_eq!(code(&e), "out_of_bounds");
        let e = snap_region(rect(0.0, 0.0, 15.4, 100.0), 200, 100).unwrap_err();
        assert_eq!(code(&e), "region_too_small");
        let e = snap_region(rect(190.0, 0.0, 50.0, 50.0), 200, 100).unwrap_err();
        assert_eq!(code(&e), "region_too_small", "clamped to 10 px wide");
        assert!(snap_region(rect(0.0, 0.0, 16.0, 16.0), 200, 100).is_ok());
    }

    #[test]
    fn fr_rgn_02_margin_is_clamped_and_outer_capped_at_4096() {
        let inner = RectPx {
            x: 50,
            y: 10,
            w: 100,
            h: 100,
        };
        let o = outer_rect(inner, 128, 1000, 1000).unwrap();
        assert_eq!(
            o,
            RectPx {
                x: 0,
                y: 0,
                w: 278,
                h: 238
            },
            "asymmetric at the image edge"
        );
        let big = RectPx {
            x: 200,
            y: 200,
            w: 3800,
            h: 100,
        };
        let e = outer_rect(big, 200, 8000, 8000).unwrap_err();
        assert_eq!(code(&e), "region_too_large");
        assert_eq!(outer_rect(big, 148, 8000, 8000).unwrap().w, 4096);
        let e = region_fit(rect(0.0, 0.0, 50.0, 50.0), 513, 100, 100, None).unwrap_err();
        assert_eq!(
            (e.code, e.field.as_deref()),
            (ErrorCode::InvalidInput, Some("margin"))
        );
    }

    #[test]
    fn fr_rgn_05_fits_unresampled_by_growing_the_margin() {
        // 300×200 inner + 128 → 556×456 outer; grows to multiples of 8 (560×456).
        let f = region_fit(
            rect(1000.0, 1000.0, 300.0, 200.0),
            128,
            4200,
            5600,
            Some(FAKE_PX),
        )
        .unwrap();
        assert_eq!((f.outer.w, f.outer.h), (560, 456));
        assert_eq!((f.sent_w, f.sent_h), (560, 456));
        assert!(f.unresampled());
        assert_eq!(f.scale, 1.0);
        assert!(
            f.outer.x <= 872 && f.outer.right() >= 1428,
            "contains the old outer"
        );
        // Below min: 16×16 with margin 0 grows to 256 for GPT.
        let f = region_fit(rect(500.0, 500.0, 16.0, 16.0), 0, 2000, 2000, Some(GPT_PX)).unwrap();
        assert_eq!(
            (f.outer.w, f.outer.h, f.sent_w, f.sent_h),
            (256, 256, 256, 256)
        );
        assert!(f.outer.x <= 500 && f.outer.right() >= 516);
    }

    #[test]
    fn fr_rgn_05_no_room_resamples_to_the_nearest_valid_size() {
        // A 100 px tall image can't give GPT its 256 min: the height is resampled up.
        let f = region_fit(rect(0.0, 0.0, 500.0, 100.0), 0, 1000, 100, Some(GPT_PX)).unwrap();
        assert_eq!((f.outer.w, f.outer.h), (512, 100));
        assert_eq!((f.sent_w, f.sent_h), (512, 256));
        assert!(!f.unresampled());
        // A 1003 px wide image, outer spans it all: nearest multiple of 8 is 1000 (down).
        let f = region_fit(rect(0.0, 0.0, 1003.0, 200.0), 0, 1003, 200, Some(FAKE_PX)).unwrap();
        assert_eq!((f.outer.w, f.sent_w), (1003, 1000));
    }

    #[test]
    fn fr_rgn_05_downscales_with_lanczos_ratio_and_rounds_down() {
        // 4096×2000 outer for GPT (max 3840): scale 3840/4096 = 0.9375.
        let f = region_fit(
            rect(100.0, 100.0, 4096.0, 2000.0),
            0,
            8000,
            8000,
            Some(GPT_PX),
        )
        .unwrap();
        assert_eq!((f.outer.w, f.outer.h), (4096, 2000));
        assert_eq!((f.sent_w, f.sent_h), (3840, 1872));
        assert_eq!(f.scale, 0.9375);
        assert!(f.sent_w.is_multiple_of(16) && f.sent_h.is_multiple_of(16));
        // A long thin region: the short side grows so it lands at ≥ min after scaling.
        let f = region_fit(
            rect(100.0, 1000.0, 4096.0, 100.0),
            0,
            8000,
            8000,
            Some(GPT_PX),
        )
        .unwrap();
        assert!(f.sent_h >= 256 && f.sent_h.is_multiple_of(16), "{f:?}");
        assert!(f.outer.h >= 273);
        // Without a model nothing is fitted.
        let f = region_fit(rect(10.0, 10.0, 33.0, 17.0), 0, 100, 100, None).unwrap();
        assert_eq!((f.sent_w, f.sent_h, f.scale), (33, 17, 1.0));
    }
}

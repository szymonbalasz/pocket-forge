//! The patch bake (ADR 0004, KQ-23, FR-RGN-25/26/32): the candidate resized to the outer rect,
//! a least-squares colour match fitted on the unmasked context, an inward feather on the mask,
//! baked into an RGBA patch cropped to alpha > 0. In sRGB, integer box blurs: deterministic.
//!
//! The algorithm is the imaging spike's (`spikes/imaging/src/region.rs` in git history), which
//! measured an edge step of 0.28 levels/px and a residual offset of 0.03 (FR-RGN-32).
//! The pixel work is `pf_imaging::bake_patch` (ADR 0004); RGN owns the controls, the notices
//! and the layer's `blend` record.

use image::{RgbImage, RgbaImage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use pf_imaging::{ColourFit, GAIN_RANGE, MIN_CONTEXT_SHARE, feather, fit_colour};

use super::fit::RectPx;

/// Feather range (FR-RGN-26).
pub const MAX_FEATHER: u32 = 256;
pub const DEFAULT_FEATHER: u32 = 48;

/// Blend controls as the panel sends them (`{feather_px, colour_match}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlendInput {
    #[serde(default = "default_feather")]
    pub feather_px: u32,
    #[serde(default = "default_on")]
    pub colour_match: bool,
}

fn default_feather() -> u32 {
    DEFAULT_FEATHER
}

fn default_on() -> bool {
    true
}

impl Default for BlendInput {
    fn default() -> Self {
        BlendInput {
            feather_px: DEFAULT_FEATHER,
            colour_match: true,
        }
    }
}

/// A baked patch: RGBA cropped to alpha > 0, its placement in image px, and the layer's
/// `blend` record (schema §4.3).
#[derive(Debug, Clone)]
pub struct Baked {
    pub rgba: RgbaImage,
    pub rect: RectPx,
    pub blend: Value,
    /// `colour_match_skipped` when too little context is unmasked (FR-RGN-26).
    pub notices: Vec<String>,
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Bakes a patch from `cand` (already at the outer rect's size), the current stack's pixels
/// `base` at the outer rect, and the canonical mask at the outer rect (255 = edit). `outer` places
/// the result in image px.
pub fn bake(
    base: &RgbImage,
    cand: &RgbImage,
    mask: &[u8],
    outer: RectPx,
    blend: BlendInput,
) -> Baked {
    let p = pf_imaging::bake_patch(
        base,
        cand,
        mask,
        blend.feather_px.min(MAX_FEATHER),
        blend.colour_match,
    );
    let mut notices = vec![];
    if p.colour_match_skipped {
        notices.push("colour_match_skipped".to_owned());
    }
    let f = p.fit.unwrap_or(ColourFit::IDENTITY);
    let blend_record = json!({
        "feather_px": blend.feather_px,
        "colour_match": {
            "on": p.fit.is_some(),
            "gain": f.gain.map(round3),
            "offset": f.offset.map(round3),
        },
    });
    Baked {
        rgba: p.rgba,
        rect: RectPx {
            x: outer.x + p.crop.x,
            y: outer.y + p.crop.y,
            w: p.crop.width,
            h: p.crop.height,
        },
        blend: blend_record,
        notices,
    }
}

/// The blend input recorded on a layer (`blend{feather_px, colour_match{on}}`), for re-tune.
pub fn blend_of(record: &Value) -> BlendInput {
    BlendInput {
        feather_px: record["feather_px"]
            .as_u64()
            .map_or(DEFAULT_FEATHER, |v| v as u32),
        colour_match: record["colour_match"]["on"].as_bool().unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_rgn_26_feather_zero_is_the_painted_edge_and_never_reaches_outside() {
        let (w, h) = (64, 32);
        let mask: Vec<u8> = (0..w * h)
            .map(|i| if i % w >= 32 { 255 } else { 0 })
            .collect();
        assert_eq!(feather(&mask, w, h, 0), mask);
        let f = feather(&mask, w, h, 12);
        for (i, (&a, &m)) in f.iter().zip(&mask).enumerate() {
            if m == 0 {
                assert_eq!(a, 0, "outside never changes ({i})");
            }
        }
        let row = &f[16 * w as usize..17 * w as usize];
        assert!(row[32] < 32, "about 0 at the mask edge: {}", row[32]);
        assert_eq!(row[63], 255, "full deep inside");
        assert!(
            row.windows(2).skip(32).all(|p| p[1] >= p[0]),
            "ramps inward"
        );
    }

    #[test]
    fn fr_rgn_26_colour_match_fits_gain_and_offset_and_skips_without_context() {
        let base = RgbImage::from_fn(40, 40, |x, y| {
            image::Rgb([(x * 5) as u8, (y * 5) as u8, 100])
        });
        let cand = RgbImage::from_fn(40, 40, |x, y| {
            let p = base.get_pixel(x, y).0;
            image::Rgb([
                p[0].saturating_add(12),
                p[1].saturating_add(12),
                p[2].saturating_add(12),
            ])
        });
        let mask: Vec<u8> = (0..1600)
            .map(|i| if i % 40 >= 20 { 255 } else { 0 })
            .collect();
        let fit = fit_colour(&base, &cand, &mask).unwrap();
        assert!((fit.offset[0] + 12.0).abs() < 0.5, "{fit:?}");
        assert!((fit.gain[0] - 1.0).abs() < 0.01);
        let full = vec![255u8; 1600];
        assert!(fit_colour(&base, &cand, &full).is_none());
        let baked = bake(
            &base,
            &cand,
            &full,
            RectPx {
                x: 0,
                y: 0,
                w: 40,
                h: 40,
            },
            BlendInput::default(),
        );
        assert_eq!(baked.notices, ["colour_match_skipped"]);
        assert_eq!(baked.blend["colour_match"]["on"], false);
    }

    #[test]
    fn fr_rgn_27_bake_crops_to_alpha_and_places_in_image_px() {
        let base = RgbImage::from_pixel(50, 40, image::Rgb([10, 20, 30]));
        let cand = RgbImage::from_pixel(50, 40, image::Rgb([200, 100, 50]));
        let mask: Vec<u8> = (0..2000)
            .map(|i| {
                let (x, y) = (i % 50, i / 50);
                if (10..30).contains(&x) && (5..25).contains(&y) {
                    255
                } else {
                    0
                }
            })
            .collect();
        let outer = RectPx {
            x: 100,
            y: 200,
            w: 50,
            h: 40,
        };
        let b = bake(
            &base,
            &cand,
            &mask,
            outer,
            BlendInput {
                feather_px: 0,
                colour_match: false,
            },
        );
        assert_eq!(
            b.rect,
            RectPx {
                x: 110,
                y: 205,
                w: 20,
                h: 20
            }
        );
        assert_eq!(b.rgba.get_pixel(0, 0).0, [200, 100, 50, 255]);
        assert_eq!(
            b.blend,
            json!({"feather_px": 0, "colour_match": {"on": false, "gain": [1.0, 1.0, 1.0], "offset": [0.0, 0.0, 0.0]}})
        );
    }
}

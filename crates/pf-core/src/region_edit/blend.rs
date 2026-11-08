//! The patch bake (ADR 0004, KQ-23, FR-RGN-25/26/32): the candidate resized to the outer rect,
//! a least-squares colour match fitted on the unmasked context, an inward feather on the mask,
//! baked into an RGBA patch cropped to alpha > 0. In sRGB, integer box blurs: deterministic.
//!
//! The algorithm is the imaging spike's (`spikes/imaging/src/region.rs` in git history), which
//! measured an edge step of 0.28 levels/px and a residual offset of 0.03 (FR-RGN-32).
//! > pf-imaging has no blend module yet; this lives in RGN until one exists.

use image::{RgbImage, RgbaImage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::fit::RectPx;

/// Feather range (FR-RGN-26).
pub const MAX_FEATHER: u32 = 256;
pub const DEFAULT_FEATHER: u32 = 48;
/// Colour match needs this share of the outer rect unmasked (FR-RGN-26).
pub const MIN_CONTEXT_SHARE: f64 = 0.01;
/// The fitted gain is clamped to this range (the spike's guard against a flat context).
pub const GAIN_RANGE: (f64, f64) = (0.8, 1.25);

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

/// A fitted colour match: per channel `out = in × gain + offset`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColourFit {
    pub gain: [f64; 3],
    pub offset: [f64; 3],
}

impl ColourFit {
    pub const IDENTITY: ColourFit = ColourFit {
        gain: [1.0; 3],
        offset: [0.0; 3],
    };
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

fn box_h(src: &[u8], w: usize, r: usize) -> Vec<u8> {
    let mut out = vec![0u8; src.len()];
    let n = (2 * r + 1) as u32;
    for (o, s) in out.chunks_mut(w).zip(src.chunks(w)) {
        let at = |i: isize| u32::from(s[i.clamp(0, w as isize - 1) as usize]);
        let mut sum: u32 = (-(r as isize)..=r as isize).map(at).sum();
        for (x, px) in o.iter_mut().enumerate() {
            *px = ((sum + n / 2) / n) as u8;
            sum += at(x as isize + r as isize + 1);
            sum -= at(x as isize - r as isize);
        }
    }
    out
}

fn transpose(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            out[x * h + y] = src[y * w + x];
        }
    }
    out
}

/// Inward feather: alpha is 0 at the mask edge and reaches the mask's value about `f` px inside.
/// Pixels outside the mask (0) stay 0. `f` = 0 returns the painted mask.
pub fn feather(mask: &[u8], w: u32, h: u32, f: u32) -> Vec<u8> {
    if f == 0 || mask.is_empty() {
        return mask.to_vec();
    }
    let (w, h) = (w as usize, h as usize);
    let r = (f as usize / 3).max(1);
    let mut b = mask.to_vec();
    for _ in 0..3 {
        b = box_h(&b, w, r);
    }
    let mut t = transpose(&b, w, h);
    for _ in 0..3 {
        t = box_h(&t, h, r);
    }
    let b = transpose(&t, h, w);
    b.iter()
        .zip(mask)
        .map(|(&v, &m)| {
            if m == 0 {
                0
            } else {
                (2 * i32::from(v) - 255).clamp(0, i32::from(m)) as u8
            }
        })
        .collect()
}

/// Least-squares gain and offset per channel over the unmasked context (mask = 0), or `None`
/// when less than [`MIN_CONTEXT_SHARE`] of the rect is unmasked.
pub fn fit_colour(base: &RgbImage, cand: &RgbImage, mask: &[u8]) -> Option<ColourFit> {
    let total = mask.len();
    let n = mask.iter().filter(|&&m| m == 0).count();
    if total == 0 || (n as f64) < MIN_CONTEXT_SHARE * total as f64 || n == 0 {
        return None;
    }
    let (b, c) = (base.as_raw(), cand.as_raw());
    let mut fit = ColourFit::IDENTITY;
    for ch in 0..3 {
        let (mut sx, mut sy, mut sxx, mut sxy) = (0f64, 0f64, 0f64, 0f64);
        for (i, _) in mask.iter().enumerate().filter(|(_, m)| **m == 0) {
            let x = f64::from(c[i * 3 + ch]);
            let y = f64::from(b[i * 3 + ch]);
            sx += x;
            sy += y;
            sxx += x * x;
            sxy += x * y;
        }
        let nf = n as f64;
        let (mx, my) = (sx / nf, sy / nf);
        let var = sxx / nf - mx * mx;
        let cov = sxy / nf - mx * my;
        let gain = if var > 1e-6 {
            (cov / var).clamp(GAIN_RANGE.0, GAIN_RANGE.1)
        } else {
            1.0
        };
        fit.gain[ch] = gain;
        fit.offset[ch] = my - gain * mx;
    }
    Some(fit)
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
    let (w, h) = cand.dimensions();
    debug_assert_eq!(base.dimensions(), (w, h));
    debug_assert_eq!(mask.len(), (w * h) as usize);
    let mut notices = vec![];
    let fit = if blend.colour_match {
        match fit_colour(base, cand, mask) {
            Some(f) => Some(f),
            None => {
                notices.push("colour_match_skipped".to_owned());
                None
            }
        }
    } else {
        None
    };
    let mut alpha = feather(mask, w, h, blend.feather_px.min(MAX_FEATHER));
    if !alpha.iter().any(|&a| a > 0) {
        // A sliver of a mask the feather ate completely: keep the painted edge.
        alpha = mask.to_vec();
    }
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if alpha[(y * w + x) as usize] > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        (x0, y0, x1, y1) = (0, 0, 1, 1);
    }
    let f = fit.unwrap_or(ColourFit::IDENTITY);
    let src = cand.as_raw();
    let rgba = RgbaImage::from_fn(x1 - x0, y1 - y0, |x, y| {
        let (sx, sy) = (x + x0, y + y0);
        let i = (sy * w + sx) as usize;
        let mut px = [0u8; 4];
        for ch in 0..3 {
            let v = f64::from(src[i * 3 + ch]) * f.gain[ch] + f.offset[ch];
            px[ch] = v.round().clamp(0.0, 255.0) as u8;
        }
        px[3] = alpha[i];
        image::Rgba(px)
    });
    let blend_record = json!({
        "feather_px": blend.feather_px,
        "colour_match": {
            "on": fit.is_some(),
            "gain": f.gain.map(round3),
            "offset": f.offset.map(round3),
        },
    });
    Baked {
        rgba,
        rect: RectPx {
            x: outer.x + x0,
            y: outer.y + y0,
            w: x1 - x0,
            h: y1 - y0,
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

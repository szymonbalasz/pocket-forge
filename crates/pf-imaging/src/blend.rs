//! Patch blend (ADR 0004, KQ-23): an inward feather on the mask, a least-squares colour match
//! fitted on the unmasked context, baked into an RGBA patch cropped to alpha > 0. In sRGB,
//! integer box blurs: deterministic.
//!
//! The algorithm is the imaging spike's (`spikes/imaging/src/region.rs` in git history), which
//! measured an edge step of 0.28 levels/px and a residual offset of 0.03. Region edit (RGN) owns
//! the controls and the layer's `blend` record; this module only works on pixels.

use image::{RgbImage, RgbaImage};

use crate::PixelRect;

/// Colour match needs this share of the rect unmasked.
pub const MIN_CONTEXT_SHARE: f64 = 0.01;
/// The fitted gain is clamped to this range (the spike's guard against a flat context).
pub const GAIN_RANGE: (f64, f64) = (0.8, 1.25);

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

/// A baked patch: RGBA cropped to alpha > 0 and its placement inside the input rect.
#[derive(Debug, Clone)]
pub struct BakedPatch {
    pub rgba: RgbaImage,
    /// Where `rgba` sits, in px of the input rect.
    pub crop: PixelRect,
    /// The colour match applied, or `None` when it was off or skipped.
    pub fit: Option<ColourFit>,
    /// Colour match was asked for but too little context is unmasked ([`MIN_CONTEXT_SHARE`]).
    pub colour_match_skipped: bool,
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

/// Bakes a patch from `cand`, the current pixels `base` at the same rect, and the mask at that
/// rect (255 = edit). All three have the same size. `feather_px` is used as given (the caller
/// clamps it to its range).
pub fn bake_patch(
    base: &RgbImage,
    cand: &RgbImage,
    mask: &[u8],
    feather_px: u32,
    colour_match: bool,
) -> BakedPatch {
    let (w, h) = cand.dimensions();
    debug_assert_eq!(base.dimensions(), (w, h));
    debug_assert_eq!(mask.len(), (w * h) as usize);
    let fit = if colour_match {
        fit_colour(base, cand, mask)
    } else {
        None
    };
    let colour_match_skipped = colour_match && fit.is_none();
    let mut alpha = feather(mask, w, h, feather_px);
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
    BakedPatch {
        rgba,
        crop: PixelRect::new(x0, y0, x1 - x0, y1 - y0),
        fit,
        colour_match_skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A smooth base, the candidate = base + 12 inside and out (the fake `offset(12)` case),
    /// a centred square mask.
    fn seam_case() -> (RgbImage, RgbImage, Vec<u8>, u32, u32) {
        let (w, h) = (160u32, 120u32);
        let base = RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(40 + x) as u8, (60 + y) as u8, (100 + (x + y) / 4) as u8])
        });
        let cand = RgbImage::from_fn(w, h, |x, y| {
            image::Rgb(base.get_pixel(x, y).0.map(|v| v.saturating_add(12)))
        });
        let mask = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if (40..120).contains(&x) && (30..90).contains(&y) {
                    255
                } else {
                    0
                }
            })
            .collect();
        (base, cand, mask, w, h)
    }

    fn over(base: &RgbImage, p: &BakedPatch) -> RgbImage {
        let mut out = base.clone();
        for (x, y, px) in p.rgba.enumerate_pixels() {
            let (ox, oy) = (x + p.crop.x, y + p.crop.y);
            let a = f64::from(px.0[3]) / 255.0;
            let b = out.get_pixel(ox, oy).0;
            let mut v = [0u8; 3];
            for ch in 0..3 {
                v[ch] = (f64::from(px.0[ch]) * a + f64::from(b[ch]) * (1.0 - a)).round() as u8;
            }
            out.put_pixel(ox, oy, image::Rgb(v));
        }
        out
    }

    #[test]
    fn fr_rgn_32_bake_patch_removes_the_offset_and_leaves_no_seam() {
        let (base, cand, mask, w, _) = seam_case();
        let p = bake_patch(&base, &cand, &mask, 48, true);
        assert!(!p.colour_match_skipped);
        let fit = p.fit.expect("context is unmasked");
        assert!((fit.offset[0] + 12.0).abs() < 0.5, "{fit:?}");
        let out = over(&base, &p);
        // Residual offset over the patch, and the step across the mask's left edge on row 60.
        let resid: f64 = (40..120)
            .map(|x| f64::from(out.get_pixel(x, 60).0[0]) - f64::from(base.get_pixel(x, 60).0[0]))
            .sum::<f64>()
            / 80.0;
        assert!(resid.abs() <= 0.1, "residual offset {resid}");
        let step = |x: u32| {
            (f64::from(out.get_pixel(x + 1, 60).0[0]) - f64::from(out.get_pixel(x, 60).0[0]) - 1.0)
                .abs()
        };
        assert!(step(39) <= 0.5 && step(119) <= 0.5, "edge step");
        assert_eq!(
            out.get_pixel(10, 10),
            base.get_pixel(10, 10),
            "outside never changes"
        );
        assert!(p.crop.x >= 40 && p.crop.x + p.crop.width <= 120 && w == 160);
    }

    #[test]
    fn fr_rgn_26_bake_patch_skips_colour_match_without_context_and_keeps_a_sliver() {
        let base = RgbImage::from_pixel(20, 20, image::Rgb([10, 20, 30]));
        let cand = RgbImage::from_pixel(20, 20, image::Rgb([200, 100, 50]));
        let full = vec![255u8; 400];
        let p = bake_patch(&base, &cand, &full, 0, true);
        assert!(p.colour_match_skipped && p.fit.is_none());
        assert_eq!(p.crop, PixelRect::new(0, 0, 20, 20));
        assert_eq!(p.rgba.get_pixel(5, 5).0, [200, 100, 50, 255]);
        // A one-px column the feather eats completely keeps its painted edge.
        let sliver: Vec<u8> = (0..400)
            .map(|i| if i % 20 == 7 { 255 } else { 0 })
            .collect();
        let p = bake_patch(&base, &cand, &sliver, 48, false);
        assert!(!p.colour_match_skipped && p.fit.is_none());
        assert_eq!(p.crop, PixelRect::new(7, 0, 1, 20));
        assert_eq!(p.rgba.get_pixel(0, 3).0[3], 255);
    }
}

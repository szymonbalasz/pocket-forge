//! Downscaled copies (FR-EDT-32): Lanczos3, aspect kept, never upscaled, PNG out, alpha kept.

use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageFormat as Fmt, imageops::FilterType};

use crate::probe::ProbeError;
use crate::rgb_source::decode_whole;

/// The size of a copy whose longer side is at most `max_side` (never larger than the source).
pub fn fit_within(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= max_side || max_side == 0 {
        return (width, height);
    }
    let f = f64::from(max_side) / f64::from(long);
    let side = |s: u32| ((f64::from(s) * f).round() as u32).clamp(1, max_side);
    (side(width), side(height))
}

/// A PNG copy of `src` that fits within `max_side` px: `(png bytes, width, height)`.
pub fn downscaled_png(src: &Path, max_side: u32) -> Result<(Vec<u8>, u32, u32), ProbeError> {
    let img = decode_whole(src)?;
    let (w, h) = fit_within(img.width(), img.height(), max_side);
    let img = if (w, h) == (img.width(), img.height()) {
        img
    } else {
        img.resize_exact(w, h, FilterType::Lanczos3)
    };
    let img = if img.color().has_alpha() {
        DynamicImage::ImageRgba8(img.into_rgba8())
    } else {
        DynamicImage::ImageRgb8(img.into_rgb8())
    };
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, Fmt::Png)
        .map_err(|e| ProbeError::Corrupt(e.to_string()))?;
    Ok((out.into_inner(), w, h))
}

/// A patch layer drawn onto a working-image thumbnail.
#[derive(Debug, Clone, Copy)]
pub struct ThumbPatch<'a> {
    /// The baked RGBA PNG (cropped to alpha > 0).
    pub path: &'a Path,
    /// Top-left in image px.
    pub x: i64,
    pub y: i64,
    /// 0–1.
    pub opacity: f32,
}

/// A JPEG thumbnail of the **working image** (PRJ FR-PRJ-50): the original with its visible
/// patch layers drawn in order, fitted within `max_side`. Returns `(jpeg bytes, width, height)`.
/// An approximation for the home grid; the export compositor stays the only exact one (KQ-25).
pub fn working_thumbnail_jpeg(
    original: &Path,
    patches: &[ThumbPatch<'_>],
    max_side: u32,
    quality: u8,
) -> Result<(Vec<u8>, u32, u32), ProbeError> {
    let base = decode_whole(original)?;
    let (bw, bh) = (base.width(), base.height());
    let (w, h) = fit_within(bw, bh, max_side);
    let mut canvas = if (w, h) == (bw, bh) {
        base.into_rgba8()
    } else {
        base.resize_exact(w, h, FilterType::Lanczos3).into_rgba8()
    };
    onto_black(&mut canvas);
    let f = f64::from(w) / f64::from(bw);
    for p in patches {
        let img = decode_whole(p.path)?.into_rgba8();
        let pw = ((f64::from(img.width()) * f).round() as u32).max(1);
        let ph = ((f64::from(img.height()) * f).round() as u32).max(1);
        let mut small = image::imageops::resize(&img, pw, ph, FilterType::Triangle);
        let opacity = p.opacity.clamp(0.0, 1.0);
        if opacity < 1.0 {
            for px in small.pixels_mut() {
                px.0[3] = (f32::from(px.0[3]) * opacity).round() as u8;
            }
        }
        let x = (p.x as f64 * f).round() as i64;
        let y = (p.y as f64 * f).round() as i64;
        image::imageops::overlay(&mut canvas, &small, x, y);
    }
    let rgb = DynamicImage::ImageRgba8(canvas).into_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&rgb)
        .map_err(|e| ProbeError::Corrupt(e.to_string()))?;
    Ok((out, w, h))
}

/// The original sits on opaque black (export spec FR-EXP-02).
fn onto_black(img: &mut image::RgbaImage) {
    for px in img.pixels_mut() {
        let a = u16::from(px.0[3]);
        for c in &mut px.0[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
        px.0[3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prj_50_working_thumbnail_draws_patches() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("map.png");
        image::RgbImage::from_pixel(1024, 512, image::Rgb([0, 0, 200]))
            .save(&src)
            .unwrap();
        let patch = dir.path().join("patch.png");
        image::RgbaImage::from_pixel(256, 256, image::Rgba([255, 0, 0, 255]))
            .save(&patch)
            .unwrap();
        let (jpeg, w, h) = working_thumbnail_jpeg(
            &src,
            &[ThumbPatch {
                path: &patch,
                x: 0,
                y: 0,
                opacity: 1.0,
            }],
            512,
            80,
        )
        .unwrap();
        assert_eq!((w, h), (512, 256));
        let back = image::load_from_memory(&jpeg).unwrap().into_rgb8();
        assert_eq!((back.width(), back.height()), (512, 256));
        let red = back.get_pixel(60, 60).0;
        let blue = back.get_pixel(400, 200).0;
        assert!(red[0] > 200 && red[2] < 60, "{red:?}");
        assert!(blue[2] > 150 && blue[0] < 60, "{blue:?}");
    }

    #[test]
    fn fr_edt_32_fit_keeps_aspect_and_never_upscales() {
        assert_eq!(fit_within(1024, 512, 256), (256, 128));
        assert_eq!(fit_within(300, 1000, 256), (77, 256));
        assert_eq!(fit_within(100, 50, 256), (100, 50));
        assert_eq!(fit_within(5000, 1, 256), (256, 1));
    }

    #[test]
    fn fr_edt_32_downscaled_png_keeps_alpha() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.png");
        image::RgbaImage::from_fn(600, 300, |x, _| image::Rgba([x as u8, 0, 0, 128]))
            .save(&src)
            .unwrap();
        let (png, w, h) = downscaled_png(&src, 256).unwrap();
        assert_eq!((w, h), (256, 128));
        let back = image::load_from_memory(&png).unwrap();
        assert_eq!((back.width(), back.height()), (256, 128));
        assert!(back.color().has_alpha());
        assert_eq!(back.to_rgba8().get_pixel(10, 10).0[3], 128);
    }
}

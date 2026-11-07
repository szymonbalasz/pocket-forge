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

#[cfg(test)]
mod tests {
    use super::*;

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

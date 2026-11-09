//! The **flatten colour** (GEN FR-GEN-12, OQ-GEN-16, #60): every RGB8 use of an image with alpha
//! (canvas tiles, thumbnail, region crop, export) composites it over neutral mid grey `#808080`,
//! so the canvas, the region sent to the provider and the export agree pixel for pixel. Files keep
//! their alpha; only the RGB8 view is flattened.

use image::{DynamicImage, RgbImage, RgbaImage};

/// Mid grey `#808080`.
pub const FLATTEN_BG: [u8; 3] = [0x80, 0x80, 0x80];

/// One channel `c` with alpha `a` over the flatten colour's channel, rounded.
#[inline]
fn over(c: u8, a: u8, bg: u8) -> u8 {
    let (c, a, bg) = (u32::from(c), u32::from(a), u32::from(bg));
    ((c * a + bg * (255 - a) + 127) / 255) as u8
}

/// An RGBA8 pixel over [`FLATTEN_BG`].
#[inline]
pub fn flatten_px(px: [u8; 4]) -> [u8; 3] {
    let a = px[3];
    [
        over(px[0], a, FLATTEN_BG[0]),
        over(px[1], a, FLATTEN_BG[1]),
        over(px[2], a, FLATTEN_BG[2]),
    ]
}

/// A grey + alpha pixel over [`FLATTEN_BG`].
#[inline]
pub(crate) fn flatten_grey_alpha(g: u8, a: u8) -> [u8; 3] {
    flatten_px([g, g, g, a])
}

/// Flattens an RGBA8 image in place: RGB over [`FLATTEN_BG`], alpha 255.
pub fn flatten_rgba_in_place(img: &mut RgbaImage) {
    for px in img.pixels_mut() {
        let [r, g, b] = flatten_px(px.0);
        px.0 = [r, g, b, 255];
    }
}

/// Any decoded image as RGB8: an image with alpha is composited over [`FLATTEN_BG`]; an opaque
/// one is converted as is. RGBA8 is flattened inside its own buffer, so a 16k original never
/// holds the RGBA and the RGB copy at once (ADR 0004, review #12).
pub fn to_rgb8_flattened(img: DynamicImage) -> RgbImage {
    if !img.color().has_alpha() {
        return img.into_rgb8();
    }
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    let mut buf = rgba.into_raw();
    // Pixel i moves from 4i to 3i ≤ 4i: every read is ahead of (or at) the write position.
    let n = buf.len() / 4;
    for i in 0..n {
        let px = [buf[4 * i], buf[4 * i + 1], buf[4 * i + 2], buf[4 * i + 3]];
        buf[3 * i..3 * i + 3].copy_from_slice(&flatten_px(px));
    }
    buf.truncate(3 * n);
    buf.shrink_to_fit();
    RgbImage::from_raw(w, h, buf).expect("w·h·3 bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_gen_12_flatten_colour_is_mid_grey() {
        assert_eq!(FLATTEN_BG, [0x80, 0x80, 0x80]);
        assert_eq!(
            flatten_px([255, 0, 10, 0]),
            FLATTEN_BG,
            "alpha 0 = the flatten colour"
        );
        assert_eq!(
            flatten_px([255, 0, 10, 255]),
            [255, 0, 10],
            "alpha 255 = the pixel"
        );
        // Half alpha: (255·128 + 128·127 + 127) / 255 = 192; (0·128 + 128·127 + 127) / 255 = 64.
        assert_eq!(flatten_px([255, 0, 128, 128]), [192, 64, 128]);
    }

    #[test]
    fn fr_gen_12_to_rgb8_flattens_alpha_and_keeps_opaque_images() {
        let rgba = RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                image::Rgba([10, 20, 30, 0])
            } else {
                image::Rgba([10, 20, 30, 255])
            }
        });
        let out = to_rgb8_flattened(DynamicImage::ImageRgba8(rgba));
        assert_eq!(out.get_pixel(0, 0).0, FLATTEN_BG);
        assert_eq!(out.get_pixel(1, 0).0, [10, 20, 30]);
        let rgb = RgbImage::from_pixel(1, 1, image::Rgb([1, 2, 3]));
        let same = to_rgb8_flattened(DynamicImage::ImageRgb8(rgb.clone()));
        assert_eq!(same, rgb);
    }
}

//! Export rescale: Lanczos3 in sRGB with `fast_image_resize` (FR-EXP-09, OQ-IMG-13).
//!
//! `Resizer` grows its private two-pass scratch with an infallible `Vec::resize` (about
//! 400 MiB at 16k → 8k), so an allocation failure there would abort the process. We run the two
//! passes ourselves instead: each call then needs only one pass and uses no scratch, and the
//! intermediate image is reserved fallibly, so a failure returns `out_of_memory` (FR-EXP-23,
//! BL-06). The pass order matches the crate's own for u8 pixels (vertical first), so the output
//! is pixel-identical to a single `Resizer::resize` call.

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::RgbImage;

use crate::compose::{ComposeError, try_vec};
use crate::encode::EncodeError;

/// Resamples `image` to `width × height` with Lanczos3, treating values as sRGB (no
/// linearisation). The output and the intermediate buffers are reserved fallibly (FR-EXP-23).
pub fn resize_lanczos3(image: &RgbImage, width: u32, height: u32) -> Result<RgbImage, EncodeError> {
    resize_with(image, width, height, &mut try_vec)
}

/// The fallible buffer allocator; tests inject one with a limit.
type Alloc<'a> = &'a mut dyn FnMut(usize) -> Result<Vec<u8>, ComposeError>;

fn resize_with(
    image: &RgbImage,
    width: u32,
    height: u32,
    alloc: Alloc<'_>,
) -> Result<RgbImage, EncodeError> {
    if width == 0 || height == 0 {
        return Err(EncodeError::Failed(format!("resize to {width}x{height}")));
    }
    let (src_w, src_h) = image.dimensions();
    if (src_w, src_h) == (width, height) {
        let mut out = alloc(image.as_raw().len())?;
        out.copy_from_slice(image.as_raw());
        return RgbImage::from_raw(width, height, out).ok_or(EncodeError::OutOfMemory);
    }
    let out = if src_w == width || src_h == height {
        one_pass(image.as_raw(), (src_w, src_h), (width, height), alloc)?
    } else {
        // Vertical pass first (src_w × height), then horizontal, as the crate does for u8.
        let tmp = one_pass(image.as_raw(), (src_w, src_h), (src_w, height), alloc)?;
        one_pass(&tmp, (src_w, height), (width, height), alloc)?
    };
    RgbImage::from_raw(width, height, out).ok_or(EncodeError::OutOfMemory)
}

/// A resize that changes only one axis (or none), so `Resizer` needs no scratch buffer.
fn one_pass(
    src: &[u8],
    (src_w, src_h): (u32, u32),
    (width, height): (u32, u32),
    alloc: Alloc<'_>,
) -> Result<Vec<u8>, EncodeError> {
    debug_assert!(src_w == width || src_h == height);
    let failed = |e: &dyn std::fmt::Display| EncodeError::Failed(format!("resize: {e}"));
    let src = ImageRef::new(src_w, src_h, src, PixelType::U8x3).map_err(|e| failed(&e))?;
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(3))
        .ok_or(EncodeError::OutOfMemory)?;
    let buf = alloc(len)?;
    let mut dst =
        Image::from_vec_u8(width, height, buf, PixelType::U8x3).map_err(|e| failed(&e))?;
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    Resizer::new()
        .resize(&src, &mut dst, &options)
        .map_err(|e| failed(&e))?;
    Ok(dst.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_exp_09_lanczos3_halves_a_flat_image_exactly() {
        let img = RgbImage::from_pixel(140, 100, image::Rgb([90, 120, 200]));
        let out = resize_lanczos3(&img, 70, 50).unwrap();
        assert_eq!(out.dimensions(), (70, 50));
        assert!(out.pixels().all(|p| p.0 == [90, 120, 200]));
    }

    #[test]
    fn fr_exp_09_resize_to_the_same_size_is_a_copy() {
        let img = RgbImage::from_fn(9, 7, |x, y| image::Rgb([x as u8, y as u8, 3]));
        assert_eq!(resize_lanczos3(&img, 9, 7).unwrap(), img);
    }

    #[test]
    fn fr_exp_09_resize_rejects_an_empty_target() {
        let img = RgbImage::new(4, 4);
        assert!(matches!(
            resize_lanczos3(&img, 0, 4),
            Err(EncodeError::Failed(_))
        ));
    }

    /// The crate's own single-call two-pass resize, for comparison.
    fn reference(image: &RgbImage, width: u32, height: u32) -> RgbImage {
        let src = ImageRef::new(
            image.width(),
            image.height(),
            image.as_raw(),
            PixelType::U8x3,
        )
        .unwrap();
        let mut dst = Image::new(width, height, PixelType::U8x3);
        let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
        Resizer::new().resize(&src, &mut dst, &options).unwrap();
        RgbImage::from_raw(width, height, dst.into_vec()).unwrap()
    }

    #[test]
    fn fr_exp_09_split_passes_match_the_single_call_resize() {
        // High-frequency content so rounding differences between pass orders would show.
        let img = RgbImage::from_fn(97, 61, |x, y| {
            let v = x
                .wrapping_mul(2_654_435_761)
                .wrapping_add(y.wrapping_mul(40_503));
            image::Rgb([(v >> 3) as u8, (v >> 11) as u8, (v >> 19) as u8])
        });
        for (w, h) in [
            (48, 30),
            (200, 130),
            (48, 130),
            (200, 30),
            (97, 30),
            (40, 61),
        ] {
            assert_eq!(
                resize_lanczos3(&img, w, h).unwrap(),
                reference(&img, w, h),
                "{w}x{h}"
            );
        }
    }

    #[test]
    fn fr_exp_23_a_failed_scratch_allocation_returns_out_of_memory() {
        let img = RgbImage::new(64, 48);
        // 32×24 output: the scratch is 64×24×3 = 4608 bytes, the output 2304. A limit between
        // the two fails exactly on the scratch, before any pass runs.
        let mut requests = Vec::new();
        let mut limited = |len: usize| {
            requests.push(len);
            if len > 3000 {
                Err(ComposeError::OutOfMemory)
            } else {
                try_vec(len)
            }
        };
        let r = resize_with(&img, 32, 24, &mut limited);
        assert!(matches!(r, Err(EncodeError::OutOfMemory)), "{r:?}");
        assert_eq!(requests, [64 * 24 * 3]);
    }

    #[test]
    fn fr_exp_23_a_failed_output_allocation_returns_out_of_memory() {
        let img = RgbImage::new(64, 48);
        let mut calls = 0;
        let mut second_fails = |len: usize| {
            calls += 1;
            if calls == 2 {
                Err(ComposeError::OutOfMemory)
            } else {
                try_vec(len)
            }
        };
        let r = resize_with(&img, 32, 24, &mut second_fails);
        assert!(matches!(r, Err(EncodeError::OutOfMemory)), "{r:?}");
    }
}

//! Export rescale: Lanczos3 in sRGB with `fast_image_resize` (FR-EXP-09, OQ-IMG-13).

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::RgbImage;

use crate::compose::try_vec;
use crate::encode::EncodeError;

/// Resamples `image` to `width × height` with Lanczos3, treating values as sRGB (no
/// linearisation). The output buffer is reserved fallibly (FR-EXP-23).
pub fn resize_lanczos3(image: &RgbImage, width: u32, height: u32) -> Result<RgbImage, EncodeError> {
    if width == 0 || height == 0 {
        return Err(EncodeError::Failed(format!("resize to {width}x{height}")));
    }
    if image.dimensions() == (width, height) {
        let mut out = try_vec(image.as_raw().len())?;
        out.copy_from_slice(image.as_raw());
        return RgbImage::from_raw(width, height, out).ok_or(EncodeError::OutOfMemory);
    }
    let failed = |e: &dyn std::fmt::Display| EncodeError::Failed(format!("resize: {e}"));
    let src = ImageRef::new(
        image.width(),
        image.height(),
        image.as_raw(),
        PixelType::U8x3,
    )
    .map_err(|e| failed(&e))?;
    let buf = try_vec(width as usize * height as usize * 3)?;
    let mut dst =
        Image::from_vec_u8(width, height, buf, PixelType::U8x3).map_err(|e| failed(&e))?;
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    Resizer::new()
        .resize(&src, &mut dst, &options)
        .map_err(|e| failed(&e))?;
    RgbImage::from_raw(width, height, dst.into_vec()).ok_or(EncodeError::OutOfMemory)
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
}

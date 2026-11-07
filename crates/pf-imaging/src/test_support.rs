//! Shared test helpers (compiled for tests only).

use image::RgbImage;

/// A smooth, deterministic RGB image (gradients and soft rings), so lossy codecs reach their
/// typical quality instead of fighting noise.
pub fn smooth_rgb(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        let (fx, fy) = (f64::from(x) / f64::from(w), f64::from(y) / f64::from(h));
        let ring = ((fx * fx + fy * fy).sqrt() * 12.0).sin() * 0.5 + 0.5;
        image::Rgb([
            (40.0 + 170.0 * fx) as u8,
            (60.0 + 120.0 * ring) as u8,
            (200.0 - 150.0 * fy) as u8,
        ])
    })
}

/// Peak signal-to-noise ratio in dB over all channels.
pub fn psnr(a: &RgbImage, b: &RgbImage) -> f64 {
    assert_eq!(a.dimensions(), b.dimensions());
    let mse = a
        .as_raw()
        .iter()
        .zip(b.as_raw())
        .map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2))
        .sum::<f64>()
        / a.as_raw().len() as f64;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0 * 255.0 / mse).log10()
    }
}

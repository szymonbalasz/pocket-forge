//! Test images for the probe: the Region edit overlay on the committed grid-map fixture
//! (FR-PRM-24 look-alike), and noise images for the 4 MiB turn (FR-PRM-25 worst case).

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

/// The committed, generated battlemap fixture (KQ-22, public-safe).
const GRID_MAP: &[u8] = include_bytes!("../../../../../fixtures/images/grid-map-512x640.png");

/// FR-PRM-25: long side ≤ 1568 px, JPEG q85.
pub const MAX_SIDE: u32 = 1568;
const QUALITY: u8 = 85;
const MAGENTA: [u8; 3] = [255, 0, 255];

fn jpeg(img: RgbImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    DynamicImage::ImageRgb8(img)
        .write_with_encoder(JpegEncoder::new_with_quality(&mut out, QUALITY))
        .map_err(|e| format!("JPEG encode: {e}"))?;
    Ok(out)
}

/// The grid map with a rectangle tinted magenta at 35 % and a 2 px outline, as JPEG.
pub fn overlay_jpeg() -> Result<(Vec<u8>, u32, u32), String> {
    let mut img = image::load_from_memory_with_format(GRID_MAP, ImageFormat::Png)
        .map_err(|e| format!("decode fixture: {e}"))?
        .to_rgb8();
    let (w, h) = img.dimensions();
    let (x0, y0, x1, y1) = (w / 4, h / 3, w * 3 / 4, h * 2 / 3);
    for y in y0..y1 {
        for x in x0..x1 {
            let edge = x < x0 + 2 || x >= x1 - 2 || y < y0 + 2 || y >= y1 - 2;
            let p = img.get_pixel_mut(x, y);
            for (ch, m) in p.0.iter_mut().zip(MAGENTA) {
                *ch = if edge {
                    m
                } else {
                    (f32::from(*ch) * 0.65 + f32::from(m) * 0.35).round() as u8
                };
            }
        }
    }
    let img = if w.max(h) > MAX_SIDE {
        let s = f64::from(MAX_SIDE) / f64::from(w.max(h));
        image::imageops::resize(
            &img,
            (f64::from(w) * s).round() as u32,
            (f64::from(h) * s).round() as u32,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };
    let (w, h) = img.dimensions();
    Ok((jpeg(img)?, w, h))
}

fn noise(side: u32, seed: u32) -> RgbImage {
    let mut state = seed.wrapping_mul(2_654_435_761).max(1);
    RgbImage::from_fn(side, side, |_, _| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let [r, g, b, _] = state.to_le_bytes();
        Rgb([r, g, b])
    })
}

/// `count` noise JPEGs whose total size is close to, and not over, `budget` bytes.
pub fn noise_jpegs(count: u32, budget: usize) -> Result<Vec<Vec<u8>>, String> {
    let per = budget / count.max(1) as usize;
    let probe = jpeg(noise(512, 7))?.len().max(1);
    // JPEG size of noise grows with the pixel count.
    let mut side = ((per as f64 / probe as f64).sqrt() * 512.0) as u32;
    side = side.clamp(64, MAX_SIDE);
    loop {
        let imgs = (1..=count)
            .map(|i| jpeg(noise(side, i)))
            .collect::<Result<Vec<_>, _>>()?;
        let total: usize = imgs.iter().map(Vec::len).sum();
        if total <= budget || side <= 64 {
            return Ok(imgs);
        }
        side = (f64::from(side) * 0.95) as u32;
    }
}

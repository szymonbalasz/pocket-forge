//! Reference packs (REF): the pack thumbnail mosaic (FR-REF-13) and the deterministic
//! downscale of a reference before it is sent (FR-REF-22).

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{DynamicImage, RgbImage, imageops::FilterType};

use crate::probe::{ImageFormat, ProbeError, probe_bytes};
use crate::rgb_source::{decode_whole, limits};
use crate::thumbnail::fit_within;

/// Why a mosaic wasn't made.
#[derive(Debug, thiserror::Error)]
pub enum MosaicError {
    /// None of the sources decoded.
    #[error("no image to show")]
    NoImage,
    #[error("encoding the thumbnail: {0}")]
    Encode(String),
}

/// Background of empty cells (neutral dark grey, ui-guidelines).
const EMPTY: [u8; 3] = [0x2a, 0x2a, 0x2a];

/// A `side × side` JPEG mosaic of the first four sources that decode: one image fills it, two
/// sit side by side, three or four make a 2×2 grid. Each image is centre-cropped to its cell.
pub fn mosaic_jpeg(sources: &[PathBuf], side: u32, quality: u8) -> Result<Vec<u8>, MosaicError> {
    let images: Vec<DynamicImage> = sources
        .iter()
        .filter_map(|p| decode_small(p, side).ok())
        .take(4)
        .collect();
    if images.is_empty() {
        return Err(MosaicError::NoImage);
    }
    let half = side / 2;
    let cells: Vec<(u32, u32, u32, u32)> = match images.len() {
        1 => vec![(0, 0, side, side)],
        2 => vec![(0, 0, half, side), (half, 0, side - half, side)],
        _ => vec![
            (0, 0, half, half),
            (half, 0, side - half, half),
            (0, half, half, side - half),
            (half, half, side - half, side - half),
        ],
    };
    let mut canvas = RgbImage::from_pixel(side, side, image::Rgb(EMPTY));
    for (img, (x, y, w, h)) in images.iter().zip(cells) {
        let cell = img.resize_to_fill(w, h, FilterType::Triangle).into_rgb8();
        image::imageops::replace(&mut canvas, &cell, i64::from(x), i64::from(y));
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&canvas)
        .map_err(|e| MosaicError::Encode(e.to_string()))?;
    Ok(out)
}

/// Decodes a file and shrinks it near `side` first, so a 16k source doesn't stay in memory.
fn decode_small(path: &Path, side: u32) -> Result<DynamicImage, ProbeError> {
    let img = decode_whole(path)?;
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= side * 2 {
        return Ok(img);
    }
    let f = f64::from(side * 2) / f64::from(w.min(h));
    let (nw, nh) = (
        ((f64::from(w) * f).round() as u32).max(1),
        ((f64::from(h) * f).round() as u32).max(1),
    );
    Ok(img.resize_exact(nw, nh, FilterType::Triangle))
}

/// A reference as it will be sent (FR-REF-22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentImage {
    /// The bytes to send: the file as-is, or a PNG.
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// True when downscaled and re-encoded.
    pub resized: bool,
    pub format: ImageFormat,
}

/// FR-REF-22: `bytes` are sent as-is if they are PNG, JPEG or WebP, the long side is
/// ≤ `max_side` and the size is ≤ `max_bytes`. Otherwise the image is downscaled with Lanczos3
/// to fit `max_side` and encoded as PNG (alpha kept). Deterministic: the same input always
/// gives the same bytes. If the PNG is still over `max_bytes`, the side shrinks by a quarter
/// until it fits.
pub fn prepare_reference(
    bytes: &[u8],
    max_side: u32,
    max_bytes: u64,
) -> Result<SentImage, ProbeError> {
    let info = probe_bytes(bytes)?;
    let long = info.width.max(info.height);
    if long <= max_side && bytes.len() as u64 <= max_bytes {
        return Ok(SentImage {
            bytes: bytes.to_vec(),
            width: info.width,
            height: info.height,
            resized: false,
            format: info.format,
        });
    }
    let mut reader = image::ImageReader::with_format(
        Cursor::new(bytes),
        match info.format {
            ImageFormat::Png => image::ImageFormat::Png,
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
            ImageFormat::Webp => image::ImageFormat::WebP,
        },
    );
    reader.limits(limits());
    let img = reader.decode().map_err(|e| match e {
        image::ImageError::IoError(io) => ProbeError::Io(io),
        other => ProbeError::Corrupt(other.to_string()),
    })?;
    let mut side = max_side.max(1);
    loop {
        let (w, h) = fit_within(img.width(), img.height(), side);
        let small = if (w, h) == (img.width(), img.height()) {
            img.clone()
        } else {
            img.resize_exact(w, h, FilterType::Lanczos3)
        };
        let small = if small.color().has_alpha() {
            DynamicImage::ImageRgba8(small.into_rgba8())
        } else {
            DynamicImage::ImageRgb8(small.into_rgb8())
        };
        let mut out = Cursor::new(Vec::new());
        small
            .write_to(&mut out, image::ImageFormat::Png)
            .map_err(|e| ProbeError::Corrupt(e.to_string()))?;
        let out = out.into_inner();
        if out.len() as u64 <= max_bytes || side <= 64 {
            return Ok(SentImage {
                bytes: out,
                width: w,
                height: h,
                resized: true,
                format: ImageFormat::Png,
            });
        }
        side = side * 3 / 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 7])
        })
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
        out
    }

    #[test]
    fn fr_ref_13_mosaic_layouts_and_skips_broken_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = vec![];
        for (i, c) in [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]]
            .iter()
            .enumerate()
        {
            let p = dir.path().join(format!("{i}.png"));
            RgbImage::from_pixel(64, 48, image::Rgb(*c))
                .save(&p)
                .unwrap();
            paths.push(p);
        }
        let broken = dir.path().join("broken.png");
        std::fs::write(&broken, b"\x89PNG\r\n\x1a\nnope").unwrap();
        paths.insert(1, broken);
        let jpeg = mosaic_jpeg(&paths, 512, 80).unwrap();
        let back = image::load_from_memory(&jpeg).unwrap().into_rgb8();
        assert_eq!(back.dimensions(), (512, 512));
        let near = |p: [u8; 3], c: [u8; 3]| p.iter().zip(c).all(|(a, b)| a.abs_diff(b) < 40);
        assert!(near(back.get_pixel(128, 128).0, [255, 0, 0]));
        assert!(near(back.get_pixel(384, 128).0, [0, 255, 0]));
        assert!(near(back.get_pixel(128, 384).0, [0, 0, 255]));
        assert!(near(back.get_pixel(384, 384).0, [255, 255, 0]));
        let one = mosaic_jpeg(&paths[..1], 512, 80).unwrap();
        let back = image::load_from_memory(&one).unwrap().into_rgb8();
        assert!(
            near(back.get_pixel(400, 400).0, [255, 0, 0]),
            "one image fills it"
        );
        assert!(matches!(
            mosaic_jpeg(&paths[1..2], 512, 80),
            Err(MosaicError::NoImage)
        ));
    }

    #[test]
    fn fr_ref_22_small_files_are_sent_as_is() {
        let bytes = png(300, 200);
        let s = prepare_reference(&bytes, 2048, u64::MAX).unwrap();
        assert!(!s.resized);
        assert_eq!(s.bytes, bytes);
        assert_eq!((s.width, s.height), (300, 200));
    }

    #[test]
    fn fr_ref_22_large_files_are_downscaled_deterministically() {
        let bytes = png(3000, 1500);
        let a = prepare_reference(&bytes, 2048, u64::MAX).unwrap();
        let b = prepare_reference(&bytes, 2048, u64::MAX).unwrap();
        assert!(a.resized);
        assert_eq!((a.width, a.height), (2048, 1024));
        assert_eq!(a.format, ImageFormat::Png);
        assert_eq!(a.bytes, b.bytes, "deterministic");
        let small = prepare_reference(&png(300, 200), 100, u64::MAX).unwrap();
        assert_eq!((small.width, small.height), (100, 67));
        let tight = prepare_reference(&png(300, 200), 2048, 2000).unwrap();
        assert!(tight.resized && (tight.bytes.len() <= 2000 || tight.width <= 64));
    }

    #[test]
    fn fr_ref_22_corrupt_input_is_an_error() {
        assert!(prepare_reference(b"not an image", 2048, u64::MAX).is_err());
    }
}

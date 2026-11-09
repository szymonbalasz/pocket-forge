//! An image file as RGB8 rows (ADR 0004 "pixel format"): non-interlaced PNGs are streamed in row
//! bands, so the whole image is never in memory; JPEG, WebP and interlaced PNGs are decoded whole,
//! once, with explicit limits. Alpha is composited over the flatten colour (`FLATTEN_BG`, FR-GEN-12),
//! grey is expanded, 16-bit is stripped to 8-bit.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use crate::probe::{ImageFormat, MAX_SIDE, ProbeError, probe_file};

/// Largest decode allocation: a 16k RGBA8 image plus headroom.
const MAX_ALLOC: u64 = 1_200 * 1024 * 1024;

/// Decode limits for whole-image decodes (≤ 16384 per side, bounded allocation).
pub(crate) fn limits() -> image::Limits {
    let mut l = image::Limits::default();
    l.max_image_width = Some(MAX_SIDE);
    l.max_image_height = Some(MAX_SIDE);
    l.max_alloc = Some(MAX_ALLOC);
    l
}

/// Decodes a whole file with the app's limits.
pub(crate) fn decode_whole(path: &Path) -> Result<image::DynamicImage, ProbeError> {
    // The header probe reports unsupported, animated and too-large files with their sizes.
    let format = probe_file(path)?.format;
    let mut reader = image::ImageReader::with_format(
        BufReader::new(File::open(path)?),
        match format {
            ImageFormat::Png => image::ImageFormat::Png,
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
            ImageFormat::Webp => image::ImageFormat::WebP,
        },
    );
    reader.limits(limits());
    reader.decode().map_err(|e| match e {
        image::ImageError::IoError(io) => ProbeError::Io(io),
        other => ProbeError::data(other),
    })
}

/// Decodes a whole file to RGB8 (PRJ `DecodedOriginal`, FR-PRJ-42): non-interlaced PNGs are
/// streamed in bands, so no RGBA intermediate is held; other files decode once with the limits.
pub fn decode_rgb8(path: &Path) -> Result<image::RgbImage, ProbeError> {
    let src = RgbSource::open(path)?;
    let (w, h) = (src.width, src.height);
    let mut buf = Vec::new();
    buf.try_reserve_exact(w as usize * h as usize * 3)
        .map_err(|_| ProbeError::data("out of memory"))?;
    src.for_each_band::<ProbeError>(512, |_, rows| {
        buf.extend_from_slice(rows);
        Ok(())
    })?;
    image::RgbImage::from_raw(w, h, buf)
        .ok_or_else(|| ProbeError::Corrupt("the image data is truncated".into()))
}

enum Kind {
    /// Streamed: the file is re-read on every pass.
    Png(PathBuf),
    /// Decoded once.
    Whole(Vec<u8>),
}

/// An RGB8 image source.
pub struct RgbSource {
    pub width: u32,
    pub height: u32,
    kind: Kind,
}

impl RgbSource {
    pub fn open(path: &Path) -> Result<Self, ProbeError> {
        // Unsupported, animated and too-large files fail here, from the header alone.
        let info = probe_file(path)?;
        if info.format == ImageFormat::Png && !png_reader(path)?.info().interlaced {
            return Ok(RgbSource {
                width: info.width,
                height: info.height,
                kind: Kind::Png(path.to_path_buf()),
            });
        }
        let img = crate::alpha::to_rgb8_flattened(decode_whole(path)?);
        Ok(RgbSource {
            width: img.width(),
            height: img.height(),
            kind: Kind::Whole(img.into_raw()),
        })
    }

    /// Calls `f(y0, rows)` for consecutive bands of `band` rows (the last may be shorter).
    /// `rows` is `n × width × 3` bytes.
    pub fn for_each_band<E: From<ProbeError>>(
        &self,
        band: u32,
        mut f: impl FnMut(u32, &[u8]) -> Result<(), E>,
    ) -> Result<(), E> {
        let stride = self.width as usize * 3;
        match &self.kind {
            Kind::Whole(buf) => {
                for (i, rows) in buf.chunks(stride * band as usize).enumerate() {
                    f(i as u32 * band, rows)?;
                }
                Ok(())
            }
            Kind::Png(path) => {
                let mut reader = png_reader(path)?;
                let (color, _) = reader.output_color_type();
                let mut buf = Vec::with_capacity(stride * band as usize);
                let mut y0 = 0;
                loop {
                    let row = reader
                        .next_row()
                        .map_err(|e| png_error(e, ProbeError::data))?;
                    let Some(row) = row else { break };
                    push_rgb(&mut buf, row.data(), color)?;
                    if buf.len() == stride * band as usize {
                        f(y0, &buf)?;
                        y0 += band;
                        buf.clear();
                    }
                }
                if !buf.is_empty() {
                    f(y0, &buf)?;
                    y0 += (buf.len() / stride) as u32;
                }
                if y0 != self.height {
                    return Err(ProbeError::Corrupt("the image data is truncated".into()).into());
                }
                Ok(())
            }
        }
    }
}

fn png_reader(path: &Path) -> Result<png::Reader<BufReader<File>>, ProbeError> {
    let mut dec = png::Decoder::new(BufReader::new(File::open(path)?));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    dec.set_limits(png::Limits {
        bytes: MAX_ALLOC as usize,
    });
    dec.read_info()
        .map_err(|e| png_error(e, ProbeError::header))
}

/// `part` says whether the header or the data failed (BL-27: a broken body is not a header error).
fn png_error(e: png::DecodingError, part: fn(png::DecodingError) -> ProbeError) -> ProbeError {
    match e {
        png::DecodingError::IoError(io) => ProbeError::Io(io),
        other => part(other),
    }
}

fn push_rgb(out: &mut Vec<u8>, row: &[u8], color: png::ColorType) -> Result<(), ProbeError> {
    use png::ColorType as C;
    match color {
        C::Rgb => out.extend_from_slice(row),
        C::Rgba => row
            .as_chunks::<4>()
            .0
            .iter()
            .for_each(|p| out.extend_from_slice(&crate::alpha::flatten_px(*p))),
        C::Grayscale => row.iter().for_each(|&g| out.extend_from_slice(&[g, g, g])),
        C::GrayscaleAlpha => row
            .as_chunks::<2>()
            .0
            .iter()
            .for_each(|p| out.extend_from_slice(&crate::alpha::flatten_grey_alpha(p[0], p[1]))),
        C::Indexed => return Err(ProbeError::data("unexpanded palette row")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid PNG whose `IDAT` body is broken: the header and `IEND` are intact (walk D5).
    fn body_corrupt_png(path: &Path) {
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, 16, 8);
            enc.set_color(png::ColorType::Rgb);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[90; 16 * 8 * 3]).unwrap();
        }
        let idat = bytes.windows(4).position(|w| w == b"IDAT").unwrap() + 4;
        bytes[idat..idat + 6].copy_from_slice(&[0xFF; 6]);
        std::fs::write(path, bytes).unwrap();
    }

    /// FR-GEN-12 golden: the streamed PNG path, the whole-image path (WebP) and grey + alpha all
    /// composite alpha over the flatten colour.
    #[test]
    fn fr_gen_12_decode_rgb8_flattens_alpha_over_mid_grey() {
        let tmp = tempfile::tempdir().unwrap();
        let img = image::RgbaImage::from_fn(4, 2, |x, _| match x {
            0 => image::Rgba([200, 10, 10, 0]),
            1 => image::Rgba([255, 0, 128, 128]),
            _ => image::Rgba([1, 2, 3, 255]),
        });
        let png = tmp.path().join("a.png");
        img.save(&png).unwrap();
        let webp = tmp.path().join("a.webp");
        image::DynamicImage::ImageRgba8(img.clone())
            .save_with_format(&webp, image::ImageFormat::WebP)
            .unwrap();
        for path in [&png, &webp] {
            let rgb = decode_rgb8(path).unwrap();
            assert_eq!(
                rgb.get_pixel(0, 0).0,
                crate::FLATTEN_BG,
                "{}",
                path.display()
            );
            assert_eq!(rgb.get_pixel(1, 0).0, [192, 64, 128], "{}", path.display());
            assert_eq!(rgb.get_pixel(3, 1).0, [1, 2, 3], "{}", path.display());
        }
        let la = tmp.path().join("la.png");
        image::GrayAlphaImage::from_pixel(2, 2, image::LumaA([0, 0]))
            .save(&la)
            .unwrap();
        assert_eq!(
            decode_rgb8(&la).unwrap().get_pixel(1, 1).0,
            crate::FLATTEN_BG
        );
    }

    /// BL-27: a broken body says the image data is unreadable, not the header.
    #[test]
    fn bl_27_a_broken_png_body_is_a_data_error_not_a_header_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("broken.png");
        body_corrupt_png(&path);
        assert!(
            crate::probe_file_complete(&path).is_ok(),
            "the header is fine"
        );
        let err = decode_rgb8(&path).unwrap_err();
        assert!(matches!(err, ProbeError::Corrupt(_)), "{err:?}");
        let text = err.to_string();
        assert!(text.starts_with("image data is unreadable: "), "{text}");
        assert!(!text.contains("header"), "{text}");
    }

    #[test]
    fn bl_27_a_broken_png_header_is_a_header_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("broken.png");
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R', 0xFF, 0xFF]);
        std::fs::write(&path, bytes).unwrap();
        let text = crate::probe_file(&path).unwrap_err().to_string();
        assert!(text.starts_with("image header is unreadable: "), "{text}");
    }
}

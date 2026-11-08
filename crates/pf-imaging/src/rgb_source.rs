//! An image file as RGB8 rows (ADR 0004 "pixel format"): non-interlaced PNGs are streamed in row
//! bands, so the whole image is never in memory; JPEG, WebP and interlaced PNGs are decoded whole,
//! once, with explicit limits. Alpha is dropped, grey is expanded, 16-bit is stripped to 8-bit.

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
        other => ProbeError::Corrupt(other.to_string()),
    })
}

/// Decodes a whole file to RGB8 (PRJ `DecodedOriginal`, FR-PRJ-42): non-interlaced PNGs are
/// streamed in bands, so no RGBA intermediate is held; other files decode once with the limits.
pub fn decode_rgb8(path: &Path) -> Result<image::RgbImage, ProbeError> {
    let src = RgbSource::open(path)?;
    let (w, h) = (src.width, src.height);
    let mut buf = Vec::new();
    buf.try_reserve_exact(w as usize * h as usize * 3)
        .map_err(|_| ProbeError::Corrupt("out of memory".into()))?;
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
        let img = decode_whole(path)?.into_rgb8();
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
                    let row = reader.next_row().map_err(png_error)?;
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
    dec.read_info().map_err(png_error)
}

fn png_error(e: png::DecodingError) -> ProbeError {
    match e {
        png::DecodingError::IoError(io) => ProbeError::Io(io),
        other => ProbeError::Corrupt(other.to_string()),
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
            .for_each(|p| out.extend_from_slice(&p[..3])),
        C::Grayscale => row.iter().for_each(|&g| out.extend_from_slice(&[g, g, g])),
        C::GrayscaleAlpha => row
            .as_chunks::<2>()
            .0
            .iter()
            .for_each(|p| out.extend_from_slice(&[p[0], p[0], p[0]])),
        C::Indexed => return Err(ProbeError::Corrupt("unexpanded palette row".into())),
    }
    Ok(())
}

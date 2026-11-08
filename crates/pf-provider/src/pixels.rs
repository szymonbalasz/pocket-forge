//! Small PNG helpers shared by the fake, the mask conversion and the limit checks.

use std::io::Cursor;

pub(crate) struct Rgb8 {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

pub(crate) struct Grey8 {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Width and height from the PNG header, without decoding pixels.
pub(crate) fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let reader = decoder.read_info().ok()?;
    let info = reader.info();
    Some((info.width, info.height))
}

fn decode(bytes: &[u8]) -> Option<(u32, u32, png::ColorType, png::BitDepth, Vec<u8>)> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let depth = reader.info().bit_depth;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, info.color_type, depth, buf))
}

/// Any 8-bit PNG as RGB (alpha dropped). Indexed images are expanded by `normalize_to_color8`.
pub(crate) fn decode_rgb8(bytes: &[u8]) -> Option<Rgb8> {
    let (width, height, color, _, buf) = decode(bytes)?;
    let data = match color {
        png::ColorType::Rgb => buf,
        png::ColorType::Rgba => buf
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g]).collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    Some(Rgb8 {
        width,
        height,
        data,
    })
}

/// The canonical mask: an 8-bit greyscale (L8) PNG, nothing else (FR-PRV-12).
pub(crate) fn decode_grey8(bytes: &[u8]) -> Option<Grey8> {
    let (width, height, color, depth, data) = decode(bytes)?;
    (color == png::ColorType::Grayscale && depth == png::BitDepth::Eight).then_some(Grey8 {
        width,
        height,
        data,
    })
}

/// An image file format a reference may be sent in (FR-PRV-33).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Webp => "image/webp",
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
        }
    }
}

/// The format from the file's magic bytes; `None` for anything else.
pub(crate) fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else {
        None
    }
}

pub(crate) use crate::fixtures::encode_png;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prv_33_sniffs_png_jpeg_and_webp_only() {
        assert_eq!(
            sniff(&encode_png(1, 1, png::ColorType::Grayscale, &[0])),
            Some(ImageFormat::Png)
        );
        assert_eq!(sniff(b"\xFF\xD8\xFF\xE0rest"), Some(ImageFormat::Jpeg));
        assert_eq!(sniff(b"RIFF\x10\0\0\0WEBPVP8 "), Some(ImageFormat::Webp));
        for bad in [&b""[..], b"GIF89a", b"RIFF\x10\0\0\0WAVE", b"\x89PN"] {
            assert_eq!(sniff(bad), None, "{bad:?}");
        }
    }
}

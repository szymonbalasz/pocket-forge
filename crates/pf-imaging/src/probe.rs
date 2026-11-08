//! Format sniffing and header probing (no full decode).

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Largest supported side in px (#11, FR-PRJ-06).
pub const MAX_SIDE: u32 = 16384;

/// Bytes read for sniffing and animation detection: enough for the PNG chunks before `IDAT`
/// in practice, and for the WebP `VP8X` header.
const HEAD_BYTES: usize = 64 * 1024;

/// Input formats the app accepts (KQ-28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    /// Manifest spelling (`png|jpeg|webp`, manifest-schema §4.2).
    pub fn as_str(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Webp => "webp",
        }
    }

    /// File extension the app uses when it names a file of this format.
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
        }
    }

    /// MIME type for serving the bytes.
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Webp => "image/webp",
        }
    }

    fn to_image(self) -> image::ImageFormat {
        match self {
            ImageFormat::Png => image::ImageFormat::Png,
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
            ImageFormat::Webp => image::ImageFormat::WebP,
        }
    }
}

/// Header facts about an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
}

/// Why a file can't be used as an original (the FR-PRJ-06 skip reasons).
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("not a PNG, JPEG or WebP image")]
    Unsupported,
    #[error("animated images are not supported")]
    Animated,
    #[error("image is {width}x{height} px; the maximum is {MAX_SIDE} px per side")]
    TooLarge { width: u32, height: u32 },
    /// The whole message: built by [`ProbeError::header`], [`ProbeError::data`] or
    /// [`ProbeError::encode`], so it says which part failed (BL-27).
    #[error("{0}")]
    Corrupt(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl ProbeError {
    /// The header (format, size) can't be read.
    pub fn header(e: impl std::fmt::Display) -> Self {
        Self::Corrupt(format!("image header is unreadable: {e}"))
    }

    /// The header is fine but the pixel data can't be decoded (e.g. a broken PNG body).
    pub fn data(e: impl std::fmt::Display) -> Self {
        Self::Corrupt(format!("image data is unreadable: {e}"))
    }

    /// A decoded image could not be encoded again (thumbnail, sent copy).
    pub fn encode(e: impl std::fmt::Display) -> Self {
        Self::Corrupt(format!("image could not be re-encoded: {e}"))
    }
}

/// Detects the format from magic bytes. Extensions are never trusted.
pub fn sniff(head: &[u8]) -> Option<ImageFormat> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else {
        None
    }
}

/// Extensions of the accepted formats. Never trusted for the format itself, only to tell a
/// broken image (`corrupt`) from another kind of file (`unsupported`) (BL-05).
const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// True if the file name has one of the accepted image extensions.
pub fn has_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Probes an image file: format, size, animation and the size limit. Reads only the header.
/// Unknown magic bytes are `unsupported`, or `corrupt` when the name says it's an image (BL-05).
pub fn probe_file(path: &Path) -> Result<ImageInfo, ProbeError> {
    let mut head = Vec::with_capacity(HEAD_BYTES);
    File::open(path)?
        .take(HEAD_BYTES as u64)
        .read_to_end(&mut head)?;
    let format = sniff(&head).ok_or_else(|| {
        if has_image_extension(path) {
            ProbeError::Corrupt("the file is not a valid PNG, JPEG or WebP image".into())
        } else {
            ProbeError::Unsupported
        }
    })?;
    check_animated(format, &head)?;
    let reader =
        image::ImageReader::with_format(BufReader::new(File::open(path)?), format.to_image());
    let (width, height) = reader.into_dimensions().map_err(ProbeError::header)?;
    finish(format, width, height)
}

/// [`probe_file`] plus the cheap end-of-file check (R33): a truncated file is `corrupt`
/// without a full decode (16k decodes are slow).
pub fn probe_file_complete(path: &Path) -> Result<ImageInfo, ProbeError> {
    let info = probe_file(path)?;
    check_complete(path, info.format)?;
    Ok(info)
}

/// Bytes read from the end of a JPEG to find its end-of-image marker (encoders may pad).
const JPEG_TAIL_BYTES: u64 = 4096;

/// End-of-file check (R33, BL-01): a PNG ends with its `IEND` chunk, a JPEG has an end-of-image
/// marker near the end, a WebP is at least as long as its RIFF header says.
pub fn check_complete(path: &Path, format: ImageFormat) -> Result<(), ProbeError> {
    use std::io::{Seek, SeekFrom};
    let truncated = || ProbeError::Corrupt("the image data is truncated".into());
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    match format {
        ImageFormat::Png => {
            const IEND: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82];
            if len < 12 {
                return Err(truncated());
            }
            let mut tail = [0u8; 12];
            f.seek(SeekFrom::End(-12))?;
            f.read_exact(&mut tail)?;
            if tail != IEND {
                return Err(truncated());
            }
        }
        ImageFormat::Jpeg => {
            let n = len.min(JPEG_TAIL_BYTES);
            let mut tail = vec![0u8; n as usize];
            f.seek(SeekFrom::End(-(n as i64)))?;
            f.read_exact(&mut tail)?;
            if !tail.windows(2).any(|w| w == [0xFF, 0xD9]) {
                return Err(truncated());
            }
        }
        ImageFormat::Webp => {
            let mut head = [0u8; 8];
            f.read_exact(&mut head).map_err(|_| truncated())?;
            let riff = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
            if len < u64::from(riff) + 8 {
                return Err(truncated());
            }
        }
    }
    Ok(())
}

/// Probes in-memory image bytes (e.g. pasted data).
pub fn probe_bytes(bytes: &[u8]) -> Result<ImageInfo, ProbeError> {
    let format = sniff(bytes).ok_or(ProbeError::Unsupported)?;
    check_animated(format, bytes)?;
    let reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format.to_image());
    let (width, height) = reader.into_dimensions().map_err(ProbeError::header)?;
    finish(format, width, height)
}

fn finish(format: ImageFormat, width: u32, height: u32) -> Result<ImageInfo, ProbeError> {
    if width == 0 || height == 0 {
        return Err(ProbeError::header("zero-sized image"));
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(ProbeError::TooLarge { width, height });
    }
    Ok(ImageInfo {
        format,
        width,
        height,
    })
}

/// PNG sizes are refused from the `IHDR` alone, before any decoder runs (a 16k+ file may be a
/// header-only fixture, or simply huge).
fn check_png_ihdr_size(head: &[u8]) -> Result<(), ProbeError> {
    if head.len() >= 24 && &head[12..16] == b"IHDR" {
        let width = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
        let height = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(ProbeError::TooLarge { width, height });
        }
    }
    Ok(())
}

fn check_animated(format: ImageFormat, head: &[u8]) -> Result<(), ProbeError> {
    if format == ImageFormat::Png {
        check_png_ihdr_size(head)?;
    }
    let animated = match format {
        ImageFormat::Png => png_has_actl(head),
        // A VP8X chunk at offset 12; the animation bit (0x02) is in its flags byte at offset 20.
        ImageFormat::Webp => head.len() > 20 && &head[12..16] == b"VP8X" && head[20] & 0x02 != 0,
        ImageFormat::Jpeg => false,
    };
    if animated {
        Err(ProbeError::Animated)
    } else {
        Ok(())
    }
}

/// APNG files carry an `acTL` chunk before the first `IDAT`.
fn png_has_actl(head: &[u8]) -> bool {
    let mut pos = 8;
    while pos + 8 <= head.len() {
        let len = u32::from_be_bytes([head[pos], head[pos + 1], head[pos + 2], head[pos + 3]]);
        let kind = &head[pos + 4..pos + 8];
        if kind == b"acTL" {
            return true;
        }
        if kind == b"IDAT" {
            return false;
        }
        pos = pos.saturating_add(12).saturating_add(len as usize);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        let mut writer = enc.write_header().unwrap();
        writer
            .write_image_data(&vec![128u8; (w * h * 3) as usize])
            .unwrap();
        writer.finish().unwrap();
        out
    }

    /// A PNG header claiming the given size, with no pixel data (the 16385 px fixture).
    fn png_header_only(w: u32, h: u32) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = b"IHDR".to_vec();
        chunk.extend_from_slice(&w.to_be_bytes());
        chunk.extend_from_slice(&h.to_be_bytes());
        chunk.extend_from_slice(&[8, 2, 0, 0, 0]);
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc32(&chunk).to_be_bytes());
        out
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn fr_prj_06_accepts_png_and_reports_size() {
        let info = probe_bytes(&png_bytes(40, 30)).unwrap();
        assert_eq!(
            info,
            ImageInfo {
                format: ImageFormat::Png,
                width: 40,
                height: 30
            }
        );
    }

    #[test]
    fn fr_prj_06_rejects_unknown_magic_as_unsupported() {
        assert!(matches!(
            probe_bytes(b"8BPS\0\x01psd"),
            Err(ProbeError::Unsupported)
        ));
    }

    #[test]
    fn fr_prj_06_rejects_over_16384_px_as_too_large() {
        let err = probe_bytes(&png_header_only(16385, 10));
        assert!(
            matches!(err, Err(ProbeError::TooLarge { width: 16385, .. })),
            "{err:?}"
        );
    }

    #[test]
    fn fr_prj_06_rejects_apng_as_animated() {
        let mut bytes = png_bytes(4, 4);
        // An acTL chunk right after IHDR (8 signature + 25 IHDR bytes).
        let actl = [
            0, 0, 0, 8, b'a', b'c', b'T', b'L', 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        bytes.splice(33..33, actl);
        assert!(matches!(probe_bytes(&bytes), Err(ProbeError::Animated)));
    }

    #[test]
    fn fr_prj_06_rejects_animated_webp() {
        let mut bytes = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        bytes.extend_from_slice(&[10, 0, 0, 0, 0x02, 0, 0, 0]);
        assert!(matches!(probe_bytes(&bytes), Err(ProbeError::Animated)));
    }

    #[test]
    fn fr_prj_06_truncated_header_is_corrupt() {
        let bytes = &png_bytes(8, 8)[..20];
        assert!(matches!(probe_bytes(bytes), Err(ProbeError::Corrupt(_))));
    }

    #[test]
    fn bl_05_garbage_named_png_is_corrupt_and_psd_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("broken.PNG");
        std::fs::write(&png, b"just some text").unwrap();
        assert!(matches!(probe_file(&png), Err(ProbeError::Corrupt(_))));
        let psd = dir.path().join("art.psd");
        std::fs::write(&psd, b"8BPS\0\x01rest").unwrap();
        assert!(matches!(probe_file(&psd), Err(ProbeError::Unsupported)));
    }

    #[test]
    fn bl_01_fr_prj_06_truncated_files_fail_the_end_of_file_check() {
        let dir = tempfile::tempdir().unwrap();
        let full = png_bytes(64, 64);
        let ok = dir.path().join("ok.png");
        std::fs::write(&ok, &full).unwrap();
        probe_file_complete(&ok).unwrap();
        let cut = dir.path().join("cut.png");
        std::fs::write(&cut, &full[..full.len() - 20]).unwrap();
        probe_file(&cut).expect("the header alone is fine");
        assert!(matches!(
            probe_file_complete(&cut),
            Err(ProbeError::Corrupt(_))
        ));

        let mut jpeg = Vec::new();
        image::RgbImage::new(16, 16)
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        let j = dir.path().join("a.jpg");
        std::fs::write(&j, &jpeg).unwrap();
        probe_file_complete(&j).unwrap();
        std::fs::write(&j, &jpeg[..jpeg.len() - 2]).unwrap();
        assert!(matches!(
            probe_file_complete(&j),
            Err(ProbeError::Corrupt(_))
        ));

        let mut webp = Vec::new();
        image::RgbaImage::new(8, 8)
            .write_to(
                &mut std::io::Cursor::new(&mut webp),
                image::ImageFormat::WebP,
            )
            .unwrap();
        let w = dir.path().join("a.webp");
        std::fs::write(&w, &webp).unwrap();
        probe_file_complete(&w).unwrap();
        std::fs::write(&w, &webp[..webp.len() - 4]).unwrap();
        assert!(matches!(
            probe_file_complete(&w),
            Err(ProbeError::Corrupt(_))
        ));
    }

    #[test]
    fn sniff_reads_magic_bytes() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(ImageFormat::Jpeg));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(ImageFormat::Webp));
        assert_eq!(sniff(b"GIF89a"), None);
    }

    #[test]
    fn probe_file_reads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.bin");
        std::fs::write(&path, png_bytes(3, 5)).unwrap();
        assert_eq!(probe_file(&path).unwrap().height, 5);
    }
}

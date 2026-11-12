use std::sync::Arc;

use image::RgbaImage;

use super::*;
use crate::compose::{Layer, LayerStack, Placement, flatten_full};
use crate::test_support::{psnr, smooth_rgb};

fn go() -> impl FnMut(Phase, f32) -> ControlFlow<()> {
    |_, _| ControlFlow::Continue(())
}

fn stack(w: u32, h: u32) -> LayerStack {
    let base = Arc::new(smooth_rgb(w, h));
    let patch = Arc::new(RgbaImage::from_fn(60, 40, |x, y| {
        image::Rgba([
            200,
            (x * 4) as u8,
            (y * 6) as u8,
            if x < 10 { 0 } else { 220 },
        ])
    }));
    let layer = Layer {
        image: patch,
        placement: Placement::at(20, (h / 2) as i64, 60, 40),
        opacity: 1.0,
        visible: true,
    };
    LayerStack::new(base, &[layer]).unwrap()
}

fn flat(stack: &LayerStack) -> RgbImage {
    flatten_full::<ComposeError, _>(stack, STRIP_H, |_| Ok(())).unwrap()
}

fn png_of(stack: &LayerStack) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rows = StripFlattener::new(stack, STRIP_H).unwrap();
    encode_png(&mut rows, &mut out, Phase::Encode, &mut go()).unwrap();
    out
}

fn jpeg_of(rows: &mut dyn RowSource) -> Vec<u8> {
    let mut out = Vec::new();
    encode_jpeg(rows, &mut out, Phase::Encode, &mut go()).unwrap();
    out
}

fn decode(bytes: &[u8]) -> RgbImage {
    image::load_from_memory(bytes).unwrap().to_rgb8()
}

/// PNG chunk types in file order.
fn png_chunks(bytes: &[u8]) -> Vec<String> {
    let mut pos = 8;
    let mut out = Vec::new();
    while pos + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        out.push(String::from_utf8_lossy(&bytes[pos + 4..pos + 8]).into_owned());
        pos += 12 + len;
    }
    out
}

/// JPEG marker bytes up to the start of scan.
fn jpeg_markers(bytes: &[u8]) -> Vec<u8> {
    assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
    let mut pos = 2;
    let mut out = vec![0xD8];
    while pos + 4 <= bytes.len() && bytes[pos] == 0xFF {
        let marker = bytes[pos + 1];
        out.push(marker);
        if marker == 0xDA {
            break;
        }
        let len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        pos += 2 + len;
    }
    out
}

#[test]
fn fr_exp_09_png_stream_decodes_pixel_equal_to_the_flatten() {
    let stack = stack(300, 1100);
    assert_eq!(decode(&png_of(&stack)), flat(&stack));
}

#[test]
fn fr_exp_10_png_has_only_ihdr_idat_iend() {
    let chunks = png_chunks(&png_of(&stack(64, 600)));
    assert_eq!(chunks.first().map(String::as_str), Some("IHDR"));
    assert_eq!(chunks.last().map(String::as_str), Some("IEND"));
    assert!(
        chunks[1..chunks.len() - 1].iter().all(|c| c == "IDAT"),
        "{chunks:?}"
    );
}

#[test]
fn fr_exp_10_jpeg_has_only_the_jfif_app0_segment() {
    let bytes = jpeg_of(&mut StripFlattener::new(&stack(64, 600), STRIP_H).unwrap());
    let markers = jpeg_markers(&bytes);
    let apps: Vec<u8> = markers
        .iter()
        .copied()
        .filter(|m| (0xE0..=0xEF).contains(m))
        .collect();
    assert_eq!(apps, [0xE0], "{markers:x?}");
    assert!(!markers.contains(&0xFE), "no COM segment");
    assert_eq!(&bytes[6..11], b"JFIF\0");
    assert_eq!(&bytes[bytes.len() - 2..], &[0xFF, 0xD9]);
}

#[test]
fn fr_exp_09_jpeg_stream_is_within_40_db_of_the_png() {
    // Odd height: the encoder repeats the last row past the end of the image.
    let stack = stack(301, 1101);
    let reference = flat(&stack);
    let jpeg = decode(&jpeg_of(&mut StripFlattener::new(&stack, STRIP_H).unwrap()));
    assert_eq!(jpeg.dimensions(), reference.dimensions());
    let db = psnr(&jpeg, &reference);
    assert!(db >= 40.0, "PSNR {db:.1} dB");
}

#[test]
fn fr_exp_09_full_buffer_rows_encode_like_the_stream() {
    let stack = stack(200, 1030);
    let image = flat(&stack);
    let mut png = Vec::new();
    encode_png(
        &mut ImageRows::new(&image),
        &mut png,
        Phase::Encode,
        &mut go(),
    )
    .unwrap();
    assert_eq!(decode(&png), image);
    let jpeg_stream = jpeg_of(&mut StripFlattener::new(&stack, STRIP_H).unwrap());
    let jpeg_buffer = jpeg_of(&mut ImageRows::new(&image));
    assert_eq!(jpeg_stream, jpeg_buffer);
}

#[test]
fn fr_exp_19_progress_is_reported_per_strip_up_to_one() {
    let stack = stack(50, 1100);
    for jpeg in [false, true] {
        let mut seen = Vec::new();
        let mut rows = StripFlattener::new(&stack, STRIP_H).unwrap();
        let mut progress = |phase, f| {
            seen.push((phase, f));
            ControlFlow::Continue(())
        };
        if jpeg {
            encode_jpeg(&mut rows, std::io::sink(), Phase::Compose, &mut progress).unwrap();
        } else {
            encode_png(&mut rows, std::io::sink(), Phase::Compose, &mut progress).unwrap();
        }
        let fractions: Vec<f32> = seen.iter().map(|s| s.1).collect();
        assert!(seen.iter().all(|s| s.0 == Phase::Compose));
        assert_eq!(fractions.len(), 3, "jpeg={jpeg}: {fractions:?}");
        assert!(fractions.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(fractions.last(), Some(&1.0));
    }
}

/// Counts the bands a source hands out.
struct Counting<'a> {
    inner: StripFlattener<'a>,
    pulled: usize,
}

impl RowSource for Counting<'_> {
    fn dimensions(&self) -> (u32, u32) {
        self.inner.dimensions()
    }
    fn next_rows(&mut self) -> Result<Option<Strip<'_>>, EncodeError> {
        self.pulled += 1;
        self.inner.next_rows()
    }
}

#[test]
fn fr_exp_20_cancel_stops_png_and_jpeg_between_strips() {
    let stack = stack(80, 2000);
    for jpeg in [false, true] {
        let mut rows = Counting {
            inner: StripFlattener::new(&stack, STRIP_H).unwrap(),
            pulled: 0,
        };
        let mut out = Vec::new();
        let mut cancel = |_, _| ControlFlow::Break(());
        let r = if jpeg {
            encode_jpeg(&mut rows, &mut out, Phase::Encode, &mut cancel)
        } else {
            encode_png(&mut rows, &mut out, Phase::Encode, &mut cancel)
        };
        assert!(
            matches!(r, Err(EncodeError::Canceled)),
            "jpeg={jpeg}: {r:?}"
        );
        assert_eq!(rows.pulled, 1, "jpeg={jpeg}: stopped after the first strip");
    }
}

/// A writer that fails like a full disk after `budget` bytes.
struct FullDisk {
    budget: usize,
}

impl Write for FullDisk {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.budget {
            return Err(io::Error::new(io::ErrorKind::StorageFull, "disk full"));
        }
        self.budget -= buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn write_errors_keep_their_io_kind() {
    let stack = stack(300, 1100);
    for jpeg in [false, true] {
        let mut rows = StripFlattener::new(&stack, STRIP_H).unwrap();
        let out = FullDisk { budget: 2000 };
        let r = if jpeg {
            encode_jpeg(&mut rows, out, Phase::Encode, &mut go())
        } else {
            encode_png(&mut rows, out, Phase::Encode, &mut go())
        };
        match r {
            Err(EncodeError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::StorageFull),
            other => panic!("jpeg={jpeg}: {other:?}"),
        }
    }
}

/// A source that claims a size and fails on read.
struct Claims(u32, u32);

impl RowSource for Claims {
    fn dimensions(&self) -> (u32, u32) {
        (self.0, self.1)
    }
    fn next_rows(&mut self) -> Result<Option<Strip<'_>>, EncodeError> {
        Err(EncodeError::OutOfMemory)
    }
}

#[test]
fn fr_exp_23_source_errors_surface_from_both_encoders() {
    for jpeg in [false, true] {
        let r = if jpeg {
            encode_jpeg(
                &mut Claims(10, 10),
                std::io::sink(),
                Phase::Encode,
                &mut go(),
            )
        } else {
            encode_png(
                &mut Claims(10, 10),
                std::io::sink(),
                Phase::Encode,
                &mut go(),
            )
        };
        assert!(
            matches!(r, Err(EncodeError::OutOfMemory)),
            "jpeg={jpeg}: {r:?}"
        );
    }
}

#[test]
fn jpeg_refuses_sides_over_65535() {
    let r = encode_jpeg(
        &mut Claims(65536, 4),
        std::io::sink(),
        Phase::Encode,
        &mut go(),
    );
    assert!(matches!(
        r,
        Err(EncodeError::TooLarge {
            format: ImageFormat::Jpeg,
            limit: 65535
        })
    ));
}

#[test]
fn fr_exp_32_png_rgba_keeps_alpha_and_reports_progress() {
    let img = RgbaImage::from_fn(70, 1100, |x, y| {
        image::Rgba([
            (x * 3) as u8,
            (y % 256) as u8,
            9,
            if x < 35 { 0 } else { 255 },
        ])
    });
    let mut out = Vec::new();
    let mut reports = Vec::new();
    encode_png_rgba(&img, &mut out, &mut |p, f| {
        reports.push((p, f));
        ControlFlow::Continue(())
    })
    .unwrap();
    let back = image::load_from_memory(&out).unwrap();
    assert!(back.color().has_alpha());
    assert_eq!(back.to_rgba8(), img);
    // One report per band of STRIP_H rows, ending at 1.
    assert_eq!(reports.len(), 3);
    assert_eq!(reports.last(), Some(&(Phase::Encode, 1.0)));
    // A cancel stops it.
    let err = encode_png_rgba(&img, Vec::new(), &mut |_, _| ControlFlow::Break(())).unwrap_err();
    assert!(matches!(err, EncodeError::Canceled));
}

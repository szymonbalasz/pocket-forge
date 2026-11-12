use std::cell::RefCell;

use super::*;
use crate::test_support::{psnr, smooth_rgb};

fn go() -> impl FnMut(Phase, f32) -> ControlFlow<()> {
    |_, _| ControlFlow::Continue(())
}

/// RIFF chunk ids after the `WEBP` form type.
fn riff_chunks(bytes: &[u8]) -> Vec<String> {
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WEBP");
    let mut pos = 12;
    let mut out = Vec::new();
    while pos + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        out.push(String::from_utf8_lossy(&bytes[pos..pos + 4]).into_owned());
        pos += 8 + len + (len & 1);
    }
    out
}

#[test]
fn fr_exp_10_lossy_webp_q90_is_close_and_has_no_metadata() {
    let img = smooth_rgb(320, 240);
    let out = encode_webp(&img, &mut go()).unwrap();
    assert!(!out.lossless_fallback);
    assert_eq!(riff_chunks(&out.bytes), ["VP8 "]);
    let decoded = image::load_from_memory(&out.bytes).unwrap().to_rgb8();
    assert_eq!(decoded.dimensions(), (320, 240));
    let db = psnr(&decoded, &img);
    assert!(db >= 40.0, "PSNR {db:.1} dB");
}

#[test]
fn fr_exp_12_lossless_mode_round_trips_exactly() {
    let img = smooth_rgb(97, 61);
    let bytes = LibWebp
        .encode(&img, WebpMode::Lossless, &mut |_| ControlFlow::Continue(()))
        .unwrap();
    assert_eq!(riff_chunks(&bytes), ["VP8L"]);
    assert_eq!(image::load_from_memory(&bytes).unwrap().to_rgb8(), img);
}

/// Fails lossy with a partition overflow, then delegates to libwebp.
struct Overflowing {
    calls: RefCell<Vec<WebpMode>>,
}

impl WebpEncoder for Overflowing {
    fn encode(
        &self,
        image: &RgbImage,
        mode: WebpMode,
        progress: &mut dyn FnMut(f32) -> ControlFlow<()>,
    ) -> Result<Vec<u8>, EncodeError> {
        self.calls.borrow_mut().push(mode);
        match mode {
            WebpMode::Lossy { .. } => Err(EncodeError::PartitionOverflow),
            WebpMode::Lossless => LibWebp.encode(image, mode, progress),
        }
    }
}

#[test]
fn fr_exp_12_partition_overflow_retries_once_as_lossless() {
    let img = smooth_rgb(40, 30);
    let enc = Overflowing {
        calls: RefCell::new(Vec::new()),
    };
    let out = encode_webp_with(&enc, &img, &mut go()).unwrap();
    assert!(out.lossless_fallback);
    assert_eq!(
        *enc.calls.borrow(),
        [
            WebpMode::Lossy {
                quality: WEBP_QUALITY
            },
            WebpMode::Lossless
        ]
    );
    assert_eq!(riff_chunks(&out.bytes), ["VP8L"]);
}

#[test]
fn fr_exp_12_other_failures_are_not_retried() {
    struct Broken(RefCell<usize>);
    impl WebpEncoder for Broken {
        fn encode(
            &self,
            _: &RgbImage,
            _: WebpMode,
            _: &mut dyn FnMut(f32) -> ControlFlow<()>,
        ) -> Result<Vec<u8>, EncodeError> {
            *self.0.borrow_mut() += 1;
            Err(EncodeError::Failed("boom".into()))
        }
    }
    let enc = Broken(RefCell::new(0));
    let r = encode_webp_with(&enc, &smooth_rgb(8, 8), &mut go());
    assert!(matches!(r, Err(EncodeError::Failed(_))));
    assert_eq!(*enc.0.borrow(), 1);
}

#[test]
fn fr_exp_11_webp_refuses_sides_over_16383_before_encoding() {
    let img = RgbImage::new(16384, 1);
    let r = encode_webp(&img, &mut go());
    assert!(matches!(
        r,
        Err(EncodeError::TooLarge {
            format: ImageFormat::Webp,
            limit: 16383
        })
    ));
}

#[test]
fn fr_exp_19_webp_progress_comes_from_the_libwebp_hook() {
    let img = smooth_rgb(256, 256);
    let mut seen = Vec::new();
    encode_webp(&img, &mut |phase, f| {
        seen.push((phase, f));
        ControlFlow::Continue(())
    })
    .unwrap();
    assert!(seen.len() >= 3, "{seen:?}");
    assert!(seen.iter().all(|s| s.0 == Phase::Encode));
    assert!(seen.windows(2).all(|w| w[0].1 <= w[1].1));
    assert_eq!(seen.last().map(|s| s.1), Some(1.0));
}

#[test]
fn fr_exp_20_webp_cancel_aborts_through_the_hook() {
    let img = smooth_rgb(256, 256);
    let mut calls = 0;
    let r = encode_webp(&img, &mut |_, _| {
        calls += 1;
        ControlFlow::Break(())
    });
    assert!(matches!(r, Err(EncodeError::Canceled)), "{r:?}");
    assert_eq!(calls, 1);
}

#[test]
fn a_panicking_progress_callback_does_not_unwind_into_libwebp() {
    let img = smooth_rgb(64, 64);
    let r = LibWebp.encode(&img, WebpMode::Lossy { quality: 90.0 }, &mut |_| {
        panic!("progress panicked")
    });
    assert!(matches!(r, Err(EncodeError::Failed(_))), "{r:?}");
}

#[test]
fn fr_exp_32_webp_rgba_keeps_alpha() {
    let img = image::RgbaImage::from_fn(64, 48, |x, y| {
        image::Rgba([
            (x * 3) as u8,
            (y * 5) as u8,
            90,
            if x < 32 { 0 } else { 255 },
        ])
    });
    let out = encode_webp_rgba(&img, &mut go()).unwrap();
    assert!(!out.lossless_fallback);
    let back = image::load_from_memory(&out.bytes).unwrap();
    assert!(back.color().has_alpha());
    let back = back.to_rgba8();
    assert_eq!(back.get_pixel(3, 3)[3], 0);
    assert_eq!(back.get_pixel(60, 3)[3], 255);
    let too_big = image::RgbaImage::new(WEBP_MAX_SIDE + 1, 1);
    assert!(matches!(
        encode_webp_rgba(&too_big, &mut go()),
        Err(EncodeError::TooLarge { .. })
    ));
}

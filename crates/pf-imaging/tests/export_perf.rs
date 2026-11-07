//! Export timings for the §12 budget (OQ-EXP-07). Ignored by default; run in release:
//!
//! ```text
//! node fixtures/gen/gen-fixtures.mjs --size 4200x5600
//! cargo test -p pf-imaging --release --test export_perf -- --ignored --nocapture
//! ```
//!
//! `PF_PERF_IMAGE` overrides the input (e.g. a 16384² map).

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use pf_imaging::{
    ComposeError, ImageRows, Layer, LayerStack, Phase, Placement, STRIP_H, StripFlattener,
    encode_jpeg, encode_png, encode_webp, flatten_full, resize_lanczos3,
};

#[test]
#[ignore = "timing run; see the module docs"]
fn export_timings() {
    let path = std::env::var("PF_PERF_IMAGE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/generated/grid-map-4200x5600.png")
        });
    let t = Instant::now();
    let mut reader = image::ImageReader::open(&path).expect("input");
    reader.no_limits();
    let base = Arc::new(reader.decode().expect("decode").to_rgb8());
    let (w, h) = base.dimensions();
    println!("decode {w}x{h}: {:?}", t.elapsed());
    let patch = Arc::new(image::RgbaImage::from_pixel(
        900,
        700,
        image::Rgba([200, 90, 40, 180]),
    ));
    let layers = [Layer {
        image: patch,
        placement: Placement::at(1000, 1500, 900, 700),
        opacity: 1.0,
        visible: true,
    }];
    let stack = LayerStack::new(base, &layers).expect("stack");
    let mut go = |_: Phase, _: f32| ControlFlow::Continue(());

    let t = Instant::now();
    let mut png = std::io::BufWriter::new(tempfile::tempfile().expect("tmp"));
    encode_png(
        &mut StripFlattener::new(&stack, STRIP_H).expect("rows"),
        &mut png,
        Phase::Encode,
        &mut go,
    )
    .expect("png");
    println!("PNG stream, scale 1: {:?}", t.elapsed());

    let t = Instant::now();
    let mut jpeg = std::io::BufWriter::new(tempfile::tempfile().expect("tmp"));
    encode_jpeg(
        &mut StripFlattener::new(&stack, STRIP_H).expect("rows"),
        &mut jpeg,
        Phase::Encode,
        &mut go,
    )
    .expect("jpeg");
    println!("JPEG stream, scale 1: {:?}", t.elapsed());

    let t = Instant::now();
    let full = flatten_full::<ComposeError, _>(&stack, STRIP_H, |_| Ok(())).expect("flatten");
    let flat = t.elapsed();
    let half = resize_lanczos3(&full, w / 2, h / 2).expect("resize");
    let resized = t.elapsed();
    let mut png = std::io::BufWriter::new(tempfile::tempfile().expect("tmp"));
    encode_png(&mut ImageRows::new(&half), &mut png, Phase::Encode, &mut go).expect("png");
    println!(
        "flatten {flat:?}, + resize 0.5x {resized:?}, + PNG {:?}",
        t.elapsed()
    );

    if w <= 16383 && h <= 16383 {
        let t = Instant::now();
        let out = encode_webp(&full, &mut go).expect("webp");
        println!(
            "WebP lossy q90 (full buffer): {:?}, {} KiB",
            t.elapsed(),
            out.bytes.len() / 1024
        );
    }
}

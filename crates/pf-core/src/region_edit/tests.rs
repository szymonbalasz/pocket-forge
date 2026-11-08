//! RGN slice 1 through the core: commands with the fake provider and an in-memory project.

use std::sync::Arc;

use image::{Rgb, RgbImage, Rgba, RgbaImage};
use pf_provider::fake::FakeProvider;
use pf_provider::{Capabilities, InputPx, Provider};
use serde_json::{Value, json};
use ulid::Ulid;

use super::fit::{RectPx, region_fit};
use super::pixels::{MaskRecord, MemorySource};
use super::prepare::{prepare, sent_mask, sent_region};
use crate::context::Context;
use crate::core::Core;
use crate::error::ErrorCode;
use crate::manifest::{Layer, LayerKind};
use crate::time::Timestamp;

const W: u32 = 1200;
const H: u32 = 900;
const INNER: RectPx = RectPx {
    x: 400,
    y: 300,
    w: 300,
    h: 200,
};

fn project_id() -> Ulid {
    Ulid::from_parts(7, 7)
}

fn mask_id() -> Ulid {
    Ulid::from_parts(8, 8)
}

/// A textured "map": every pixel differs from its neighbours.
fn original() -> RgbImage {
    RgbImage::from_fn(W, H, |x, y| {
        Rgb([
            (x % 251) as u8,
            (y % 241) as u8,
            ((x * 7 + y * 13) % 256) as u8,
        ])
    })
}

/// A disc painted in the middle of `rect` (255 inside, soft edge).
fn disc_mask(rect: RectPx) -> Vec<u8> {
    let (cx, cy) = (f64::from(rect.w) / 2.0, f64::from(rect.h) / 2.0);
    let r = f64::from(rect.w.min(rect.h)) / 3.0;
    (0..rect.w * rect.h)
        .map(|i| {
            let (x, y) = (f64::from(i % rect.w) + 0.5, f64::from(i / rect.w) + 0.5);
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            ((r + 2.0 - d) / 4.0 * 255.0).clamp(0.0, 255.0) as u8
        })
        .collect()
}

fn mask_record(rect: RectPx, bytes: &[u8]) -> MaskRecord {
    MaskRecord {
        id: mask_id(),
        rect,
        png: pf_imaging::encode_grey8_png(rect.w, rect.h, bytes).unwrap(),
    }
}

fn source() -> MemorySource {
    let mut s = MemorySource::new(project_id(), original());
    s.masks.push(mask_record(INNER, &disc_mask(INNER)));
    s
}

struct Fixture {
    _dir: tempfile::TempDir,
    core: Core,
}

fn fixture_with(src: MemorySource) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(1_800_000_000_000));
    ctx.library_override = Some(dir.path().join("lib"));
    let core = Core::new(ctx).unwrap();
    core.state().rgn.source_override = Some(Arc::new(src));
    Fixture { _dir: dir, core }
}

fn fixture() -> Fixture {
    fixture_with(source())
}

fn fake_caps(model: &str) -> Capabilities {
    pf_provider::block_on(FakeProvider::default().get_capabilities(model)).unwrap()
}

// ---------------------------------------------------------------- slice 1

#[test]
fn fr_rgn_01_region_fit_command_snaps_clamps_and_checks_the_project() {
    let f = fixture();
    let out = f
        .core
        .dispatch(
            "rgn_region_fit",
            json!({"project_id": project_id(), "inner": {"x": 1100.4, "y": -20.0, "w": 300.0, "h": 100.6}}),
        )
        .unwrap();
    assert_eq!(out["inner"], json!({"x": 1100, "y": 0, "w": 100, "h": 81}));
    assert_eq!(out["margin"], 128);
    assert_eq!(out["outer"], json!({"x": 972, "y": 0, "w": 228, "h": 209}));
    assert_eq!(
        (out["sent_w"].as_u64(), out["scale"].as_f64()),
        (Some(228), Some(1.0))
    );

    let err = |input: Value| f.core.dispatch("rgn_region_fit", input).unwrap_err();
    let e =
        err(json!({"project_id": project_id(), "inner": {"x": 5000, "y": 0, "w": 50, "h": 50}}));
    assert_eq!(e.code, ErrorCode::OutOfBounds);
    let e = err(json!({"project_id": project_id(), "inner": {"x": 0, "y": 0, "w": 15, "h": 50}}));
    assert_eq!(e.code, ErrorCode::RegionTooSmall);
    let e = err(
        json!({"project_id": Ulid::from_parts(1, 1), "inner": {"x": 0, "y": 0, "w": 50, "h": 50}}),
    );
    assert_eq!(e.code, ErrorCode::UnknownId);
    let e = err(
        json!({"project_id": project_id(), "inner": {"x": 0, "y": 0, "w": 50, "h": 50}, "model_id": "nope"}),
    );
    assert_eq!(e.code, ErrorCode::UnknownModel);
    let e = err(
        json!({"project_id": project_id(), "inner": {"x": 0, "y": 0, "w": 50, "h": 50}, "margin": 600}),
    );
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::InvalidInput, Some("margin"))
    );
}

#[test]
fn fr_rgn_01_region_fit_without_a_project_is_no_project() {
    let f = fixture();
    f.core.state().rgn.source_override = None;
    let e = f
        .core
        .dispatch(
            "rgn_region_fit",
            json!({"project_id": project_id(), "inner": {"x": 0, "y": 0, "w": 50, "h": 50}}),
        )
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NoProject);
}

#[test]
fn fr_rgn_02_region_fit_margin_and_4096_cap() {
    let big = MemorySource::new(project_id(), RgbImage::new(6000, 600));
    let f = fixture_with(big);
    let fit = |margin: u32, w: u32| {
        f.core.dispatch(
            "rgn_region_fit",
            json!({"project_id": project_id(), "inner": {"x": 1000, "y": 100, "w": w, "h": 100}, "margin": margin}),
        )
    };
    let out = fit(0, 300).unwrap();
    assert_eq!(out["outer"], out["inner"]);
    let out = fit(512, 300).unwrap();
    assert_eq!(out["outer"], json!({"x": 488, "y": 0, "w": 1324, "h": 600}));
    assert_eq!(fit(48, 4000).unwrap()["outer"]["w"], 4096);
    assert_eq!(fit(49, 4000).unwrap_err().code, ErrorCode::RegionTooLarge);
}

#[test]
fn fr_rgn_05_region_fit_command_uses_the_models_input_px() {
    let f = fixture();
    let out = f
        .core
        .dispatch(
            "rgn_region_fit",
            json!({"project_id": project_id(), "inner": {"x": 400, "y": 300, "w": 301, "h": 203}, "model_id": "fake-white-edit"}),
        )
        .unwrap();
    let (sw, sh) = (
        out["sent_w"].as_u64().unwrap(),
        out["sent_h"].as_u64().unwrap(),
    );
    assert!(sw % 8 == 0 && sh % 8 == 0, "{out}");
    assert_eq!(
        (out["outer"]["w"].as_u64(), out["outer"]["h"].as_u64()),
        (Some(sw), Some(sh))
    );
    assert_eq!(out["scale"], 1.0, "sent unresampled");
}

#[test]
fn fr_rgn_03_mask_is_padded_with_zero_resampled_and_validated() {
    let fit = region_fit(INNER.into(), 128, W, H, None).unwrap();
    let mask = disc_mask(INNER);
    let sent = sent_mask(&mask_record(INNER, &mask), &fit).unwrap();
    assert_eq!(sent.len(), (fit.sent_w * fit.sent_h) as usize);
    let o = fit.outer;
    for y in 0..o.h {
        for x in 0..o.w {
            let v = sent[(y * o.w + x) as usize];
            let (ix, iy) = (o.x + x, o.y + y);
            let inside =
                (INNER.x..INNER.right()).contains(&ix) && (INNER.y..INNER.bottom()).contains(&iy);
            if inside {
                assert_eq!(v, mask[((iy - INNER.y) * INNER.w + ix - INNER.x) as usize]);
            } else {
                assert_eq!(v, 0, "context pixel ({ix},{iy}) must be 0");
            }
        }
    }

    // A mask painted for a region that has since moved: only the part inside the current
    // region is kept.
    let shifted = RectPx { x: 350, ..INNER };
    let full = vec![255u8; (INNER.w * INNER.h) as usize];
    let sent = sent_mask(&mask_record(shifted, &full), &fit).unwrap();
    let at = |ix: u32, iy: u32| sent[((iy - o.y) * o.w + ix - o.x) as usize];
    assert_eq!(at(380, 350), 0, "outside the current region");
    assert_eq!(at(400, 350), 255);
    assert_eq!(at(649, 350), 255);
    assert_eq!(at(650, 350), 0, "past the shifted mask");

    // Validation.
    let reason =
        |e: crate::CoreError| (e.code, e.detail["reason"].as_str().unwrap_or("").to_owned());
    let e = sent_mask(
        &mask_record(INNER, &vec![0; (INNER.w * INNER.h) as usize]),
        &fit,
    )
    .unwrap_err();
    assert_eq!(reason(e), (ErrorCode::MaskInvalid, "empty".into()));
    let small = RectPx {
        w: 10,
        h: 10,
        ..INNER
    };
    let mut wrong_size = mask_record(small, &[255; 100]);
    wrong_size.rect = INNER;
    assert_eq!(
        reason(sent_mask(&wrong_size, &fit).unwrap_err()),
        (ErrorCode::MaskInvalid, "size".into())
    );
    let rgb = MaskRecord {
        id: mask_id(),
        rect: INNER,
        png: pf_imaging::encode_rgb8_png(&RgbImage::new(INNER.w, INNER.h)).unwrap(),
    };
    assert_eq!(
        reason(sent_mask(&rgb, &fit).unwrap_err()),
        (ErrorCode::MaskInvalid, "depth".into())
    );
}

#[test]
fn fr_rgn_03_mask_is_box_resampled_by_the_fit_scale() {
    // A model whose max forces a 2× downscale: 400×300 outer → 200×150.
    let inner = RectPx {
        x: 100,
        y: 100,
        w: 400,
        h: 300,
    };
    let px = InputPx {
        min: 8,
        max: 200,
        multiple_of: 2,
    };
    let fit = region_fit(inner.into(), 0, W, H, Some(px)).unwrap();
    assert_eq!((fit.sent_w, fit.sent_h, fit.scale), (200, 150, 0.5));
    // Left half painted 255, right half 0 → left half 255, right half 0, no seam column.
    let bytes: Vec<u8> = (0..400 * 300)
        .map(|i| if i % 400 < 200 { 255 } else { 0 })
        .collect();
    let sent = sent_mask(&mask_record(inner, &bytes), &fit).unwrap();
    assert_eq!(sent.len(), 200 * 150);
    assert!(
        sent.chunks(200)
            .all(|row| row[..100].iter().all(|&v| v == 255) && row[100..].iter().all(|&v| v == 0))
    );
}

#[test]
fn fr_rgn_04_region_pixels_are_a_bit_exact_crop_without_patches() {
    let src = source();
    let fit = region_fit(INNER.into(), 128, W, H, None).unwrap();
    let px = sent_region(&src, &fit).unwrap();
    let orig = original();
    let crop = image::imageops::crop_imm(&orig, fit.outer.x, fit.outer.y, fit.outer.w, fit.outer.h)
        .to_image();
    assert_eq!(px, crop);
}

#[test]
fn fr_rgn_04_region_pixels_compose_the_current_layer_stack() {
    let mut src = source();
    // A 50×50 opaque red patch at (420, 320), scale 1, rotation 0.
    let mut layer = Layer::original(Ulid::from_parts(9, 9), 50, 50);
    layer.kind = LayerKind::Patch;
    layer.cx = 420.0 + 25.0;
    layer.cy = 320.0 + 25.0;
    src.patches.push((
        layer,
        Arc::new(RgbaImage::from_pixel(50, 50, Rgba([255, 0, 0, 255]))),
    ));
    let fit = region_fit(INNER.into(), 0, W, H, None).unwrap();
    let px = sent_region(&src, &fit).unwrap();
    assert_eq!(px.get_pixel(20, 20).0, [255, 0, 0]);
    assert_eq!(px.get_pixel(69, 69).0, [255, 0, 0]);
    assert_eq!(px.get_pixel(70, 70), original().get_pixel(470, 370));
}

#[test]
fn fr_rgn_04_original_not_ready_is_original_unavailable() {
    let mut src = source();
    src.original = None;
    let fit = region_fit(INNER.into(), 128, W, H, None).unwrap();
    let e = sent_region(&src, &fit).unwrap_err();
    assert_eq!(e.code, ErrorCode::OriginalUnavailable);
}

#[test]
fn fr_rgn_05_downscaled_region_is_lanczos_resampled_to_the_sent_size() {
    let inner = RectPx {
        x: 100,
        y: 100,
        w: 400,
        h: 300,
    };
    let mut caps = fake_caps("fake-white-edit");
    caps.input_px = InputPx {
        min: 8,
        max: 200,
        multiple_of: 8,
    };
    let fit = region_fit(inner.into(), 0, W, H, Some(caps.input_px)).unwrap();
    assert_eq!((fit.sent_w, fit.sent_h), (200, 144));
    let mut src = source();
    src.masks = vec![mask_record(inner, &vec![255; 400 * 300])];
    let images = prepare(&src, &fit, &src.masks[0].clone(), &caps).unwrap();
    let region = image::load_from_memory(&images.region_png).unwrap();
    assert_eq!((region.width(), region.height()), (200, 144));
    let (mw, mh, _) = pf_imaging::decode_grey8_png(&images.mask_png).unwrap();
    assert_eq!((mw, mh), (200, 144));
}

#[test]
fn fr_rgn_06_sent_bytes_are_checked_against_max_bytes() {
    let src = source();
    let mut caps = fake_caps("fake-white-edit");
    let fit = region_fit(INNER.into(), 128, W, H, Some(caps.input_px)).unwrap();
    caps.max_bytes = 1000;
    let e = prepare(&src, &fit, &src.masks[0], &caps).unwrap_err();
    assert_eq!(e.code, ErrorCode::InputTooLarge);
    assert_eq!(e.detail, json!({"field": "image", "limit": 1000}));
}

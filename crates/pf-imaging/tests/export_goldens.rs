//! Export golden images (export spec §13, OQ-IMG-07).
//!
//! Decoded pixels must match exactly on Windows and within ±1 elsewhere. Strip height and
//! `COMPOSE_VERSION` are pinned. Regenerate with `PF_UPDATE_GOLDENS=1 cargo test -p pf-imaging
//! --test export_goldens` and review the PNG diff.
//!
//! Inputs are generated: the base is `fixtures/images/grid-map-1024x1280.png` (fixtures/gen) and
//! patches are synthetic RGBA (feathered disc). No personal art.

use std::path::PathBuf;
use std::sync::Arc;

use image::{RgbImage, RgbaImage};
use pf_imaging::{
    COMPOSE_VERSION, ComposeError, Layer, LayerStack, Placement, STRIP_H, flatten_full,
};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn base() -> Arc<RgbImage> {
    let img = image::open(fixtures().join("images/grid-map-1024x1280.png")).expect("base fixture");
    Arc::new(img.to_rgb8())
}

/// A feathered disc with a coloured stripe texture: alpha 255 inside r − feather, ramping to 0
/// at r. The texture makes resampling visible in the golden.
fn feathered_disc(size: u32, feather: f64) -> Arc<RgbaImage> {
    let c = f64::from(size) / 2.0;
    Arc::new(RgbaImage::from_fn(size, size, |x, y| {
        let (dx, dy) = (f64::from(x) + 0.5 - c, f64::from(y) + 0.5 - c);
        let d = (dx * dx + dy * dy).sqrt();
        let a = ((c - d) / feather).clamp(0.0, 1.0);
        let stripe = (x + 2 * y) / 9 % 3;
        let rgb = match stripe {
            0 => [220, 60, 40],
            1 => [40, 180, 90],
            _ => [250, 230, 120],
        };
        image::Rgba([rgb[0], rgb[1], rgb[2], (a * 255.0).round() as u8])
    }))
}

fn flatten(stack: &LayerStack) -> RgbImage {
    flatten_full::<ComposeError, _>(stack, STRIP_H, |_| Ok(())).expect("flatten")
}

/// Compares decoded pixels with the golden, or rewrites it when `PF_UPDATE_GOLDENS=1`.
fn assert_golden(name: &str, got: &RgbImage) {
    let path = fixtures()
        .join("goldens/export")
        .join(format!("{name}.png"));
    if std::env::var("PF_UPDATE_GOLDENS").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        got.save(&path).expect("write golden");
        return;
    }
    let want = image::open(&path)
        .unwrap_or_else(|e| {
            panic!(
                "golden {}: {e} (PF_UPDATE_GOLDENS=1 writes it)",
                path.display()
            )
        })
        .to_rgb8();
    assert_eq!(got.dimensions(), want.dimensions(), "{name}: size");
    assert_pixels_close(name, got, &want);
}

fn assert_pixels_close(name: &str, got: &RgbImage, want: &RgbImage) {
    let tolerance: u8 = if cfg!(windows) { 0 } else { 1 };
    let mut worst = 0u8;
    let mut bad = 0usize;
    for (g, w) in got.as_raw().iter().zip(want.as_raw()) {
        let d = g.abs_diff(*w);
        worst = worst.max(d);
        if d > tolerance {
            bad += 1;
        }
    }
    assert_eq!(
        bad, 0,
        "{name}: {bad} channels differ by more than {tolerance} (worst {worst})"
    );
}

#[test]
fn golden_pins_strip_height_and_compose_version() {
    assert_eq!(
        (STRIP_H, COMPOSE_VERSION),
        (512, 1),
        "regenerate the goldens when these change"
    );
}

/// (a) No layers: the flatten is the original (FR-EXP-04).
#[test]
fn fr_exp_04_golden_a_no_layers_equals_original() {
    let base = base();
    let stack = LayerStack::new(base.clone(), &[]).expect("stack");
    assert!(
        flatten(&stack) == *base,
        "flatten differs from the original"
    );
}

/// (c) One affine layer: scale 1.3, rotation 15°, opacity 0.8 (FR-EXP-02, FR-EXP-03).
#[test]
fn fr_exp_03_golden_c_affine_layer() {
    let layer = Layer {
        image: feathered_disc(256, 24.0),
        placement: Placement {
            cx: 512.3,
            cy: 600.7,
            scale: 1.3,
            rotation: 15.0,
        },
        opacity: 0.8,
        visible: true,
    };
    let base = base();
    let stack = LayerStack::new(base.clone(), &[layer]).expect("stack");
    let out = flatten(&stack);
    assert_golden("c-affine", &out);
    // Far from the layer the original is untouched.
    assert_eq!(out.get_pixel(10, 10), base.get_pixel(10, 10));
    assert_eq!(out.get_pixel(1000, 1270), base.get_pixel(1000, 1270));
}

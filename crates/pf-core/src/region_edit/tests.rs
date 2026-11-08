//! RGN slices 1–2 through the core: commands with the fake provider and an in-memory project.

use std::collections::BTreeMap;
use std::sync::Arc;

use image::{Rgb, RgbImage, Rgba, RgbaImage};
use pf_provider::fake::{FakeCase, FakeProvider};
use pf_provider::{
    Capabilities, Control, ControlRole, ControlType, CostAmount, InputPx, Provider, Rule,
    RuleCondition, RuleEffect, RuleOp,
};
use serde_json::{Value, json};
use ulid::Ulid;

use super::controls::{check_controls, evaluate_controls};
use super::fit::{RectPx, region_fit};
use super::pixels::{MaskRecord, MemorySource};
use super::prepare::{prepare, sent_mask, sent_region};
use crate::context::{Context, RecordedEvents};
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
    events: Arc<RecordedEvents>,
}

fn fixture_with(src: MemorySource) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(1_800_000_000_000));
    ctx.library_override = Some(dir.path().join("lib"));
    let events = Arc::new(RecordedEvents::default());
    ctx.events = events.clone();
    let core = Core::new(ctx).unwrap();
    core.state().rgn.source_override = Some(Arc::new(src));
    Fixture {
        _dir: dir,
        core,
        events,
    }
}

fn fixture() -> Fixture {
    fixture_with(source())
}

fn fake(core: &Core) -> &FakeProvider {
    core.ctx.provider.as_fake().unwrap()
}

fn fake_caps(model: &str) -> Capabilities {
    pf_provider::block_on(FakeProvider::default().get_capabilities(model)).unwrap()
}

fn draft(prompt: &str) -> Value {
    json!({
        "region": {"inner": {"x": 400, "y": 300, "w": 300, "h": 200}, "margin": 128},
        "mask_id": mask_id(),
        "model_id": "fake-white-edit",
        "final_prompt": prompt,
        "controls": {},
        "seed": null,
    })
}

fn estimate(core: &Core, draft: Value) -> Result<Value, crate::CoreError> {
    core.dispatch("rgn_job_estimate", json!({ "draft": draft }))
}

fn ctl(name: &str, kind: ControlType, role: ControlRole) -> Control {
    Control {
        name: name.into(),
        kind,
        role,
        min: None,
        max: None,
        step: None,
        allowed: vec![],
        default: None,
        required: false,
        cost_affecting: false,
    }
}

/// A model with every control type and a `hidden` / `allowed` / `required` rule each.
fn rule_caps() -> Capabilities {
    let mut caps = fake_caps("fake-white-edit");
    let mut mode = ctl("mode", ControlType::Enum, ControlRole::Other);
    mode.allowed = vec![json!("edit"), json!("inpaint"), json!("remix")];
    mode.default = Some(json!("edit"));
    let mut strength = ctl("strength", ControlType::Float, ControlRole::Other);
    (strength.min, strength.max, strength.step) = (Some(0.0), Some(1.0), Some(0.25));
    strength.default = Some(json!(0.5));
    let mut quality = ctl("quality", ControlType::Enum, ControlRole::Other);
    quality.allowed = vec![json!("low"), json!("mid"), json!("high")];
    quality.default = Some(json!("mid"));
    quality.cost_affecting = true;
    let style = ctl("style", ControlType::String, ControlRole::Other);
    let mut magic = ctl("magic", ControlType::Bool, ControlRole::Other);
    magic.default = Some(json!(false));
    caps.controls
        .extend([mode, strength, quality, style, magic]);
    let when = |control: &str, op, value: Value, then| Rule {
        when: RuleCondition {
            control: control.into(),
            op,
            value,
        },
        then,
    };
    let effect = |control: &str| RuleEffect {
        control: control.into(),
        required: None,
        allowed: None,
        hidden: None,
    };
    caps.rules = vec![
        when(
            "mode",
            RuleOp::Eq,
            json!("inpaint"),
            RuleEffect {
                hidden: Some(true),
                ..effect("strength")
            },
        ),
        when(
            "mode",
            RuleOp::Eq,
            json!("remix"),
            RuleEffect {
                allowed: Some(vec![json!("low")]),
                ..effect("quality")
            },
        ),
        when(
            "magic",
            RuleOp::Eq,
            json!(true),
            RuleEffect {
                required: Some(true),
                ..effect("style")
            },
        ),
    ];
    caps.warnings = vec!["input \"lora\" has an unknown type and is omitted".into()];
    caps
}

fn vals(v: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(v).unwrap()
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
    let f = fixture_with(src);
    let e = estimate(&f.core, draft("a kitchen")).unwrap_err();
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

// ---------------------------------------------------------------- slice 2: controls

#[test]
fn fr_rgn_08_controls_follow_the_fake_models_capabilities() {
    let f = fixture();
    for model in pf_provider::fake::FAKE_MODEL_IDS {
        let caps = fake_caps(model);
        let out = f
            .core
            .dispatch(
                "rgn_controls_evaluate",
                json!({"model_id": model, "values": {}}),
            )
            .unwrap();
        let names: Vec<&str> = out["controls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        let expected: Vec<&str> = caps
            .controls
            .iter()
            .filter(|c| {
                !matches!(
                    c.role,
                    ControlRole::Source | ControlRole::Mask | ControlRole::Reference
                )
            })
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(
            names, expected,
            "{model}: exactly the capability controls, no image inputs"
        );
        assert_eq!(out["warnings"], json!(caps.warnings));
        assert_eq!(out["prompt_required"], caps.prompt_required);
    }
    let e = f
        .core
        .dispatch(
            "rgn_controls_evaluate",
            json!({"model_id": "nope", "values": {}}),
        )
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::UnknownModel);
}

#[test]
fn fr_rgn_33_rules_hide_narrow_and_require() {
    let caps = rule_caps();
    let by_name = |e: &super::controls::ControlsEvaluation, n: &str| {
        e.controls.iter().find(|c| c.name == n).cloned().unwrap()
    };
    // Defaults: everything visible, defaults prefilled.
    let e = evaluate_controls(&caps, &vals(json!({"prompt": "a kitchen"})));
    assert!(by_name(&e, "strength").visible);
    assert_eq!(by_name(&e, "strength").value, Some(json!(0.5)));
    assert_eq!(e.warnings, caps.warnings);
    assert!(e.required_missing.is_empty(), "{:?}", e.required_missing);

    // hidden: strength disappears and carries no value.
    let e = evaluate_controls(
        &caps,
        &vals(json!({"prompt": "p", "mode": "inpaint", "strength": 0.75})),
    );
    let s = by_name(&e, "strength");
    assert!(!s.visible);
    assert_eq!(s.value, None);

    // allowed: quality narrows to low; a remembered "high" resets to the default with a notice.
    let e = evaluate_controls(
        &caps,
        &vals(json!({"prompt": "p", "mode": "remix", "quality": "high"})),
    );
    let q = by_name(&e, "quality");
    assert_eq!(q.allowed, vec![json!("low")]);
    assert!(q.reset);
    assert_eq!(
        q.value,
        Some(json!("low")),
        "the default isn't allowed either: first allowed"
    );
    assert!(q.error.is_none());
    // Without the narrowing rule the reset lands on the default.
    let e = evaluate_controls(&caps, &vals(json!({"prompt": "p", "quality": "ultra"})));
    let q = by_name(&e, "quality");
    assert_eq!((q.value, q.reset), (Some(json!("mid")), true));

    // required: style becomes required when magic is on; submit is disabled naming it.
    let e = evaluate_controls(&caps, &vals(json!({"prompt": "p", "magic": true})));
    assert!(by_name(&e, "style").required);
    assert_eq!(e.required_missing, vec!["style".to_owned()]);
    // An empty prompt is missing too (FR-RGN-09).
    let e = evaluate_controls(&caps, &vals(json!({"prompt": ""})));
    assert_eq!(e.required_missing, vec!["prompt".to_owned()]);
    // A value for a control the model doesn't have (another model's draft) is dropped.
    let e = evaluate_controls(&caps, &vals(json!({"prompt": "p", "lora": 1})));
    assert_eq!(e.unknown, vec!["lora".to_owned()]);
}

#[test]
fn fr_rgn_33_boundary_check_rejects_invalid_controls() {
    let caps = rule_caps();
    let check = |c: Value| check_controls(&caps, &vals(c), "a kitchen", None, true);
    let reason = |e: crate::CoreError| {
        (
            e.code,
            e.detail["name"].as_str().unwrap().to_owned(),
            e.detail["reason"].as_str().unwrap().to_owned(),
        )
    };
    assert_eq!(
        reason(check(json!({"strength": 2.0})).unwrap_err()),
        (
            ErrorCode::InvalidControl,
            "strength".into(),
            "out_of_range".into()
        )
    );
    assert_eq!(
        reason(check(json!({"mode": "remix", "quality": "high"})).unwrap_err()),
        (
            ErrorCode::InvalidControl,
            "quality".into(),
            "not_allowed".into()
        )
    );
    assert_eq!(
        reason(check(json!({"magic": true})).unwrap_err()),
        (ErrorCode::InvalidControl, "style".into(), "required".into())
    );
    assert_eq!(
        reason(check(json!({"lora": 1})).unwrap_err()).2,
        "unknown_control"
    );
    assert_eq!(
        reason(check(json!({"image": "x"})).unwrap_err()).2,
        "bound_by_role"
    );
    assert_eq!(
        reason(check(json!({"prompt": "x"})).unwrap_err()).2,
        "bound_by_role"
    );
}

#[test]
fn fr_rgn_34_sent_values_are_required_or_user_set_and_visible_only() {
    let caps = rule_caps();
    let sent = check_controls(
        &caps,
        &vals(json!({"mode": "inpaint", "strength": 0.25, "num_samples": 2})),
        "p",
        None,
        true,
    )
    .unwrap();
    // mode was set; strength is hidden by the rule → dropped; untouched quality/magic omitted.
    assert_eq!(sent.controls, vals(json!({"mode": "inpaint"})));
    assert_eq!(sent.candidate_count, 2);
    assert_eq!(
        sent.cost_inputs,
        vals(json!({"num_samples": 2, "quality": "mid"}))
    );
    // Required with a default → sent with the default.
    let mut caps2 = caps.clone();
    caps2
        .controls
        .iter_mut()
        .find(|c| c.name == "magic")
        .unwrap()
        .required = true;
    let sent = check_controls(&caps2, &BTreeMap::new(), "p", None, true).unwrap();
    assert_eq!(sent.controls, vals(json!({"magic": false})));
    assert_eq!(sent.candidate_count, 3, "the count control's default");
}

/// Owner decision 2026-11-08: a model without a `candidate_count` control (real Ideogram V3 /
/// Character) gives exactly one result per job. Estimate and submit share this check, so both
/// ask for 1; a count sent anyway is an unknown control, never a silent fan-out.
#[test]
fn fr_rgn_08_owner_2026_11_08_no_count_control_means_one_result_per_job() {
    let mut caps = fake_caps("fake-white-edit");
    caps.controls
        .retain(|c| c.role != ControlRole::CandidateCount);
    for require_prompt in [false, true] {
        let sent = check_controls(&caps, &BTreeMap::new(), "p", None, require_prompt).unwrap();
        assert_eq!(sent.candidate_count, 1);
        assert!(sent.cost_inputs.is_empty(), "no count in the estimate id");
    }
    let e = check_controls(&caps, &vals(json!({"num_samples": 3})), "p", None, true).unwrap_err();
    assert_eq!(
        (e.code, e.detail["reason"].as_str()),
        (ErrorCode::InvalidControl, Some("unknown_control"))
    );
}

#[test]
fn fr_rgn_09_prompt_is_required_at_submit_but_not_for_an_estimate() {
    let caps = fake_caps("fake-white-edit");
    let e = check_controls(&caps, &BTreeMap::new(), "  ", None, true).unwrap_err();
    assert_eq!(
        (e.code, e.detail["reason"].as_str()),
        (ErrorCode::InvalidControl, Some("required"))
    );
    assert!(check_controls(&caps, &BTreeMap::new(), "", None, false).is_ok());
    // The estimate works before a prompt is typed.
    let f = fixture();
    assert!(estimate(&f.core, draft("")).is_ok());
}

#[test]
fn fr_rgn_10_seed_is_pinned_or_random() {
    let caps = fake_caps("fake-white-edit");
    let sent = check_controls(&caps, &BTreeMap::new(), "p", None, true).unwrap();
    assert_eq!(sent.seed, None);
    if caps.seed {
        let sent = check_controls(&caps, &BTreeMap::new(), "p", Some(1234), true).unwrap();
        assert_eq!(sent.seed, Some(1234));
    }
    let mut no_seed = caps.clone();
    no_seed.seed = false;
    let e = check_controls(&no_seed, &BTreeMap::new(), "p", Some(1), true).unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidControl);
}

// ---------------------------------------------------------------- slice 2: estimate + spend

#[test]
fn fr_rgn_12_estimate_reuses_the_cost_until_a_cost_input_changes() {
    let f = fixture();
    let first = estimate(&f.core, draft("a kitchen")).unwrap();
    assert_eq!(
        first["cost"],
        json!({"amount": 3.0, "unit": "fake"}),
        "num_samples default 3"
    );
    assert_eq!(first["candidate_count"], 3);
    let id = first["estimate_id"].as_str().unwrap().to_owned();

    // From now on the provider would answer "unknown": a reused estimate proves no call.
    fake(&f.core).set_case(FakeCase::EstimateUnknown);
    let mut other_prompt = draft("a pantry");
    other_prompt["region"]["inner"]["x"] = json!(401);
    let again = estimate(&f.core, other_prompt).unwrap();
    assert_eq!(
        again["estimate_id"],
        json!(id),
        "prompt and position don't change the id"
    );
    assert_eq!(again["cost"]["amount"], 3.0, "no new provider estimate");

    let mut more = draft("a kitchen");
    more["controls"] = json!({"num_samples": 4});
    let changed = estimate(&f.core, more).unwrap();
    assert_ne!(changed["estimate_id"], json!(id));
    assert_eq!(changed["cost"]["amount"], "unknown");
    assert!(
        changed["needs"]
            .as_array()
            .unwrap()
            .contains(&json!("unknown_cost"))
    );
}

#[test]
fn fr_rgn_13_estimate_id_follows_model_and_sent_size() {
    let f = fixture();
    let a = estimate(&f.core, draft("p")).unwrap();
    let mut wider = draft("p");
    wider["region"]["inner"]["w"] = json!(340);
    let b = estimate(&f.core, wider).unwrap();
    assert_ne!(a["estimate_id"], b["estimate_id"]);
    assert_ne!(a["fit"]["sent_w"], b["fit"]["sent_w"]);
    let mut model = draft("p");
    model["model_id"] = json!("fake-black-edit");
    assert_ne!(
        estimate(&f.core, model).unwrap()["estimate_id"],
        a["estimate_id"]
    );
}

#[test]
fn fr_rgn_14_16_estimate_reports_spend_cap_and_needs() {
    let f = fixture();
    let out = estimate(&f.core, draft("p")).unwrap();
    assert_eq!(out["spend"], 0.0);
    assert_eq!(out["cap"], Value::Null);
    assert_eq!(out["needs"], json!(["set_cap"]));

    let set = f
        .core
        .dispatch("rgn_spend_set", json!({"cap": 2.5, "threshold": 2.0}))
        .unwrap();
    assert_eq!(set, json!({"spend": 0.0, "cap": 2.5, "threshold": 2.0}));
    let saved: Value =
        serde_json::from_slice(&std::fs::read(f.core.ctx.data_dir.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        saved["spend"],
        json!({"cap": 2.5, "threshold": 2.0}),
        "PRJ's settings section spend (R8)"
    );
    assert_eq!(
        f.events.names().last().map(String::as_str),
        Some("rgn:spend")
    );

    let out = estimate(&f.core, draft("p")).unwrap();
    assert_eq!(out["needs"], json!(["threshold"]));
    assert_eq!(out["over_cap"], true, "0 + 3 > 2.5");

    // Session spend counts submitted jobs (the ledger slice 3's submit feeds).
    f.core
        .state()
        .rgn
        .ledger
        .record_submit(Ulid::from_parts(1, 2), CostAmount::Known(1.5));
    assert_eq!(
        f.core.dispatch("rgn_spend_get", json!({})).unwrap()["spend"],
        1.5
    );
    f.core
        .dispatch("rgn_spend_set", json!({"cap": 10, "threshold": 5}))
        .unwrap();
    let out = estimate(&f.core, draft("p")).unwrap();
    assert_eq!(
        (out["spend"].as_f64(), out["over_cap"].as_bool()),
        (Some(1.5), Some(false))
    );
    assert_eq!(out["needs"], json!([]));
}

#[test]
fn fr_rgn_16_spend_set_validates_values() {
    let f = fixture();
    let e = f
        .core
        .dispatch("rgn_spend_set", json!({"cap": -1, "threshold": 1}))
        .unwrap_err();
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::InvalidValue, Some("cap"))
    );
    assert_eq!(
        f.core.dispatch("rgn_spend_get", json!({})).unwrap()["cap"],
        Value::Null
    );
    // A missing field keeps its value ("raise cap" sends only the cap); null clears it.
    f.core
        .dispatch("rgn_spend_set", json!({"cap": 4, "threshold": 2}))
        .unwrap();
    let out = f.core.dispatch("rgn_spend_set", json!({"cap": 6})).unwrap();
    assert_eq!(
        (out["cap"].as_f64(), out["threshold"].as_f64()),
        (Some(6.0), Some(2.0))
    );
    let out = f
        .core
        .dispatch("rgn_spend_set", json!({"threshold": null}))
        .unwrap();
    assert_eq!(
        (out["cap"].as_f64(), out["threshold"].clone()),
        (Some(6.0), Value::Null)
    );
}

#[test]
fn fr_rgn_31_estimate_errors_are_normalised_and_change_nothing() {
    let f = fixture();
    let settings_before = std::fs::read(f.core.ctx.data_dir.join("settings.json")).unwrap();
    let mut no_mask = draft("p");
    no_mask["mask_id"] = json!(Ulid::from_parts(5, 5));
    assert_eq!(
        estimate(&f.core, no_mask).unwrap_err().code,
        ErrorCode::UnknownId
    );
    let mut bad_control = draft("p");
    bad_control["controls"] = json!({"num_samples": 9});
    let e = estimate(&f.core, bad_control).unwrap_err();
    assert_eq!(
        (e.code, e.detail["reason"].as_str()),
        (ErrorCode::InvalidControl, Some("out_of_range"))
    );
    let mut model = draft("p");
    model["model_id"] = json!("nope");
    assert_eq!(
        estimate(&f.core, model).unwrap_err().code,
        ErrorCode::UnknownModel
    );
    fake(&f.core).set_case(FakeCase::InsufficientBalance);
    let e = estimate(&f.core, draft("p")).unwrap_err();
    assert_eq!(
        (e.code, e.detail["provider"]["code"].as_str()),
        (ErrorCode::Provider, Some("insufficient_balance"))
    );
    let e = f
        .core
        .dispatch(
            "rgn_job_estimate",
            json!({"project_id": Ulid::from_parts(1, 1), "draft": draft("p")}),
        )
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::UnknownId);
    assert_eq!(
        std::fs::read(f.core.ctx.data_dir.join("settings.json")).unwrap(),
        settings_before
    );
}

#[test]
fn fr_rgn_38_read_only_project_requests_no_estimates() {
    let mut src = source();
    src.read_only = true;
    let f = fixture_with(src);
    assert_eq!(
        estimate(&f.core, draft("p")).unwrap_err().code,
        ErrorCode::ReadOnly
    );
    // Region fit still works (view, region and mask painting do).
    assert!(
        f.core
            .dispatch(
                "rgn_region_fit",
                json!({"project_id": project_id(), "inner": {"x": 0, "y": 0, "w": 50, "h": 50}})
            )
            .is_ok()
    );
}

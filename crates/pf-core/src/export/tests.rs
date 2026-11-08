//! EXP integration tests (export spec §13): commands over a real asset project, the fake clock,
//! PRJ's fault seam and the export seams. Background work runs inline (deterministic).

use std::io;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use image::{RgbImage, RgbaImage};
use pf_imaging::{EncodeError, LibWebp, WebpEncoder, WebpMode};
use serde_json::{Value, json};
use ulid::Ulid;

use super::ExportSeams;
use crate::context::{
    Clock, Context, FaultAction, Faults, ManualClock, RecordedEvents, SequentialIds,
};
use crate::core::Core;
use crate::error::ErrorCode;
use crate::manifest::{Layer, LayerKind, Rect, layer_file};
use crate::project::ReadOnlyReason;
use crate::time::Timestamp;

const T0: i64 = 1_800_000_000_000;

/// A manual clock the test keeps a handle to.
struct SharedClock(Arc<ManualClock>);

impl Clock for SharedClock {
    fn now(&self) -> Timestamp {
        self.0.now()
    }
}

struct T {
    dir: tempfile::TempDir,
    core: Core,
    clock: Arc<ManualClock>,
    events: Arc<RecordedEvents>,
    project: PathBuf,
}

/// A gradient with texture: every pixel differs from its neighbours.
fn map_image(w: u32, h: u32) -> RgbImage {
    RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([
            (x * 7 % 256) as u8,
            (y * 5 % 256) as u8,
            ((x + y) % 200) as u8,
        ])
    })
}

fn png_of(img: &RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn setup_with(img: &RgbImage, faults: Faults) -> T {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(T0));
    ctx.library_override = Some(dir.path().join("lib"));
    let clock = Arc::new(ManualClock::new(Timestamp(T0)));
    ctx.clock = Box::new(SharedClock(clock.clone()));
    ctx.ids = Box::new(SequentialIds::default());
    let events = Arc::new(RecordedEvents::default());
    ctx.events = events.clone();
    ctx.faults = faults;
    let core = Core::new(ctx).unwrap();
    let src = dir.path().join("Crypt.png");
    std::fs::write(&src, png_of(img)).unwrap();
    let out = core
        .dispatch(
            "project_create",
            json!({"section": "maps", "paths": [src.to_string_lossy()]}),
        )
        .unwrap();
    let project = PathBuf::from(out["items"][0]["path"].as_str().unwrap());
    core.dispatch("project_open", json!({"path": project}))
        .unwrap();
    let ready = core
        .dispatch("project_prefetch_original", json!({}))
        .unwrap();
    assert_eq!(ready["ready"], true);
    T {
        dir,
        core,
        clock,
        events,
        project,
    }
}

fn setup() -> T {
    setup_with(&map_image(64, 48), Faults::none())
}

fn settings(px: Option<f64>, target: Option<i64>, format: &str) -> Value {
    json!({
        "grid": {"mode": "px", "source_px_per_square": px},
        "target_px_per_square": target,
        "format": format,
    })
}

impl T {
    fn seq(&self) -> u64 {
        self.core
            .state()
            .project
            .as_ref()
            .unwrap()
            .last_commit
            .unwrap()
            .0
    }

    fn manifest_export(&self) -> Value {
        let m: Value =
            serde_json::from_slice(&std::fs::read(self.project.join("project.json")).unwrap())
                .unwrap();
        m["export"].clone()
    }

    fn start(&self, s: Value, replace: bool) -> Result<Value, crate::error::CoreError> {
        self.core
            .dispatch("export_start", json!({"settings": s, "replace": replace}))
    }

    fn events(&self, name: &str) -> Vec<Value> {
        self.events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn last_event(&self, name: &str) -> Value {
        self.events(name)
            .pop()
            .unwrap_or_else(|| panic!("no {name} event; got {:?}", self.events.names()))
    }

    fn exports(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.project.join("exports"))
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn seams(&self, seams: ExportSeams) {
        self.core.state().exp.seams = Arc::new(seams);
    }

    fn decode(&self, rel: &str) -> RgbImage {
        image::open(self.project.join(rel)).unwrap().to_rgb8()
    }

    /// Adds a patch layer to the in-memory stack (the snapshot `export_start` takes).
    fn add_layer(&self, image: &RgbaImage, x: i64, y: i64) -> Ulid {
        let id = self.core.ctx.new_id();
        let rel = layer_file(id);
        let path = self.project.join(&rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image.save(&path).unwrap();
        let mut l = Layer::original(id, 0, 0);
        l.kind = LayerKind::Patch;
        l.file = Some(rel);
        let (w, h) = image.dimensions();
        l.cx = x as f64 + f64::from(w) / 2.0;
        l.cy = y as f64 + f64::from(h) / 2.0;
        l.rect = Some(Rect { x, y, w, h });
        self.core
            .state()
            .project
            .as_mut()
            .unwrap()
            .manifest
            .layers
            .push(l);
        id
    }
}

// ------------------------------------------------------------------ settings and plan

#[test]
fn fr_exp_13_14_destination_and_name_from_the_project_folder() {
    let t = setup();
    let plan = t
        .core
        .dispatch(
            "export_plan",
            json!({"settings": settings(None, None, "png")}),
        )
        .unwrap();
    assert_eq!(plan["path"], "exports/Crypt.png");
    let plan = t
        .core
        .dispatch(
            "export_plan",
            json!({"settings": settings(Some(16.0), Some(8), "jpeg")}),
        )
        .unwrap();
    assert_eq!(plan["path"], "exports/Crypt_8px.jpg");
    assert_eq!(
        (plan["width"].as_u64(), plan["height"].as_u64()),
        (Some(32), Some(24))
    );
    assert_eq!(plan["scale"], 0.5);
    let plan = t
        .core
        .dispatch(
            "export_plan",
            json!({"settings": settings(None, None, "webp")}),
        )
        .unwrap();
    assert_eq!(plan["path"], "exports/Crypt.webp");
}

#[test]
fn fr_exp_06_08_r43_invalid_settings_carry_field_and_limit() {
    let t = setup();
    let cases = [
        (
            settings(None, Some(70), "png"),
            "target_px_per_square",
            json!({"requires": "source_px_per_square"}),
        ),
        (
            settings(Some(16.0), Some(2000), "png"),
            "target_px_per_square",
            json!({"max": 1024.0}),
        ),
        (
            settings(Some(4.0), None, "png"),
            "source_px_per_square",
            json!({"min": 8.0}),
        ),
        (
            settings(Some(8.0), Some(64), "png"),
            "scale",
            json!({"max": 4.0}),
        ),
        (
            json!({"grid": {"mode": "cells", "cols": 0}, "target_px_per_square": null, "format": "png"}),
            "cols",
            json!({"min": 1.0}),
        ),
        (
            json!({"grid": {"mode": "cells", "cols": 4, "rows": 5000}, "target_px_per_square": null, "format": "png"}),
            "rows",
            json!({"max": 1000.0}),
        ),
    ];
    for (s, field, limit) in cases {
        for cmd in ["export_plan", "export_set_settings"] {
            let e = t
                .core
                .dispatch(cmd, json!({"settings": s.clone()}))
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidSettings, "{cmd} {s}");
            assert_eq!(e.field.as_deref(), Some(field), "{cmd} {s}");
            assert_eq!(e.detail["limit"], limit, "{cmd} {s}");
        }
    }
    // The inactive grid mode's values aren't checked.
    let s = json!({"grid": {"mode": "px", "source_px_per_square": 16.0, "cols": 0}, "target_px_per_square": null, "format": "png"});
    t.core
        .dispatch("export_plan", json!({"settings": s}))
        .unwrap();
}

#[test]
fn fr_exp_06_cells_and_grid_not_square_warning() {
    let t = setup();
    let s = json!({"grid": {"mode": "cells", "cols": 4, "rows": 2}, "target_px_per_square": 8, "format": "png"});
    let plan = t
        .core
        .dispatch("export_plan", json!({"settings": s}))
        .unwrap();
    // 64 ÷ 4 = 16 px per square; 48 ÷ 2 = 24 → not square.
    assert_eq!(
        (plan["width"].as_u64(), plan["scale"].as_f64()),
        (Some(32), Some(0.5))
    );
    assert_eq!(plan["warnings"][0]["code"], "grid_not_square");
    assert_eq!(plan["warnings"][0]["message"], "Grid isn't square");
}

#[test]
fn fr_exp_11_webp_unavailable_over_16383_and_start_refuses_it() {
    let t = setup();
    {
        let mut s = t.core.state();
        let m = &mut s.project.as_mut().unwrap().manifest;
        m.original.width = 16384;
    }
    let plan = t
        .core
        .dispatch(
            "export_plan",
            json!({"settings": settings(None, None, "png")}),
        )
        .unwrap();
    let webp = &plan["formats"][2];
    assert_eq!(
        (&webp["format"], &webp["available"], &webp["reason"]),
        (&json!("webp"), &json!(false), &json!("WebP max 16383 px"))
    );
    assert_eq!(plan["formats"][0]["available"], true);
    let e = t.start(settings(None, None, "webp"), false).unwrap_err();
    assert_eq!(e.code, ErrorCode::TooLarge);
    assert_eq!(e.detail, json!({"format": "webp", "limit": 16383}));
    assert!(t.exports().is_empty());
}

#[test]
fn fr_exp_24_plan_is_pure_and_reports_exists_and_peak() {
    let t = setup();
    let seq = t.seq();
    let plan = t
        .core
        .dispatch(
            "export_plan",
            json!({"settings": settings(None, None, "png")}),
        )
        .unwrap();
    assert_eq!(plan["exists"], Value::Null);
    assert!(plan["est_peak_mib"].as_u64().unwrap() >= 1);
    assert_eq!(t.seq(), seq, "a plan never commits");
    assert!(t.exports().is_empty(), "a plan never writes");
    t.start(settings(None, None, "png"), false).unwrap();
    let plan = t
        .core
        .dispatch(
            "export_plan",
            json!({"settings": settings(None, None, "png")}),
        )
        .unwrap();
    let bytes = std::fs::metadata(t.project.join("exports/Crypt.png"))
        .unwrap()
        .len();
    assert_eq!(plan["exists"]["bytes"], bytes);
    assert!(plan["exists"]["mtime"].is_string());
}

#[test]
fn fr_exp_24_peak_over_2_gib_warns() {
    let sizing = pf_imaging::plan_size(
        16384,
        16384,
        Some(pf_imaging::GridInput::PxPerSquare(140.0)),
        Some(70),
    )
    .unwrap();
    let est = super::settings::est_peak_mib(16384, 16384, &sizing, super::ExportFormat::Png);
    assert!(est > super::settings::PEAK_WARN_MIB, "{est}");
    let t = setup();
    {
        let mut s = t.core.state();
        let m = &mut s.project.as_mut().unwrap().manifest;
        (m.original.width, m.original.height) = (16384, 16384);
    }
    let plan = t
        .core
        .dispatch(
            "export_plan",
            json!({"settings": settings(Some(140.0), Some(70), "png")}),
        )
        .unwrap();
    assert!(
        plan["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "memory")
    );
}

#[test]
fn fr_exp_25_settings_commit_only_with_the_debounced_draft_save() {
    let t = setup();
    let seq = t.seq();
    for target in [8, 9, 10] {
        t.core
            .dispatch(
                "export_set_settings",
                json!({"settings": settings(Some(16.0), Some(target), "jpeg")}),
            )
            .unwrap();
        t.clock.advance(Duration::from_millis(500));
    }
    assert_eq!(t.seq(), seq, "a keystroke never commits");
    assert!(!super::commands::flush_settings_if_due(&t.core).unwrap());
    t.clock.advance(Duration::from_millis(1600));
    assert!(super::commands::flush_settings_if_due(&t.core).unwrap());
    assert_eq!(t.seq(), seq + 1, "one commit");
    assert!(!super::commands::flush_settings_if_due(&t.core).unwrap());
    assert_eq!(t.seq(), seq + 1);
    let rec = t.manifest_export();
    assert_eq!(rec["target_px_per_square"], 10);
    assert_eq!(rec["format"], "jpeg");
    assert_eq!(
        rec["grid"],
        json!({"mode": "px", "source_px_per_square": 16.0})
    );

    // Reopen without an export: field-identical (OQ-EXP-04).
    let before = t.core.dispatch("export_get_settings", json!({})).unwrap();
    t.core.dispatch("project_close", json!({})).unwrap();
    t.core
        .dispatch("project_open", json!({"path": t.project}))
        .unwrap();
    let after = t.core.dispatch("export_get_settings", json!({})).unwrap();
    assert_eq!(before, after);
    assert_eq!(after["source_px_per_square"], 16.0);
    assert_eq!(after["target_px_per_square"], 10);
}

#[test]
fn fr_exp_25_r30_project_put_draft_carries_pending_settings_in_its_commit() {
    let t = setup();
    let seq = t.seq();
    t.core
        .dispatch(
            "export_set_settings",
            json!({"settings": settings(Some(16.0), Some(8), "webp")}),
        )
        .unwrap();
    t.core
        .dispatch(
            "project_put_draft",
            json!({"draft": {"final_prompt": "moss"}}),
        )
        .unwrap();
    assert_eq!(t.seq(), seq + 1, "draft and settings in one commit");
    assert_eq!(t.manifest_export()["format"], "webp");
    t.clock.advance(Duration::from_secs(5));
    assert!(
        !super::commands::flush_settings_if_due(&t.core).unwrap(),
        "nothing left"
    );
}

#[test]
fn fr_exp_25_close_commits_pending_settings() {
    let t = setup();
    t.core
        .dispatch(
            "export_set_settings",
            json!({"settings": settings(Some(16.0), Some(8), "jpeg")}),
        )
        .unwrap();
    assert_eq!(t.manifest_export(), Value::Null);
    t.core.dispatch("project_close", json!({})).unwrap();
    assert_eq!(t.manifest_export()["target_px_per_square"], 8);
}

#[test]
fn fr_exp_26_settings_changed_fires_when_source_or_target_change() {
    let t = setup();
    let got = t.core.dispatch("export_get_settings", json!({})).unwrap();
    assert_eq!(
        got,
        json!({"source_px_per_square": null, "target_px_per_square": null})
    );
    let set = |s: Value| {
        t.core
            .dispatch("export_set_settings", json!({"settings": s}))
            .unwrap()
    };
    set(settings(Some(16.0), None, "png"));
    set(settings(Some(16.0), None, "jpeg"));
    let out = set(settings(Some(16.0), Some(8), "jpeg"));
    assert_eq!(out["source_px_per_square"], 16.0);
    assert_eq!(out["target_px_per_square"], 8);
    assert_eq!(out["format"], "jpeg");
    set(json!({"grid": {"mode": "cells", "cols": 4}, "target_px_per_square": 8, "format": "jpeg"}));
    assert_eq!(
        t.events("export:settings-changed"),
        [
            json!({"source": 16.0, "target": null}),
            json!({"source": 16.0, "target": 8}),
        ],
        "format and an equal source from cells don't fire"
    );
}

// ------------------------------------------------------------------ export

#[test]
fn fr_exp_16_27_png_export_equals_the_original_and_records_last() {
    let img = map_image(64, 48);
    let t = setup_with(&img, Faults::none());
    let out = t.start(settings(None, None, "png"), false).unwrap();
    let done = t.last_event("export:done");
    assert_eq!(done["export_id"], out["export_id"]);
    assert_eq!(done["record_saved"], true);
    assert_eq!(done["fallback"], Value::Null);
    let abs = t.project.join("exports").join("Crypt.png");
    // Absolute, with the platform's separators only.
    assert_eq!(done["path"].as_str(), abs.to_str());
    assert_eq!(
        t.decode("exports/Crypt.png"),
        img,
        "golden (a): no layers = original"
    );
    assert_eq!(t.exports(), ["Crypt.png"], "no temp file left");

    let last = &t.manifest_export()["last"];
    let p = t.core.state();
    let p = p.project.as_ref().unwrap();
    assert_eq!(last["path"], "exports/Crypt.png", "project-relative");
    assert_eq!(last["revision_id"], json!(p.manifest.head_revision));
    assert_eq!(
        last["layers_hash"],
        super::run::layers_hash(&p.manifest.layers)
    );
    assert!(last["layers_hash"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(
        (last["width"].as_u64(), last["height"].as_u64()),
        (Some(64), Some(48))
    );
    assert_eq!(last["bytes"], done["bytes"]);
    assert_eq!(
        (last["strip_h"].as_u64(), last["compose_version"].as_u64()),
        (Some(512), Some(1))
    );
    assert_eq!(last["format"], "png");
    assert!(last["at"].is_string());
}

#[test]
fn fr_exp_27_layers_hash_is_sha256_of_canonical_layers_json() {
    use sha2::Digest;
    let layers = vec![Layer::original(Ulid::from_parts(0, 1), 10, 20)];
    let canonical = serde_json::to_vec(&layers).unwrap();
    assert!(!canonical.contains(&b' ') && !canonical.contains(&b'\n'));
    let want: String = sha2::Sha256::digest(&canonical)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(super::run::layers_hash(&layers), format!("sha256:{want}"));
}

#[test]
fn fr_exp_09_rescale_jpeg_and_webp_decode_at_the_planned_size() {
    let t = setup_with(&map_image(128, 96), Faults::none());
    for fmt in ["jpeg", "webp", "png"] {
        t.start(settings(Some(16.0), Some(8), fmt), false).unwrap();
        let done = t.last_event("export:done");
        assert_eq!(
            (done["width"].as_u64(), done["height"].as_u64()),
            (Some(64), Some(48))
        );
        let ext = if fmt == "jpeg" { "jpg" } else { fmt };
        let img = t.decode(&format!("exports/Crypt_8px.{ext}"));
        assert_eq!(img.dimensions(), (64, 48), "{fmt}");
    }
    let phases: Vec<String> = t
        .events("export:progress")
        .iter()
        .map(|p| p["phase"].as_str().unwrap().to_owned())
        .collect();
    assert!(phases.contains(&"resize".to_owned()) && phases.contains(&"encode".to_owned()));
}

#[test]
fn fr_exp_19_progress_has_phases_and_fractions_and_ends_written() {
    let t = setup_with(&map_image(64, 1200), Faults::none());
    t.start(settings(None, None, "png"), false).unwrap();
    let progress = t.events("export:progress");
    assert!(progress.len() >= 3, "{progress:?}");
    for p in &progress {
        let f = p["fraction"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&f));
        assert!(["compose", "resize", "encode", "write"].contains(&p["phase"].as_str().unwrap()));
    }
    let last = progress.last().unwrap();
    assert_eq!(
        (&last["phase"], last["fraction"].as_f64()),
        (&json!("write"), Some(1.0))
    );
}

#[test]
fn fr_exp_15_exists_unless_replace_and_other_files_untouched() {
    let t = setup();
    t.start(settings(None, None, "png"), false).unwrap();
    std::fs::write(t.project.join("exports/mine.txt"), b"user file").unwrap();
    std::fs::write(t.project.join("exports/Crypt (conflict copy).png"), b"x").unwrap();
    let seq = t.seq();
    let e = t.start(settings(None, None, "png"), false).unwrap_err();
    assert_eq!(e.code, ErrorCode::Exists);
    let bytes = std::fs::metadata(t.project.join("exports/Crypt.png"))
        .unwrap()
        .len();
    assert_eq!(e.detail["bytes"], bytes);
    assert!(e.detail["mtime"].is_string());
    assert_eq!(t.seq(), seq, "nothing written");
    t.start(settings(None, None, "png"), true).unwrap();
    assert_eq!(t.last_event("export:done")["record_saved"], true);
    assert_eq!(
        t.exports(),
        ["Crypt (conflict copy).png", "Crypt.png", "mine.txt"]
    );
    assert_eq!(
        std::fs::read(t.project.join("exports/mine.txt")).unwrap(),
        b"user file"
    );
}

#[test]
fn fr_exp_16_fault_before_the_rename_leaves_the_old_file_and_no_temp() {
    // A target names the export `Crypt_16px.png` (the original's copy is `Crypt.png`).
    let faults = Faults::parse("after_temp:Crypt_16px.png", FaultAction::Error).unwrap();
    let t = setup_with(&map_image(64, 48), faults);
    std::fs::create_dir_all(t.project.join("exports")).unwrap();
    std::fs::write(t.project.join("exports/Crypt_16px.png"), b"old export").unwrap();
    let last_before = t.manifest_export()["last"].clone();
    t.start(settings(Some(16.0), Some(16), "png"), true)
        .unwrap();
    let failed = t.last_event("export:failed");
    assert_eq!(failed["code"], "io");
    assert_eq!(
        std::fs::read(t.project.join("exports/Crypt_16px.png")).unwrap(),
        b"old export"
    );
    assert_eq!(t.exports(), ["Crypt_16px.png"], "temp removed");
    assert_eq!(t.manifest_export()["last"], last_before);
    assert!(t.core.state().exp.running.is_empty());
}

#[test]
fn fr_exp_16_fault_after_the_rename_leaves_a_whole_file_and_the_previous_last() {
    let faults = Faults::parse("after_rename:Crypt_16px.png", FaultAction::Error).unwrap();
    let img = map_image(64, 48);
    let t = setup_with(&img, faults);
    t.start(settings(Some(16.0), Some(16), "png"), false)
        .unwrap();
    assert_eq!(t.last_event("export:failed")["code"], "io");
    assert_eq!(
        t.decode("exports/Crypt_16px.png"),
        img,
        "the renamed file is whole"
    );
    assert_eq!(
        t.manifest_export()["last"],
        Value::Null,
        "last isn't committed"
    );
}

#[test]
fn fr_exp_17_stale_temp_files_are_swept_before_writing() {
    let t = setup();
    let exports = t.project.join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    let old = exports.join(".Crypt.png.pf-tmp-01old");
    let fresh = exports.join(".Crypt.png.pf-tmp-01fresh");
    let other = exports.join("notes.pf-tmp-keep.txt");
    for p in [&old, &fresh, &other] {
        std::fs::write(p, b"x").unwrap();
    }
    let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(7200);
    for p in [&old, &other] {
        std::fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(two_hours_ago)
            .unwrap();
    }
    t.start(settings(None, None, "png"), false).unwrap();
    assert!(!old.exists(), "stale temp swept");
    assert!(fresh.exists(), "a recent temp may be another export's");
    assert!(other.exists(), "only `.*.pf-tmp-*`");
}

fn locked_error() -> io::Error {
    io::Error::other("test: sharing violation")
}

fn is_test_lock(e: &io::Error) -> bool {
    e.to_string().contains("sharing violation")
}

#[test]
fn fr_exp_18_locked_destination_is_retried_then_succeeds() {
    let t = setup();
    let fails = Arc::new(AtomicUsize::new(2));
    let f = fails.clone();
    t.seams(ExportSeams {
        rename: Box::new(move |from, to| {
            if f.load(Ordering::SeqCst) > 0 {
                f.fetch_sub(1, Ordering::SeqCst);
                return Err(locked_error());
            }
            std::fs::rename(from, to)
        }),
        is_locked: Box::new(is_test_lock),
        ..ExportSeams::default()
    });
    t.start(settings(None, None, "png"), false).unwrap();
    assert_eq!(fails.load(Ordering::SeqCst), 0);
    assert_eq!(t.last_event("export:done")["record_saved"], true);
    assert_eq!(t.exports(), ["Crypt.png"]);
}

#[test]
fn fr_exp_18_locked_for_too_long_fails_locked_and_removes_the_temp() {
    let t = setup();
    t.seams(ExportSeams {
        rename: Box::new(|_, _| Err(locked_error())),
        is_locked: Box::new(is_test_lock),
        lock_retry_for: Duration::from_millis(120),
        ..ExportSeams::default()
    });
    t.start(settings(None, None, "png"), false).unwrap();
    let failed = t.last_event("export:failed");
    assert_eq!(failed["code"], "locked");
    assert_eq!(
        failed["detail"]["message"],
        "The file is in use (VTT or sync?)"
    );
    assert!(t.exports().is_empty(), "no temp, no final file");
}

/// A WebP encoder that overflows lossy encodes, or fails with a given error.
struct FakeWebp {
    lossy: fn() -> EncodeError,
    calls: AtomicUsize,
}

impl WebpEncoder for FakeWebp {
    fn encode(
        &self,
        image: &RgbImage,
        mode: WebpMode,
        progress: &mut dyn FnMut(f32) -> ControlFlow<()>,
    ) -> Result<Vec<u8>, EncodeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if progress(0.5).is_break() {
            return Err(EncodeError::Canceled);
        }
        match mode {
            WebpMode::Lossy { .. } => Err((self.lossy)()),
            WebpMode::Lossless => LibWebp.encode(image, mode, progress),
        }
    }
}

#[test]
fn fr_exp_12_partition_overflow_retries_lossless_once() {
    let img = map_image(64, 48);
    let t = setup_with(&img, Faults::none());
    let webp = Arc::new(FakeWebp {
        lossy: || EncodeError::PartitionOverflow,
        calls: AtomicUsize::new(0),
    });
    t.seams(ExportSeams {
        webp: webp.clone(),
        ..ExportSeams::default()
    });
    t.start(settings(None, None, "webp"), false).unwrap();
    assert_eq!(webp.calls.load(Ordering::SeqCst), 2);
    let done = t.last_event("export:done");
    assert_eq!(done["fallback"], "lossless");
    assert_eq!(t.manifest_export()["last"]["fallback"], "lossless");
    assert_eq!(t.decode("exports/Crypt.webp"), img, "lossless");
}

#[test]
fn fr_exp_23_allocation_failure_is_out_of_memory() {
    let t = setup();
    t.seams(ExportSeams {
        webp: Arc::new(FakeWebp {
            lossy: || EncodeError::OutOfMemory,
            calls: AtomicUsize::new(0),
        }),
        ..ExportSeams::default()
    });
    t.start(settings(None, None, "webp"), false).unwrap();
    assert_eq!(t.last_event("export:failed")["code"], "out_of_memory");
    assert!(t.exports().is_empty());
    // The app keeps running: the next export works.
    t.seams(ExportSeams::default());
    t.start(settings(None, None, "png"), false).unwrap();
    assert_eq!(t.last_event("export:done")["record_saved"], true);
}

#[test]
fn fr_exp_20_cancel_mid_strip_leaves_nothing() {
    let t = setup_with(&map_image(64, 1600), Faults::none());
    std::fs::create_dir_all(t.project.join("exports")).unwrap();
    std::fs::write(t.project.join("exports/Crypt.png"), b"previous").unwrap();
    t.seams(ExportSeams {
        cancel_after_progress: Some(3),
        ..ExportSeams::default()
    });
    let out = t.start(settings(None, None, "png"), true).unwrap();
    assert_eq!(
        t.last_event("export:canceled")["export_id"],
        out["export_id"]
    );
    assert!(t.events("export:done").is_empty());
    assert_eq!(t.exports(), ["Crypt.png"], "no temp");
    assert_eq!(
        std::fs::read(t.project.join("exports/Crypt.png")).unwrap(),
        b"previous"
    );
    let e = t
        .core
        .dispatch("export_cancel", json!({"export_id": out["export_id"]}))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::UnknownId, "already finished");
}

#[test]
fn fr_exp_20_cancel_mid_webp_through_the_progress_hook() {
    let t = setup();
    let webp = Arc::new(FakeWebp {
        lossy: || EncodeError::Failed("unreachable".into()),
        calls: AtomicUsize::new(0),
    });
    // Reports: start, the one strip, then the encoder's hook.
    t.seams(ExportSeams {
        webp: webp.clone(),
        cancel_after_progress: Some(3),
        ..ExportSeams::default()
    });
    t.start(settings(None, None, "webp"), false).unwrap();
    assert_eq!(webp.calls.load(Ordering::SeqCst), 1);
    assert_eq!(t.events("export:canceled").len(), 1);
    assert!(t.exports().is_empty());
}

#[test]
fn fr_exp_21_r31_busy_while_an_export_or_a_fetching_job_runs() {
    let t = setup();
    let id = t.core.state().project.as_ref().unwrap().manifest.project_id;
    t.core.state().exp.running.push(super::Running {
        id: t.core.ctx.new_id(),
        project_id: id,
        cancel: Arc::default(),
    });
    let e = t.start(settings(None, None, "png"), false).unwrap_err();
    assert_eq!(e.code, ErrorCode::Busy);
    t.core.state().exp.running.clear();

    // R31: a region job in `succeeded` is fetching its candidates.
    {
        let mut s = t.core.state();
        let install = s.settings.install_id;
        let p = s.project.as_mut().unwrap();
        let ctx = &t.core.ctx;
        let mask = crate::project::store::put_mask(
            ctx,
            install,
            p,
            Rect {
                x: 0,
                y: 0,
                w: 8,
                h: 8,
            },
            vec![255; 64],
        )
        .unwrap();
        let job = crate::project::store::record_job(
            ctx,
            install,
            p,
            crate::project::store::NewJob {
                operation: "edit_masked".into(),
                model_id: "fake-model".into(),
                candidate_count: 1,
                region: json!({"inner": {"x": 0, "y": 0, "w": 8, "h": 8}}),
                mask_id: Some(mask.id),
                ..Default::default()
            },
            crate::test_support::rgba_png(8, 8, [1, 2, 3, 255]),
            crate::test_support::rgba_png(8, 8, [255, 255, 255, 255]),
        )
        .unwrap();
        for st in [
            crate::job::JobState::Submitted,
            crate::job::JobState::Running,
            crate::job::JobState::Succeeded,
        ] {
            crate::project::store::update_job(
                ctx,
                p,
                job.id,
                crate::project::store::JobUpdate {
                    state: Some(st),
                    provider: Some(json!({"handle": "h"})),
                    ..Default::default()
                },
            )
            .unwrap();
        }
    }
    let e = t.start(settings(None, None, "png"), false).unwrap_err();
    assert_eq!(e.code, ErrorCode::Busy);
}

#[test]
fn fr_exp_22_missing_layer_file_fails_with_layer_unreadable() {
    let t = setup();
    let id = t.add_layer(
        &RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255])),
        4,
        4,
    );
    std::fs::remove_file(t.project.join(layer_file(id))).unwrap();
    t.start(settings(None, None, "png"), false).unwrap();
    let failed = t.last_event("export:failed");
    assert_eq!(failed["code"], "layer_unreadable");
    assert_eq!(failed["detail"]["layer_id"], json!(id));
    assert!(
        t.exports().is_empty(),
        "no partial or layer-skipping output"
    );
}

#[test]
fn fr_exp_02_hidden_layers_are_skipped_and_patches_change_only_their_rect() {
    let img = map_image(64, 48);
    let t = setup_with(&img, Faults::none());
    t.add_layer(
        &RgbaImage::from_pixel(8, 6, image::Rgba([0, 255, 0, 255])),
        10,
        12,
    );
    let hidden = t.add_layer(
        &RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255])),
        30,
        30,
    );
    t.core
        .state()
        .project
        .as_mut()
        .unwrap()
        .manifest
        .layers
        .iter_mut()
        .find(|l| l.id == hidden)
        .unwrap()
        .visible = false;
    t.start(settings(None, None, "png"), false).unwrap();
    let out = t.decode("exports/Crypt.png");
    for (x, y, px) in out.enumerate_pixels() {
        let inside = (10..18).contains(&x) && (12..18).contains(&y);
        if inside {
            assert_eq!(px.0, [0, 255, 0]);
        } else {
            assert_eq!(px, img.get_pixel(x, y), "({x},{y})");
        }
    }
}

#[test]
fn fr_exp_28_original_unavailable_until_decoded() {
    let t = setup();
    {
        let mut s = t.core.state();
        let p = s.project.as_mut().unwrap();
        p.decoded = crate::decoded::DecodedOriginal::decoding_for_tests(p.manifest.project_id);
    }
    let e = t.start(settings(None, None, "png"), false).unwrap_err();
    assert_eq!(e.code, ErrorCode::OriginalUnavailable);
    assert!(t.exports().is_empty());
}

#[test]
fn r43_prefetch_reemits_original_ready_when_decoded() {
    let t = setup();
    let before = t.events("project:original-ready").len();
    let out = t
        .core
        .dispatch("project_prefetch_original", json!({}))
        .unwrap();
    assert_eq!(out["ready"], true);
    assert_eq!(t.events("project:original-ready").len(), before + 1);
}

#[test]
fn fr_exp_29_read_only_plans_but_start_writes_nothing() {
    let t = setup();
    t.core.state().project.as_mut().unwrap().read_only = Some(ReadOnlyReason::OriginalChanged);
    let seq = t.seq();
    t.core
        .dispatch(
            "export_plan",
            json!({"settings": settings(None, None, "png")}),
        )
        .unwrap();
    t.core
        .dispatch(
            "export_set_settings",
            json!({"settings": settings(Some(16.0), None, "png")}),
        )
        .unwrap();
    let e = t.start(settings(None, None, "png"), false).unwrap_err();
    assert_eq!(e.code, ErrorCode::ReadOnly);
    assert_eq!(t.seq(), seq);
    assert!(t.exports().is_empty());
    t.clock.advance(Duration::from_secs(5));
    assert!(!super::commands::flush_settings_if_due(&t.core).unwrap());
}

#[test]
fn fr_exp_25_start_commits_the_settings_first() {
    let t = setup();
    let seq = t.seq();
    t.start(settings(Some(16.0), Some(8), "jpeg"), false)
        .unwrap();
    // Settings commit, then `last`.
    assert_eq!(t.seq(), seq + 2);
    let rec = t.manifest_export();
    assert_eq!(
        (rec["format"].as_str(), rec["target_px_per_square"].as_i64()),
        (Some("jpeg"), Some(8))
    );
    assert_eq!(rec["last"]["path"], "exports/Crypt_8px.jpg");
    // Reopen after an export: settings and `last` field-identical.
    let before = t.manifest_export();
    t.core.dispatch("project_close", json!({})).unwrap();
    t.core
        .dispatch("project_open", json!({"path": t.project}))
        .unwrap();
    assert_eq!(t.manifest_export(), before);
    let got = t.core.dispatch("export_get_settings", json!({})).unwrap();
    assert_eq!(
        (
            got["source_px_per_square"].as_f64(),
            got["target_px_per_square"].as_i64()
        ),
        (Some(16.0), Some(8))
    );
}

#[test]
fn close_hook_cancels_a_running_export() {
    let t = setup();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let id = t.core.state().project.as_ref().unwrap().manifest.project_id;
    t.core.state().exp.running.push(super::Running {
        id: t.core.ctx.new_id(),
        project_id: id,
        cancel: cancel.clone(),
    });
    t.core.dispatch("project_close", json!({})).unwrap();
    assert!(cancel.load(Ordering::SeqCst));
    let _ = &t.dir;
}

// ------------------------------------------------------------------ goldens through the core

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn assert_golden(name: &str, got: &RgbImage) {
    let path = fixtures()
        .join("goldens/export")
        .join(format!("{name}.png"));
    if std::env::var("PF_UPDATE_GOLDENS").as_deref() == Ok("1") {
        got.save(&path).unwrap();
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
    assert_eq!(got.dimensions(), want.dimensions(), "{name}");
    let tolerance: u8 = if cfg!(windows) { 0 } else { 1 };
    let bad = got
        .as_raw()
        .iter()
        .zip(want.as_raw())
        .filter(|(a, b)| a.abs_diff(**b) > tolerance)
        .count();
    assert_eq!(
        bad, 0,
        "{name}: {bad} channels differ by more than {tolerance}"
    );
}

/// A fake-provider candidate for a `size`² region of `base` at (x, y), with a disc mask.
fn fake_candidate(base: &RgbImage, x: u32, y: u32, size: u32, seed: u64) -> RgbImage {
    use crate::provider::{JobRequest, Operation, Provider, block_on, fake::FakeProvider};
    let region = image::imageops::crop_imm(base, x, y, size, size).to_image();
    let c = f64::from(size) / 2.0;
    let mask = image::GrayImage::from_fn(size, size, |px, py| {
        let d = ((f64::from(px) + 0.5 - c).powi(2) + (f64::from(py) + 0.5 - c).powi(2)).sqrt();
        image::Luma([if d < c * 0.8 { 255 } else { 0 }])
    });
    let mut mask_png = Vec::new();
    mask.write_to(&mut io::Cursor::new(&mut mask_png), image::ImageFormat::Png)
        .unwrap();
    let req = JobRequest {
        operation: Operation::EditMasked,
        model_id: "fake-white-edit".into(),
        final_prompt: "mossy flagstones".into(),
        controls: Default::default(),
        references: vec![],
        region_png: png_of(&region),
        mask_png,
        candidate_count: 1,
        seed: Some(seed),
    };
    let p = FakeProvider::default();
    let mut rec = block_on(p.submit(&req)).unwrap();
    for _ in 0..3 {
        rec = block_on(p.status(&rec)).unwrap().provider;
    }
    let cands = block_on(p.fetch_candidates(&rec)).unwrap();
    image::load_from_memory(&cands.images[0].bytes)
        .unwrap()
        .to_rgb8()
}

/// The candidate baked as a patch: alpha is a feathered disc (synthetic, like RGN's feather).
fn bake(candidate: &RgbImage, feather: f64) -> RgbaImage {
    let (w, _) = candidate.dimensions();
    let c = f64::from(w) / 2.0;
    RgbaImage::from_fn(w, w, |x, y| {
        let d = ((f64::from(x) + 0.5 - c).powi(2) + (f64::from(y) + 0.5 - c).powi(2)).sqrt();
        let a = ((c * 0.8 - d) / feather).clamp(0.0, 1.0);
        let p = candidate.get_pixel(x, y).0;
        image::Rgba([p[0], p[1], p[2], (a * 255.0).round() as u8])
    })
}

/// Golden (b): three baked patches from fake-provider candidates at fixed seeds, exported as
/// PNG through `export_start` (EXP §13).
#[test]
fn fr_exp_01_golden_b_three_fake_provider_patches() {
    let base = image::open(fixtures().join("images/grid-map-1024x1280.png"))
        .unwrap()
        .to_rgb8();
    let t = setup_with(&base, Faults::none());
    for (i, (x, y, size)) in [(96u32, 128u32, 192u32), (520, 300, 256), (300, 820, 160)]
        .into_iter()
        .enumerate()
    {
        let cand = fake_candidate(&base, x, y, size, 1000 + i as u64);
        t.add_layer(&bake(&cand, 12.0), i64::from(x), i64::from(y));
    }
    t.start(settings(None, None, "png"), false).unwrap();
    let out = t.decode("exports/Crypt.png");
    assert_golden("b-fake-patches", &out);
    assert_eq!(
        out.get_pixel(5, 5),
        base.get_pixel(5, 5),
        "outside every patch"
    );
    assert_ne!(out, base);
}

/// R3: accept a patch, restore the parent revision, export → golden (a) (the original).
#[test]
fn r3_accept_then_restore_parent_then_export_equals_the_original() {
    let img = map_image(64, 48);
    let t = setup_with(&img, Faults::none());
    let parent = t
        .core
        .state()
        .project
        .as_ref()
        .unwrap()
        .manifest
        .head_revision;
    {
        let mut s = t.core.state();
        let install = s.settings.install_id;
        let p = s.project.as_mut().unwrap();
        let ctx = &t.core.ctx;
        use crate::project::store::*;
        let mask = put_mask(
            ctx,
            install,
            p,
            Rect {
                x: 4,
                y: 4,
                w: 8,
                h: 6,
            },
            vec![255; 48],
        )
        .unwrap();
        let job = record_job(
            ctx,
            install,
            p,
            NewJob {
                operation: "edit_masked".into(),
                model_id: "fake-model".into(),
                candidate_count: 1,
                region: json!({"inner": {"x": 4, "y": 4, "w": 8, "h": 6}}),
                mask_id: Some(mask.id),
                ..Default::default()
            },
            crate::test_support::rgba_png(8, 6, [1, 2, 3, 255]),
            crate::test_support::rgba_png(8, 6, [255, 255, 255, 255]),
        )
        .unwrap();
        for st in [
            crate::job::JobState::Submitted,
            crate::job::JobState::Running,
            crate::job::JobState::Succeeded,
        ] {
            update_job(
                ctx,
                p,
                job.id,
                JobUpdate {
                    state: Some(st),
                    provider: Some(json!({"handle": "h"})),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let cands = add_candidates(
            ctx,
            install,
            p,
            job.id,
            vec![NewCandidate {
                bytes: crate::test_support::rgba_png(8, 6, [0, 255, 0, 255]),
                seed: Some(1),
                provider_meta: json!({}),
                invalid: false,
            }],
        )
        .unwrap();
        accept_candidate(
            ctx,
            install,
            p,
            job.id,
            cands[0].id,
            NewPatch {
                png: crate::test_support::rgba_png(8, 6, [0, 255, 0, 255]),
                rect: Some(Rect {
                    x: 4,
                    y: 4,
                    w: 8,
                    h: 6,
                }),
                region: json!({}),
                blend: json!({}),
            },
            None,
        )
        .unwrap();
    }
    t.start(settings(None, None, "png"), false).unwrap();
    let patched = t.decode("exports/Crypt.png");
    assert_eq!(patched.get_pixel(5, 5).0, [0, 255, 0]);
    assert_eq!(patched.get_pixel(30, 30), img.get_pixel(30, 30));

    t.core
        .dispatch("project_restore_revision", json!({"rev_id": parent}))
        .unwrap();
    t.start(settings(None, None, "png"), true).unwrap();
    assert_eq!(t.decode("exports/Crypt.png"), img, "golden (a) again");
    let last = &t.manifest_export()["last"];
    let s = t.core.state();
    let p = s.project.as_ref().unwrap();
    assert_eq!(
        last["layers_hash"],
        super::run::layers_hash(&p.manifest.layers)
    );
    assert_eq!(last["revision_id"], json!(p.manifest.head_revision));
}

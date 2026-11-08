use serde_json::{Value, json};

use super::*;
use crate::context::Context;
use crate::project::ReadOnlyReason;
use crate::time::Timestamp;

struct Env {
    dir: tempfile::TempDir,
    core: Core,
    snap: Value,
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
    })
    .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
    .unwrap();
    out
}

/// A core with one generated 300×200 map open.
fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(1_800_000_000_000));
    ctx.library_override = Some(dir.path().join("lib"));
    let core = Core::new(ctx).unwrap();
    let src = dir.path().join("map.png");
    std::fs::write(&src, png(300, 200)).unwrap();
    let out = core
        .dispatch(
            "project_create",
            json!({"section": "maps", "paths": [src.to_string_lossy()]}),
        )
        .unwrap();
    let path = out["items"][0]["path"].as_str().unwrap().to_owned();
    let snap = core
        .dispatch("project_open", json!({"path": path}))
        .unwrap();
    Env { dir, core, snap }
}

/// Disc, ramp and checker values (EDT §13 mask patterns), w·h bytes.
fn pattern(w: u32, h: u32) -> Vec<u8> {
    let (cx, cy, r) = (w as f64 / 2.0, h as f64 / 2.0, w.min(h) as f64 / 3.0);
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let d = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
            if d < r {
                255
            } else if (x / 4 + y / 4) % 2 == 0 {
                (x * 255 / w.max(1)) as u8
            } else {
                0
            }
        })
        .collect()
}

impl Env {
    fn pid(&self) -> &Value {
        &self.snap["project_id"]
    }

    fn submit(&self, rect: Value, bytes: &[u8]) -> CoreResult<Value> {
        self.core.dispatch(
            "edt_mask_submit",
            json!({"project_id": self.pid(), "rect": rect, "data_b64": crate::b64::encode(bytes)}),
        )
    }

    fn read(&self, mask_id: &Value) -> CoreResult<Value> {
        self.core.dispatch(
            "edt_mask_read",
            json!({"project_id": self.pid(), "mask_id": mask_id}),
        )
    }

    fn get(&self) -> Value {
        self.core.dispatch("project_get", json!({})).unwrap()
    }

    fn mask_path(&self, mask_id: &Value) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(self.snap["project_dir"].as_str().unwrap());
        dir.join(crate::manifest::mask_file(
            mask_id.as_str().unwrap().parse().unwrap(),
        ))
    }

    fn set_read_only(&self, r: Option<ReadOnlyReason>) {
        self.core.state().project.as_mut().unwrap().read_only = r;
    }
}

fn bytes_of(read: &Value) -> Vec<u8> {
    crate::b64::decode(read["data_b64"].as_str().unwrap()).unwrap()
}

#[test]
fn fr_edt_18_submit_then_read_is_byte_identical_and_an_8_bit_grey_png() {
    let e = env();
    let rect = json!({"x": 40, "y": 30, "w": 97, "h": 61});
    let bytes = pattern(97, 61);
    let r = e.submit(rect.clone(), &bytes).unwrap();
    assert_eq!(r["rect"], rect);
    let read = e.read(&r["mask_id"]).unwrap();
    assert_eq!(read["rect"], rect);
    assert_eq!(bytes_of(&read), bytes);
    // On disk: 8-bit grey, the rect's size, under `.pforge/masks/` (PRJ layout).
    let path = e.mask_path(&r["mask_id"]);
    assert!(path.to_string_lossy().contains(".pforge"));
    let img = image::open(&path).unwrap();
    assert_eq!(img.color(), image::ColorType::L8);
    assert_eq!(img.into_luma8().into_raw(), bytes);
}

#[test]
fn fr_edt_18_submit_validation() {
    let e = env();
    let code = |rect: Value, len: usize| e.submit(rect, &vec![1; len]).unwrap_err();
    let err = code(json!({"x": 0, "y": 0, "w": 0, "h": 4}), 0);
    assert_eq!(
        (err.code, err.field.as_deref()),
        (ErrorCode::InvalidRect, Some("rect"))
    );
    assert_eq!(
        code(json!({"x": 0, "y": 0, "w": 4097, "h": 1}), 4097).code,
        ErrorCode::InvalidRect
    );
    // Outside the 300×200 image.
    assert_eq!(
        code(json!({"x": 290, "y": 0, "w": 20, "h": 4}), 80).code,
        ErrorCode::InvalidRect
    );
    assert_eq!(
        code(json!({"x": -1, "y": 0, "w": 4, "h": 4}), 16).code,
        ErrorCode::InvalidRect
    );
    // Non-integer rect → invalid input at the boundary.
    assert_eq!(
        code(json!({"x": 0.5, "y": 0, "w": 4, "h": 4}), 16).code,
        ErrorCode::InvalidInput
    );
    let err = code(json!({"x": 0, "y": 0, "w": 4, "h": 4}), 15);
    assert_eq!(err.code, ErrorCode::SizeMismatch);
    assert_eq!(err.code.as_str(), "size_mismatch");
    let err = e
        .core
        .dispatch(
            "edt_mask_submit",
            json!({"project_id": e.pid(), "rect": {"x": 0, "y": 0, "w": 2, "h": 2}, "data_b64": "!!"}),
        )
        .unwrap_err();
    assert_eq!(err.field.as_deref(), Some("data_b64"));
    let err = e
        .core
        .dispatch(
            "edt_mask_submit",
            json!({"project_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "rect": {"x": 0, "y": 0, "w": 1, "h": 1}, "data_b64": "AA=="}),
        )
        .unwrap_err();
    assert_eq!(
        (err.code, err.field.as_deref()),
        (ErrorCode::UnknownId, Some("project_id"))
    );
    // Nothing was written.
    assert_eq!(e.get()["draft"]["mask_id"], Value::Null);
}

#[test]
fn fr_edt_29_read_only_submit_fails_fast_and_read_still_works() {
    let e = env();
    let rect = json!({"x": 0, "y": 0, "w": 8, "h": 8});
    let first = e.submit(rect.clone(), &[200; 64]).unwrap();
    e.set_read_only(Some(ReadOnlyReason::NewerVersion));
    let seq = e.get()["commit_seq"].clone();
    // Fails before the body is looked at: even a malformed body gets `read_only`.
    let err = e
        .core
        .dispatch(
            "edt_mask_submit",
            json!({"project_id": e.pid(), "rect": rect, "data_b64": "!!"}),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ReadOnly);
    assert_eq!(
        e.submit(rect, &[1; 64]).unwrap_err().code,
        ErrorCode::ReadOnly
    );
    assert_eq!(e.get()["commit_seq"], seq, "no commit");
    assert_eq!(bytes_of(&e.read(&first["mask_id"]).unwrap()), vec![200; 64]);
}

#[test]
fn fr_edt_28_r7_the_draft_points_at_the_latest_mask_and_superseded_ones_go_to_the_trash() {
    let e = env();
    let rect = json!({"x": 10, "y": 10, "w": 16, "h": 16});
    let a = e.submit(rect.clone(), &[10; 256]).unwrap();
    assert_eq!(e.get()["draft"]["mask_id"], a["mask_id"]);
    let b = e.submit(rect, &[20; 256]).unwrap();
    assert_ne!(a["mask_id"], b["mask_id"]);
    assert_eq!(e.get()["draft"]["mask_id"], b["mask_id"]);
    // `a` is superseded: gone from the manifest's masks (trashed by PRJ at the commit).
    assert_eq!(
        e.read(&a["mask_id"]).unwrap_err().code,
        ErrorCode::UnknownId
    );
    assert_eq!(bytes_of(&e.read(&b["mask_id"]).unwrap()), vec![20; 256]);
}

#[test]
fn fr_edt_28_a_draft_save_without_mask_id_keeps_the_mask_and_null_clears_it() {
    let e = env();
    let m = e
        .submit(json!({"x": 0, "y": 0, "w": 4, "h": 4}), &[255; 16])
        .unwrap();
    // RGN's draft save doesn't carry the mask (it's EDT's).
    e.core
        .dispatch(
            "project_put_draft",
            json!({"draft": {"final_prompt": "moss", "region": {"inner": {"x": 0, "y": 0, "w": 4, "h": 4}, "margin": 8}}}),
        )
        .unwrap();
    let d = &e.get()["draft"];
    assert_eq!(
        (&d["mask_id"], &d["final_prompt"]),
        (&m["mask_id"], &json!("moss"))
    );
    assert!(
        e.read(&m["mask_id"]).is_ok(),
        "still readable after the commit"
    );
    e.core
        .dispatch(
            "project_put_draft",
            json!({"draft": {"final_prompt": "moss", "mask_id": null}}),
        )
        .unwrap();
    assert_eq!(e.get()["draft"]["mask_id"], Value::Null);
}

#[test]
fn fr_edt_19_reopen_reads_the_draft_mask_pixel_identically() {
    let e = env();
    let bytes = pattern(50, 40);
    let m = e
        .submit(json!({"x": 100, "y": 100, "w": 50, "h": 40}), &bytes)
        .unwrap();
    let path = e.snap["project_dir"].as_str().unwrap().to_owned();
    e.core.dispatch("project_close", json!({})).unwrap();
    let again = e
        .core
        .dispatch("project_open", json!({"path": path}))
        .unwrap();
    assert_eq!(again["draft"]["mask_id"], m["mask_id"]);
    assert_eq!(bytes_of(&e.read(&m["mask_id"]).unwrap()), bytes);
}

#[test]
fn edt_11_read_errors_unknown_missing_corrupt() {
    let e = env();
    assert_eq!(
        e.read(&json!("01ARZ3NDEKTSV4RRFFQ69G5FAV"))
            .unwrap_err()
            .code,
        ErrorCode::UnknownId
    );
    let m = e
        .submit(json!({"x": 0, "y": 0, "w": 6, "h": 5}), &[7; 30])
        .unwrap();
    let path = e.mask_path(&m["mask_id"]);
    // Not 8-bit grey.
    std::fs::write(&path, png(6, 5)).unwrap();
    let err = e.read(&m["mask_id"]).unwrap_err();
    assert_eq!(err.code, ErrorCode::Corrupt);
    // Grey but the wrong size.
    let mut out = Vec::new();
    image::GrayImage::new(5, 5)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(&path, out).unwrap();
    assert_eq!(e.read(&m["mask_id"]).unwrap_err().code, ErrorCode::Corrupt);
    std::fs::write(&path, b"not a png").unwrap();
    assert_eq!(e.read(&m["mask_id"]).unwrap_err().code, ErrorCode::Corrupt);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(e.read(&m["mask_id"]).unwrap_err().code, ErrorCode::NotFound);
    drop(e.dir);
}

#[test]
fn fr_edt_18_a_4096_square_mask_round_trips() {
    // Needs an image of at least 4096²; a flat PNG of that size is quick to write.
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(1_800_000_000_000));
    ctx.library_override = Some(dir.path().join("lib"));
    let core = Core::new(ctx).unwrap();
    let src = dir.path().join("big.png");
    let mut out = Vec::new();
    image::GrayImage::new(4096, 4096)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(&src, out).unwrap();
    let made = core
        .dispatch(
            "project_create",
            json!({"section": "maps", "paths": [src.to_string_lossy()]}),
        )
        .unwrap();
    let snap = core
        .dispatch("project_open", json!({"path": made["items"][0]["path"]}))
        .unwrap();
    let bytes: Vec<u8> = (0..4096u32 * 4096).map(|i| (i % 253) as u8).collect();
    let r = core
        .dispatch(
            "edt_mask_submit",
            json!({"project_id": snap["project_id"], "rect": {"x": 0, "y": 0, "w": 4096, "h": 4096}, "data_b64": crate::b64::encode(&bytes)}),
        )
        .unwrap();
    let read = core
        .dispatch(
            "edt_mask_read",
            json!({"project_id": snap["project_id"], "mask_id": r["mask_id"]}),
        )
        .unwrap();
    assert!(bytes_of(&read) == bytes);
}

//! #63 integration tests (export spec §13 "#63"): Export as and Save as from the canvas
//! (FR-EXP-30, 31), single files (FR-EXP-32), the shared runner (FR-EXP-33) and the save dialog's
//! default path. Background work runs inline.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use image::{GenericImageView, RgbImage, RgbaImage};
use serde_json::{Value, json};

use super::*;
use crate::context::FakeFsAttributes;
use crate::error::ErrorCode;
use crate::project::ReadOnlyReason;

impl T {
    /// A folder outside the library (a Save as target).
    fn outside(&self) -> PathBuf {
        let d = self.dir.path().join("out");
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let path = rel
            .split('/')
            .fold(self.project.clone(), |p, part| p.join(part));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn lib(&self) -> PathBuf {
        self.dir.path().join("lib")
    }

    fn export_file(
        &self,
        source: &Path,
        format: Option<&str>,
        dest: &str,
        replace: bool,
    ) -> Result<Value, crate::error::CoreError> {
        let mut input = json!({"source": source, "dest": dest, "replace": replace});
        if let Some(f) = format {
            input["format"] = json!(f);
        }
        self.core.dispatch("export_file", input)
    }

    fn project_id(&self) -> ulid::Ulid {
        self.core
            .state()
            .project
            .as_ref()
            .unwrap()
            .manifest
            .project_id
    }

    fn settings_json(&self) -> Value {
        serde_json::from_slice(
            &std::fs::read(self.dir.path().join("data").join("settings.json")).unwrap(),
        )
        .unwrap()
    }
}

/// RGBA with a transparent left half and an opaque, textured right half.
fn half_transparent(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        if x < w / 2 {
            image::Rgba([200, 10, 10, 0])
        } else {
            image::Rgba([(x * 9 % 256) as u8, (y * 7 % 256) as u8, 40, 255])
        }
    })
}

fn rgba_png(img: &RgbaImage) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn sniff(path: &Path) -> Option<pf_imaging::ImageFormat> {
    pf_imaging::sniff(&std::fs::read(path).unwrap())
}

// ------------------------------------------------------------------ FR-EXP-30 Export as

#[test]
fn fr_exp_30_format_override_commits_no_settings_and_records_last() {
    let t = setup();
    // The panel's settings: typed this session, not committed yet (R30).
    t.core
        .dispatch(
            "export_set_settings",
            json!({"settings": settings(Some(16.0), Some(8), "png")}),
        )
        .unwrap();
    let before = t.manifest_export();
    let seq = t.seq();
    let out = t
        .core
        .dispatch(
            "export_start",
            json!({"settings": settings(Some(16.0), Some(8), "png"), "replace": false, "format_override": "jpeg"}),
        )
        .unwrap();
    let done = t.last_event("export:done");
    assert_eq!(done["export_id"], out["export_id"]);
    assert_eq!(done["format"], "jpeg");
    assert_eq!(t.exports(), ["Crypt_8px.jpg"]);
    assert_eq!(
        sniff(&t.project.join("exports").join("Crypt_8px.jpg")),
        Some(pf_imaging::ImageFormat::Jpeg)
    );
    // No settings commit: the one commit is `last` (FR-EXP-27).
    assert_eq!(t.seq(), seq + 1);
    let rec = t.manifest_export();
    for k in ["grid", "target_px_per_square", "format"] {
        assert_eq!(rec[k], before[k], "{k} unchanged");
    }
    assert_eq!(rec["last"]["path"], "exports/Crypt_8px.jpg");
    assert_eq!(rec["last"]["format"], "jpeg");
    // The session settings stay the panel's, still waiting for their draft save.
    let got = t.core.dispatch("export_get_settings", json!({})).unwrap();
    assert_eq!(got["format"], "png");
    t.clock.advance(Duration::from_secs(5));
    assert!(super::super::commands::flush_settings_if_due(&t.core).unwrap());
    assert_eq!(t.manifest_export()["format"], "png");
}

#[test]
fn fr_exp_30_exists_unless_replace_read_only_and_too_large() {
    let t = setup();
    let start = |replace: bool, format: &str| {
        t.core.dispatch(
            "export_start",
            json!({"settings": settings(None, None, "png"), "replace": replace, "format_override": format}),
        )
    };
    start(false, "webp").unwrap();
    let e = start(false, "webp").unwrap_err();
    assert_eq!(e.code, ErrorCode::Exists);
    assert!(e.detail["bytes"].as_u64().unwrap() > 0);
    start(true, "webp").unwrap();
    t.core.state().project.as_mut().unwrap().read_only = Some(ReadOnlyReason::OriginalChanged);
    assert_eq!(start(true, "png").unwrap_err().code, ErrorCode::ReadOnly);
    t.core.state().project.as_mut().unwrap().read_only = None;
    {
        let mut s = t.core.state();
        let m = &mut s.project.as_mut().unwrap().manifest;
        m.original.as_mut().unwrap().width = 16384;
    }
    assert_eq!(start(true, "webp").unwrap_err().code, ErrorCode::TooLarge);
}

// ------------------------------------------------------------------ FR-EXP-31 Save as

#[test]
fn fr_exp_31_save_as_outside_the_library_writes_nothing_to_the_project() {
    let t = setup();
    let out = t.outside();
    // Another program's (or an old export's) temp file in the chosen folder.
    let foreign = out.join(".vault.webp.pf-tmp-01old");
    std::fs::write(&foreign, b"x").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&foreign)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(7200))
        .unwrap();
    let before = t.manifest_export();
    let seq = t.seq();
    let dest = out.join("vault.webp");
    t.core
        .dispatch(
            "export_start",
            json!({"settings": settings(Some(16.0), Some(8), "png"), "replace": false, "dest": dest}),
        )
        .unwrap();
    let done = t.last_event("export:done");
    assert_eq!(done["path"].as_str(), dest.to_str());
    assert_eq!(done["format"], "webp");
    let img = image::open(&dest).unwrap();
    assert_eq!(img.dimensions(), (32, 24));
    assert!(
        foreign.exists(),
        "FR-EXP-17's sweep never runs outside exports/"
    );
    let names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names.len(),
        2,
        "only our file and the foreign temp: {names:?}"
    );
    assert_eq!(t.seq(), seq, "no settings commit, no `last`");
    assert_eq!(t.manifest_export(), before);
    assert!(t.exports().is_empty());
    // The last Save as folder is app data (settings.json), never a project file.
    assert_eq!(
        t.settings_json()["export"]["last_save_dir"].as_str(),
        out.to_str()
    );
}

#[test]
fn fr_exp_31_save_as_works_on_a_read_only_project_and_replaces() {
    let t = setup();
    t.core.state().project.as_mut().unwrap().read_only = Some(ReadOnlyReason::OriginalChanged);
    let dest = t.outside().join("Crypt.PNG");
    std::fs::write(&dest, b"old").unwrap();
    let seq = t.seq();
    t.core
        .dispatch(
            "export_start",
            json!({"settings": settings(None, None, "jpeg"), "replace": false, "dest": dest}),
        )
        .unwrap();
    // The extension (any case) picks the format; the dialog already asked about replacing.
    assert_eq!(sniff(&dest), Some(pf_imaging::ImageFormat::Png));
    assert_eq!(t.decode_abs(&dest), map_image(64, 48));
    assert_eq!(t.seq(), seq);
}

#[test]
fn fr_exp_31_bad_extension_missing_folder_and_relative_paths_write_nothing() {
    let t = setup();
    let out = t.outside();
    let start = |dest: Value| {
        t.core.dispatch(
            "export_start",
            json!({"settings": settings(None, None, "png"), "replace": false, "dest": dest}),
        )
    };
    let e = start(json!(out.join("vault.gif"))).unwrap_err();
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::InvalidSettings, Some("dest"))
    );
    let e = start(json!("vault.png")).unwrap_err();
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::InvalidSettings, Some("dest"))
    );
    let e = start(json!(out.join("missing").join("vault.png"))).unwrap_err();
    assert_eq!(e.code, ErrorCode::DestUnavailable);
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
    assert!(t.events("export:done").is_empty());
}

#[test]
fn fr_exp_31_save_default_is_the_last_folder_else_the_project_exports() {
    let t = setup();
    let get = |input: Value| {
        PathBuf::from(
            t.core.dispatch("export_save_default", input).unwrap()["path"]
                .as_str()
                .unwrap(),
        )
    };
    t.core
        .dispatch(
            "export_set_settings",
            json!({"settings": settings(Some(16.0), Some(8), "webp")}),
        )
        .unwrap();
    // FR-EXP-14's name with the panel's format, in the project's exports/.
    assert_eq!(
        get(json!({})),
        t.project.join("exports").join("Crypt_8px.webp")
    );
    assert_eq!(
        get(json!({"format": "png"})),
        t.project.join("exports").join("Crypt_8px.png")
    );
    let out = t.outside();
    t.core
        .dispatch(
            "export_start",
            json!({"settings": settings(None, None, "png"), "replace": false, "dest": out.join("a.png")}),
        )
        .unwrap();
    assert_eq!(get(json!({})), out.join("Crypt_8px.webp"));
}

// ------------------------------------------------------------------ FR-EXP-32 single files

#[test]
fn fr_exp_32_same_format_is_a_byte_copy_named_after_the_file() {
    let t = setup();
    let bytes = rgba_png(&half_transparent(20, 10));
    let src = t.put("references/statue.png", &bytes);
    let seq = t.seq();
    t.export_file(&src, Some("png"), "exports", false).unwrap();
    let out = t.project.join("exports").join("Crypt_statue.png");
    assert_eq!(std::fs::read(&out).unwrap(), bytes, "byte copy");
    assert_eq!(t.last_event("export:done")["path"].as_str(), out.to_str());
    assert_eq!(t.seq(), seq, "nothing committed to the manifest");
}

#[test]
fn fr_exp_32_convert_keeps_alpha_in_png_and_webp_and_flattens_jpeg_over_grey() {
    let t = setup();
    let img = half_transparent(32, 16);
    let mut webp_src = Vec::new();
    image::DynamicImage::ImageRgba8(img.clone())
        .write_to(
            &mut std::io::Cursor::new(&mut webp_src),
            image::ImageFormat::WebP,
        )
        .unwrap();
    // A lossless WebP source with alpha → PNG keeps the alpha exactly.
    let src = t.put("parts/gem.webp", &webp_src);
    t.export_file(&src, Some("png"), "exports", false).unwrap();
    let png = image::open(t.project.join("exports").join("Crypt_gem.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(png, img);
    // PNG source → WebP keeps alpha (the alpha plane is lossless).
    let src = t.put("parts/coin.png", &rgba_png(&img));
    t.export_file(&src, Some("webp"), "exports", false).unwrap();
    let webp = image::open(t.project.join("exports").join("Crypt_coin.webp")).unwrap();
    assert!(webp.color().has_alpha());
    let webp = webp.to_rgba8();
    assert_eq!(webp.get_pixel(2, 2)[3], 0);
    assert_eq!(webp.get_pixel(30, 2)[3], 255);
    // → JPEG: the transparent half is the flatten colour #808080 (#60).
    t.export_file(&src, Some("jpeg"), "exports", false).unwrap();
    let jpeg = image::open(t.project.join("exports").join("Crypt_coin.jpg"))
        .unwrap()
        .to_rgb8();
    assert_eq!(jpeg.dimensions(), (32, 16), "own size, no resample");
    let px = jpeg.get_pixel(3, 8);
    for c in px.0 {
        assert!((i32::from(c) - 128).abs() <= 6, "{px:?} is mid grey");
    }
}

#[test]
fn fr_exp_32_candidate_names_exists_and_replace() {
    let t = setup();
    let src = t.put(
        "results/001 crack/01.png",
        &crate::test_support::rgba_png(8, 8, [1, 2, 3, 255]),
    );
    t.export_file(&src, Some("webp"), "exports", false).unwrap();
    assert_eq!(t.exports(), ["Crypt_001-01.webp"]);
    let e = t
        .export_file(&src, Some("webp"), "exports", false)
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Exists);
    assert!(e.detail["bytes"].as_u64().unwrap() > 0);
    assert!(e.detail["mtime"].is_string() || e.detail["mtime"].is_number());
    assert_eq!(e.detail["path"], "exports/Crypt_001-01.webp");
    t.export_file(&src, Some("webp"), "exports", true).unwrap();
    assert_eq!(t.events("export:done").len(), 2);
}

#[test]
fn fr_exp_32_names() {
    use super::super::settings::{ExportFormat as F, file_export_name, plain_export_name};
    assert_eq!(
        file_export_name("Vault", "results/001 crack/01.png", F::Webp),
        "Vault_001-01.webp"
    );
    assert_eq!(
        file_export_name("Vault", "results/1203 x/12.webp", F::Png),
        "Vault_1203-12.png"
    );
    assert_eq!(
        file_export_name("Vault", "references/statue.jpeg", F::Jpeg),
        "Vault_statue.jpg"
    );
    assert_eq!(
        file_export_name("Vault", "working/Vault.png", F::Png),
        "Vault_Vault.png"
    );
    assert_eq!(plain_export_name("dragon.webp", F::Png), "dragon.png");
}

#[test]
fn fr_exp_32_outside_library_not_found_and_unreadable_sources() {
    let t = setup();
    let outside = t.outside().join("x.png");
    std::fs::write(
        &outside,
        crate::test_support::rgba_png(4, 4, [0, 0, 0, 255]),
    )
    .unwrap();
    let code = |src: &Path| {
        t.export_file(src, Some("png"), "exports", false)
            .unwrap_err()
            .code
    };
    assert_eq!(code(&outside), ErrorCode::OutsideLibrary);
    let escape = t
        .project
        .join("..")
        .join("..")
        .join("..")
        .join("out")
        .join("x.png");
    assert_eq!(code(&escape), ErrorCode::OutsideLibrary);
    assert_eq!(
        code(&t.project.join("references").join("gone.png")),
        ErrorCode::NotFound
    );
    let text = t.put("references/notes.txt", b"not an image at all");
    assert_eq!(code(&text), ErrorCode::Unsupported);
    let mut png = crate::test_support::rgba_png(64, 64, [9, 9, 9, 255]);
    png.truncate(png.len() / 2);
    let broken = t.put("references/broken.png", &png);
    assert_eq!(code(&broken), ErrorCode::Corrupt);
    assert!(t.exports().is_empty());
}

#[test]
fn fr_exp_32_read_only_refuses_exports_but_allows_save_as() {
    let t = setup();
    let src = t.put("references/statue.png", &rgba_png(&half_transparent(8, 8)));
    t.core.state().project.as_mut().unwrap().read_only = Some(ReadOnlyReason::NewerVersion);
    let e = t
        .export_file(&src, Some("png"), "exports", false)
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::ReadOnly);
    let dest = t.outside().join("statue.webp");
    t.export_file(&src, None, dest.to_str().unwrap(), false)
        .unwrap();
    assert_eq!(sniff(&dest), Some(pf_imaging::ImageFormat::Webp));
}

#[test]
fn fr_exp_32_pack_image_has_save_as_only() {
    let t = setup();
    let pack = t.lib().join("References").join("Dragons");
    std::fs::create_dir_all(&pack).unwrap();
    let src = pack.join("red.png");
    std::fs::write(&src, crate::test_support::rgba_png(6, 6, [200, 0, 0, 255])).unwrap();
    let e = t
        .export_file(&src, Some("png"), "exports", false)
        .unwrap_err();
    assert_eq!(
        (e.code, e.field.as_deref()),
        (ErrorCode::InvalidSettings, Some("dest"))
    );
    let dest = t.outside().join("red.jpg");
    t.export_file(&src, Some("jpeg"), dest.to_str().unwrap(), false)
        .unwrap();
    assert_eq!(sniff(&dest), Some(pf_imaging::ImageFormat::Jpeg));
    // Mismatched format and extension.
    let e = t
        .export_file(&src, Some("png"), dest.to_str().unwrap(), false)
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidSettings);
    // The default name is the image's own stem.
    let got = t
        .core
        .dispatch("export_save_default", json!({"source": src}))
        .unwrap();
    assert_eq!(
        Path::new(got["path"].as_str().unwrap()).file_name(),
        Some(std::ffi::OsStr::new("red.png"))
    );
}

#[test]
fn fr_exp_32_fr_prj_17_a_cloud_only_source_is_hydrated_and_offline_refused() {
    let fs = Arc::new(FakeFsAttributes::default());
    let t = setup_ctx(&map_image(64, 48), |ctx| ctx.fs = Box::new(fs.clone()));
    let src = t.put("references/statue.png", &rgba_png(&half_transparent(8, 8)));
    let real = crate::library::plain_path(std::fs::canonicalize(&src).unwrap());
    fs.cloud_only.lock().unwrap().insert(real.clone());
    fs.offline.store(true, Ordering::SeqCst);
    let e = t
        .export_file(&src, Some("png"), "exports", false)
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotAvailableOffline);
    assert!(t.exports().is_empty());
    fs.offline.store(false, Ordering::SeqCst);
    let calls = fs.hydrate_calls.load(Ordering::SeqCst);
    t.export_file(&src, Some("png"), "exports", false).unwrap();
    assert_eq!(fs.hydrate_calls.load(Ordering::SeqCst), calls + 1);
    assert!(!fs.cloud_only.lock().unwrap().contains(&real));
}

#[test]
fn fr_exp_32_a_file_of_a_closed_project_is_no_project() {
    let t = setup();
    let src = t.put("references/statue.png", &rgba_png(&half_transparent(8, 8)));
    t.core.dispatch("project_close", json!({})).unwrap();
    let e = t
        .export_file(&src, Some("png"), "exports", false)
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NoProject);
}

// ------------------------------------------------------------------ FR-EXP-33 the runner

#[test]
fn fr_exp_33_one_export_per_project_and_one_pack_save_as_at_a_time() {
    let t = setup();
    let src = t.put("references/statue.png", &rgba_png(&half_transparent(8, 8)));
    let pack = t.lib().join("References").join("Dragons");
    std::fs::create_dir_all(&pack).unwrap();
    let pack_img = pack.join("red.png");
    std::fs::write(
        &pack_img,
        crate::test_support::rgba_png(6, 6, [200, 0, 0, 255]),
    )
    .unwrap();
    let out = t.outside();
    let id = t.project_id();
    t.core.state().exp.running.push(super::super::Running {
        id: t.core.ctx.new_id(),
        scope: super::super::Scope::Project(id),
        cancel: Arc::default(),
    });
    let busy = |r: Result<Value, crate::error::CoreError>| r.unwrap_err().code == ErrorCode::Busy;
    assert!(busy(t.export_file(&src, Some("png"), "exports", false)));
    assert!(busy(t.core.dispatch(
        "export_start",
        json!({"settings": settings(None, None, "png"), "replace": false, "dest": out.join("a.png")}),
    )));
    assert!(busy(t.core.dispatch(
        "export_start",
        json!({"settings": settings(None, None, "png"), "replace": false, "format_override": "webp"}),
    )));
    // A pack image's Save as has its own app-wide slot.
    t.export_file(
        &pack_img,
        None,
        out.join("red.png").to_str().unwrap(),
        false,
    )
    .unwrap();
    t.core.state().exp.running.clear();
    t.core.state().exp.running.push(super::super::Running {
        id: t.core.ctx.new_id(),
        scope: super::super::Scope::Pack,
        cancel: Arc::default(),
    });
    assert!(busy(t.export_file(
        &pack_img,
        None,
        out.join("red2.png").to_str().unwrap(),
        false
    )));
    t.export_file(&src, Some("png"), "exports", false).unwrap();
}

#[test]
fn fr_exp_33_close_cancels_the_projects_save_as_but_not_a_pack_one() {
    let t = setup();
    let id = t.project_id();
    let mine = Arc::new(AtomicBool::new(false));
    let pack = Arc::new(AtomicBool::new(false));
    for (scope, cancel) in [
        (super::super::Scope::Project(id), &mine),
        (super::super::Scope::Pack, &pack),
    ] {
        t.core.state().exp.running.push(super::super::Running {
            id: t.core.ctx.new_id(),
            scope,
            cancel: cancel.clone(),
        });
    }
    t.core.dispatch("project_close", json!({})).unwrap();
    assert!(mine.load(Ordering::SeqCst));
    assert!(!pack.load(Ordering::SeqCst));
}

#[test]
fn fr_exp_33_menu_exports_emit_the_export_events_and_cancel() {
    let t = setup();
    let src = t.put(
        "references/statue.png",
        &rgba_png(&half_transparent(64, 64)),
    );
    t.seams(ExportSeams {
        cancel_after_progress: Some(1),
        ..ExportSeams::default()
    });
    let out = t.export_file(&src, Some("webp"), "exports", false).unwrap();
    assert_eq!(
        t.last_event("export:canceled")["export_id"],
        out["export_id"]
    );
    assert!(t.exports().is_empty(), "no temp, no file");
    t.seams(ExportSeams::default());
    let out = t.export_file(&src, Some("jpeg"), "exports", false).unwrap();
    assert!(!t.events("export:progress").is_empty());
    let done = t.last_event("export:done");
    assert_eq!(done["export_id"], out["export_id"]);
    assert_eq!(
        (done["width"].as_u64(), done["height"].as_u64()),
        (Some(64), Some(64))
    );
    assert!(t.core.state().exp.running.is_empty());
}

impl T {
    fn decode_abs(&self, path: &Path) -> RgbImage {
        image::open(path).unwrap().to_rgb8()
    }
}

#[test]
fn fr_exp_32_the_source_type_is_sniffed_not_taken_from_its_extension() {
    let t = setup();
    // JPEG bytes in a file named `.png`.
    let jpeg = crate::test_support::jpeg_bytes(16, 12);
    let src = t.put("references/photo.png", &jpeg);
    t.export_file(&src, Some("webp"), "exports", false).unwrap();
    assert_eq!(t.last_event("export:done")["width"], 16);
    let webp = t.project.join("exports").join("Crypt_photo.webp");
    assert_eq!(sniff(&webp), Some(pf_imaging::ImageFormat::Webp));
    // The same type as its content: a byte copy.
    t.export_file(&src, Some("jpeg"), "exports", false).unwrap();
    assert_eq!(
        std::fs::read(t.project.join("exports").join("Crypt_photo.jpg")).unwrap(),
        jpeg
    );
}

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::*;
use crate::context::{Context, FaultAction, RecordedEvents};
use crate::time::Timestamp;

struct Env {
    dir: tempfile::TempDir,
    core: Core,
    events: Arc<RecordedEvents>,
}

fn env_in(dir: tempfile::TempDir, faults: Faults) -> Env {
    let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(1_800_000_000_000));
    ctx.library_override = Some(dir.path().join("lib"));
    ctx.faults = faults;
    let events = Arc::new(RecordedEvents::default());
    ctx.events = events.clone();
    let core = Core::new(ctx).unwrap();
    Env { dir, core, events }
}

fn env() -> Env {
    env_in(tempfile::tempdir().unwrap(), Faults::none())
}

/// A synthetic, public-safe pattern: gradients plus a 70 px grid.
fn pattern(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let grid = if x % 70 == 0 || y % 70 == 0 { 255 } else { 40 };
            v.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, grid]);
        }
    }
    v
}

fn png_bytes(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(rgb).unwrap();
    wr.finish().unwrap();
    out
}

/// Creates a project from a generated PNG and opens it; returns the project snapshot.
fn open_png(e: &Env, name: &str, w: u32, h: u32) -> Value {
    let src = e.dir.path().join(name);
    if !src.exists() {
        std::fs::write(&src, png_bytes(w, h, &pattern(w, h))).unwrap();
    }
    let out = e
        .core
        .dispatch(
            "project_create",
            json!({"section": "maps", "paths": [src.to_string_lossy()]}),
        )
        .unwrap();
    let path = out["items"][0]["path"].as_str().unwrap().to_owned();
    e.core
        .dispatch("project_open", json!({"path": path}))
        .unwrap()
}

fn tiles_open(e: &Env, snap: &Value, rebuild: bool) -> Value {
    e.core
        .dispatch(
            "edt_tiles_open",
            json!({"project_id": snap["project_id"], "image_id": snap["layers"][0]["id"], "rebuild": rebuild}),
        )
        .unwrap()
}

fn wait_built(e: &Env, key: &str) {
    let t = Instant::now();
    while e.core.served().is_building(key) {
        assert!(
            t.elapsed() < Duration::from_secs(60),
            "build never finished"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn scheme_path(url: &str) -> &str {
    let i = url.find("/t/").or_else(|| url.find("/i/")).unwrap();
    &url[i..]
}

fn tile_url(desc: &Value, level: u64, col: u64, row: u64) -> String {
    let ext = desc["levels"][level as usize]["ext"].as_str().unwrap();
    format!(
        "{}/{level}/{col}_{row}.{ext}",
        desc["base_url"].as_str().unwrap()
    )
}

fn progress_levels(e: &Env, key: &str) -> Vec<u64> {
    e.events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(n, p)| n == "edt:tiles-progress" && p["key"] == key)
        .map(|(_, p)| p["level"].as_u64().unwrap())
        .collect()
}

#[test]
fn fr_edt_10_key_derivation_changes_with_every_input() {
    let id = Ulid::from_parts(1, 2);
    let k = cache_key(id, 100, 5, KeyKind::Pyramid);
    let (head, hex) = k.rsplit_once('-').unwrap();
    assert_eq!(head, id.to_string());
    assert!(hex.len() == 16 && hex.bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(k.len() <= 80);
    assert_eq!(k, cache_key(id, 100, 5, KeyKind::Pyramid), "deterministic");
    for other in [
        cache_key(Ulid::from_parts(1, 3), 100, 5, KeyKind::Pyramid),
        cache_key(id, 101, 5, KeyKind::Pyramid),
        cache_key(id, 100, 6, KeyKind::Pyramid),
        cache_key(id, 100, 5, KeyKind::Image(None)),
        cache_key(id, 100, 5, KeyKind::Image(Some(256))),
    ] {
        assert_ne!(k, other);
    }
    assert_ne!(
        cache_key(id, 100, 5, KeyKind::Image(Some(256))),
        cache_key(id, 100, 5, KeyKind::Image(Some(128)))
    );
    // The builder version is part of the hash input.
    assert_ne!(
        fnv1a64(&[b"a", &1u32.to_le_bytes()]),
        fnv1a64(&[b"a", &2u32.to_le_bytes()])
    );
}

#[test]
fn fr_edt_07_fr_edt_09_builds_smallest_first_and_serves_every_tile() {
    let e = env();
    let (w, h) = (1300u32, 900u32);
    let snap = open_png(&e, "probe.png", w, h);
    let desc = tiles_open(&e, &snap, false);
    let key = desc["key"].as_str().unwrap().to_owned();
    assert_eq!(
        (desc["width"].as_u64(), desc["height"].as_u64()),
        (Some(1300), Some(900))
    );
    assert_eq!(desc["tile_size"], 512);
    assert_eq!(
        desc["base_url"],
        format!("{}/t/{key}", e.core.ctx.tiles_base_url)
    );
    let levels = desc["levels"].as_array().unwrap();
    assert_eq!(levels.len(), 3);
    assert_eq!(levels[0]["ext"], "webp");
    assert_eq!(levels[2]["scale"], 0.25);
    assert_eq!(
        (levels[0]["cols"].as_u64(), levels[0]["rows"].as_u64()),
        (Some(3), Some(2))
    );

    wait_built(&e, &key);
    assert_eq!(progress_levels(&e, &key), [2, 1, 0], "smallest first");
    assert!(e.events.names().contains(&"edt:tiles-ready".to_string()));
    let dir = cache_root(&e.core.ctx.data_dir).join(&key);
    let file: PyramidFile =
        serde_json::from_slice(&std::fs::read(dir.join(PYRAMID_FILE)).unwrap()).unwrap();
    assert!(file.levels.iter().all(|l| l.complete));

    let src = pattern(w, h);
    for (l, lv) in levels.iter().enumerate() {
        for row in 0..lv["rows"].as_u64().unwrap() {
            for col in 0..lv["cols"].as_u64().unwrap() {
                let url = tile_url(&desc, l as u64, col, row);
                let res = e.core.serve_scheme(scheme_path(&url));
                assert_eq!(res.status, 200, "{url}");
                assert!(res.cache_immutable);
                assert_eq!(
                    res.content_type,
                    Some(if l == 0 { "image/webp" } else { "image/jpeg" })
                );
                if l == 0 {
                    // Level 0 decodes exactly equal to the source.
                    let t = image::load_from_memory(&res.body).unwrap().into_rgb8();
                    for (x, y, p) in t.enumerate_pixels() {
                        let i = (((row as u32 * 512 + y) * w + col as u32 * 512 + x) * 3) as usize;
                        assert_eq!(p.0, [src[i], src[i + 1], src[i + 2]]);
                    }
                }
            }
        }
    }
    let base = scheme_path(desc["base_url"].as_str().unwrap()).to_owned();
    for (path, status) in [
        (format!("{base}/0/3_0.webp"), 404), // out of range
        (format!("{base}/0/0_0.jpg"), 404),  // wrong ext for the level
        (format!("{base}/3/0_0.jpg"), 404),  // no such level
        ("/t/unknown/0/0_0.webp".to_owned(), 404),
        (format!("{base}/0/../0_0.webp"), 400),
    ] {
        assert_eq!(e.core.serve_scheme(&path).status, status, "{path}");
    }

    // Warm open: nothing to build, no new events.
    let again = tiles_open(&e, &snap, false);
    assert_eq!(again["key"], desc["key"]);
    assert!(
        again["levels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l["complete"] == true)
    );
    assert!(!e.core.served().is_building(&key));
    assert_eq!(progress_levels(&e, &key).len(), 3);

    // Keys are unregistered on close.
    let first = tile_url(&desc, 2, 0, 0);
    e.core.dispatch("project_close", json!({})).unwrap();
    assert_eq!(e.core.serve_scheme(scheme_path(&first)).status, 404);
}

#[test]
fn fr_edt_10_aborted_build_is_never_served_and_rebuilt_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let faults = Faults::parse("after_level:1", FaultAction::Error).unwrap();
    let e = env_in(dir, faults);
    let snap = open_png(&e, "crypt.png", 1300, 900);
    // The original's image id is stable from creation (#48 (3)), so the reopen sees the same key.
    let desc = tiles_open(&e, &snap, false);
    let key = desc["key"].as_str().unwrap().to_owned();
    wait_built(&e, &key);
    assert_eq!(progress_levels(&e, &key), [2, 1]);
    assert!(e.events.names().contains(&"edt:tiles-failed".to_string()));
    // Levels 2 and 1 are served; level 0 is not built: 425.
    assert_eq!(
        e.core
            .serve_scheme(scheme_path(&tile_url(&desc, 1, 0, 0)))
            .status,
        200
    );
    assert_eq!(
        e.core
            .serve_scheme(scheme_path(&tile_url(&desc, 0, 0, 0)))
            .status,
        425
    );

    // A fresh run (no fault) over the same data dir rebuilds only the missing level.
    let Env { dir, core, .. } = e;
    drop(core);
    let e = env_in(dir, Faults::none());
    let snap = open_png(&e, "crypt.png", 1300, 900);
    let again = tiles_open(&e, &snap, false);
    assert_eq!(again["key"], key);
    let complete: Vec<bool> = again["levels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["complete"].as_bool().unwrap())
        .collect();
    assert_eq!(complete, [false, true, true]);
    wait_built(&e, &key);
    assert_eq!(progress_levels(&e, &key), [0]);
    assert_eq!(
        e.core
            .serve_scheme(scheme_path(&tile_url(&again, 0, 2, 1)))
            .status,
        200
    );

    // Rebuild deletes and rebuilds everything.
    tiles_open(&e, &snap, true);
    wait_built(&e, &key);
    assert_eq!(progress_levels(&e, &key), [0, 2, 1, 0]);
}

#[test]
fn fr_edt_12_an_empty_or_corrupt_tile_fails_and_rebuild_replaces_it() {
    // R105: tiles aren't fsynced; after a power cut a complete level may hold an empty or
    // corrupt tile. It must fail in the UI (fetch or decode → FR-EDT-12 retries, then the
    // Rebuild badge), and Rebuild must write it again.
    let e = env();
    let snap = open_png(&e, "crypt.png", 600, 400);
    let desc = tiles_open(&e, &snap, false);
    let key = desc["key"].as_str().unwrap().to_owned();
    wait_built(&e, &key);
    let dir = cache_root(&e.core.ctx.data_dir).join(&key);
    let specs = pyramid::levels(600, 400);
    let empty = pyramid::tile_path(&dir, &specs[0], 1, 0);
    let corrupt = pyramid::tile_path(&dir, &specs[1], 0, 0);
    std::fs::write(&empty, b"").unwrap();
    std::fs::write(&corrupt, b"\xff\xd8 not a jpeg").unwrap();

    let served = |level, col| {
        e.core
            .serve_scheme(scheme_path(&tile_url(&desc, level, col, 0)))
    };
    assert_eq!(served(0, 1).status, 404, "an empty tile is a failed fetch");
    let bad = served(1, 0);
    assert_eq!(bad.status, 200);
    assert!(
        image::load_from_memory(&bad.body).is_err(),
        "a corrupt tile fails the decode"
    );

    let again = tiles_open(&e, &snap, true);
    wait_built(&e, &key);
    for (level, col) in [(0, 1), (1, 0)] {
        let r = e
            .core
            .serve_scheme(scheme_path(&tile_url(&again, level, col, 0)));
        assert_eq!(r.status, 200);
        assert!(
            image::load_from_memory(&r.body).is_ok(),
            "rebuilt tile decodes"
        );
    }
}

#[test]
fn fr_edt_10_mtime_change_gives_a_new_key() {
    let e = env();
    let snap = open_png(&e, "m.png", 600, 400);
    let a = tiles_open(&e, &snap, false);
    wait_built(&e, a["key"].as_str().unwrap());
    let path = snap["image_path"].as_str().unwrap();
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000))
        .unwrap();
    drop(f);
    let b = tiles_open(&e, &snap, false);
    assert_ne!(a["key"], b["key"]);
    wait_built(&e, b["key"].as_str().unwrap());
}

#[test]
fn fr_edt_07_unknown_project_or_image_is_rejected() {
    let e = env();
    let snap = open_png(&e, "x.png", 64, 64);
    let err = e
        .core
        .dispatch(
            "edt_tiles_open",
            json!({"project_id": snap["project_id"], "image_id": Ulid::from_parts(9, 9)}),
        )
        .unwrap_err();
    assert_eq!(
        (err.code, err.field.as_deref()),
        (ErrorCode::UnknownId, Some("image_id"))
    );
    let err = e
        .core
        .dispatch(
            "edt_tiles_open",
            json!({"project_id": Ulid::from_parts(9, 9), "image_id": snap["layers"][0]["id"]}),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownId);
    e.core.dispatch("project_close", json!({})).unwrap();
    let err = e
        .core
        .dispatch(
            "edt_tiles_open",
            json!({"project_id": snap["project_id"], "image_id": snap["layers"][0]["id"]}),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NoProject);
}

#[test]
fn fr_edt_11_eviction_drops_least_recently_opened_unused_entries() {
    let root = tempfile::tempdir().unwrap();
    for (key, opened, size) in [
        ("a", 10, 400),
        ("b", 30, 400),
        ("c", 20, 400),
        ("d", 5, 400),
    ] {
        let d = root.path().join(key);
        std::fs::create_dir_all(d.join("0")).unwrap();
        std::fs::write(d.join("0/0_0.webp"), vec![0u8; size]).unwrap();
        std::fs::write(d.join(OPENED_FILE), opened.to_string()).unwrap();
    }
    // `d` is the oldest but open, so it stays; then `a` (10) and `c` (20) go.
    let in_use: HashSet<String> = ["d".to_string()].into();
    let mut removed = evict(root.path(), 900, &in_use);
    removed.sort();
    assert_eq!(removed, ["a", "c"]);
    assert!(root.path().join("b").exists() && root.path().join("d").exists());
    assert!(evict(root.path(), 900, &in_use).is_empty(), "under the cap");
    assert_eq!(CACHE_CAP_BYTES, 2 << 30);
}

#[test]
fn fr_edt_11_old_builder_caches_are_evicted_whole() {
    let data = tempfile::tempdir().unwrap();
    let root = cache_root(data.path());
    let tiles = root.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(root.join("k")).unwrap();
    std::fs::create_dir_all(tiles.join("v1/old-key/0")).unwrap();
    std::fs::write(tiles.join("v1/old-key/0/0_0.webp"), [0u8; 64]).unwrap();
    std::fs::create_dir_all(tiles.join("notes")).unwrap();
    evict(&root, CACHE_CAP_BYTES, &HashSet::new());
    assert!(!tiles.join("v1").exists(), "v1 is never served: removed");
    assert!(
        root.join("k").exists(),
        "the current cache stays under the cap"
    );
    assert!(tiles.join("notes").exists(), "only v<N> folders");
    // A root that isn't a cache root never touches its siblings.
    let other = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(other.path().join("v1")).unwrap();
    std::fs::create_dir_all(other.path().join("v2")).unwrap();
    evict(&other.path().join("v2"), 0, &HashSet::new());
    assert!(other.path().join("v1").exists());
}

#[test]
fn fr_edt_31_transient_images_cap_at_four_and_unregister_on_close() {
    let e = env();
    let snap = open_png(&e, "t.png", 64, 64);
    let pid: Ulid = snap["project_id"].as_str().unwrap().parse().unwrap();
    let png = png_bytes(8, 4, &pattern(8, 4));
    let outs: Vec<ImageOpenOutput> = (0..5)
        .map(|_| register_transient_image(&e.core, pid, png.clone()).unwrap())
        .collect();
    assert_eq!((outs[0].width, outs[0].height), (8, 4));
    assert!(outs[0].url.ends_with(&format!("/i/{}.png", outs[0].key)));
    let status = |o: &ImageOpenOutput| e.core.serve_scheme(scheme_path(&o.url)).status;
    assert_eq!(status(&outs[0]), 404, "the fifth unregisters the oldest");
    for o in &outs[1..] {
        let res = e.core.serve_scheme(scheme_path(&o.url));
        assert_eq!((res.status, res.content_type), (200, Some("image/png")));
        assert_eq!(res.body, png);
    }
    assert_eq!(
        register_transient_image(&e.core, Ulid::from_parts(1, 1), png.clone())
            .unwrap_err()
            .code,
        ErrorCode::UnknownId
    );
    assert_eq!(
        register_transient_image(&e.core, pid, b"nope".to_vec())
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    e.core.dispatch("project_close", json!({})).unwrap();
    assert!(outs.iter().all(|o| status(o) == 404));
}

#[test]
fn fr_edt_32_max_side_copy_and_use_tiles() {
    let e = env();
    let snap = open_png(&e, "big.png", 1300, 900);
    let open = |max_side: Value| {
        e.core.dispatch(
            "edt_image_open",
            json!({"project_id": snap["project_id"], "image_id": snap["layers"][0]["id"], "max_side": max_side}),
        )
    };
    let whole = open(Value::Null).unwrap();
    assert_eq!(
        (whole["width"].as_u64(), whole["height"].as_u64()),
        (Some(1300), Some(900))
    );
    let thumb = open(json!(256)).unwrap();
    assert_eq!(
        (thumb["width"].as_u64(), thumb["height"].as_u64()),
        (Some(256), Some(177))
    );
    assert_ne!(thumb["key"], whole["key"]);
    let res = e
        .core
        .serve_scheme(scheme_path(thumb["url"].as_str().unwrap()));
    assert_eq!((res.status, res.content_type), (200, Some("image/png")));
    let img = image::load_from_memory(&res.body).unwrap();
    assert_eq!((img.width(), img.height()), (256, 177));
    // Cached on disk under its own key; a second open reuses it.
    let copy = cache_root(&e.core.ctx.data_dir)
        .join(thumb["key"].as_str().unwrap())
        .join(COPY_FILE);
    assert!(copy.is_file());
    assert_eq!(open(json!(256)).unwrap()["key"], thumb["key"]);
    assert_eq!(
        open(json!(0)).unwrap_err().field.as_deref(),
        Some("max_side")
    );
    assert_eq!(open(json!(5000)).unwrap_err().code, ErrorCode::InvalidInput);

    // Over 4096 per side: whole images are refused, copies work.
    let wide = open_png(&e, "wide.png", 4100, 8);
    let err = e
        .core
        .dispatch(
            "edt_image_open",
            json!({"project_id": wide["project_id"], "image_id": wide["layers"][0]["id"]}),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::UseTiles);
    assert_eq!(err.code.as_str(), "use_tiles");
    let small = e
        .core
        .dispatch(
            "edt_image_open",
            json!({"project_id": wide["project_id"], "image_id": wide["layers"][0]["id"], "max_side": 256}),
        )
        .unwrap();
    assert_eq!(
        (small["width"].as_u64(), small["height"].as_u64()),
        (Some(256), Some(1))
    );
}

#[test]
fn bl_16_library_keys_serve_a_512_px_copy_made_on_first_view() {
    let e = env();
    let big = e.dir.path().join("ref-big.png");
    std::fs::write(&big, png_bytes(1300, 900, &pattern(1300, 900))).unwrap();
    let url = register_library_image(&e.core, &big).unwrap();
    assert!(url.contains("/i/l-") && url.ends_with(".png"), "{url}");
    let key = url.rsplit('/').next().unwrap().trim_end_matches(".png");
    let copy = cache_root(&e.core.ctx.data_dir).join(key).join(COPY_FILE);
    assert!(!copy.exists(), "registration reads metadata only");

    let res = e.core.serve_scheme(scheme_path(&url));
    assert_eq!((res.status, res.content_type), (200, Some("image/png")));
    let img = image::load_from_memory(&res.body).unwrap();
    assert_eq!((img.width(), img.height()), (512, 354));
    assert!(copy.is_file(), "cached for the next view");
    assert_eq!(e.core.serve_scheme(scheme_path(&url)).body, res.body);

    // Another limit (REF pack thumbnails) is another key.
    let small_url = register_library_image_sized(&e.core, &big, 256).unwrap();
    assert_ne!(small_url, url);
    let img = image::load_from_memory(&e.core.serve_scheme(scheme_path(&small_url)).body).unwrap();
    assert_eq!(img.width(), 256);

    // A file that already fits is served as it is, with its own type.
    let thumb = e.dir.path().join("thumb.jpg");
    let mut jpg = Vec::new();
    image::RgbImage::from_pixel(300, 200, image::Rgb([90, 60, 30]))
        .write_to(
            &mut std::io::Cursor::new(&mut jpg),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    std::fs::write(&thumb, &jpg).unwrap();
    let t = e.core.serve_scheme(scheme_path(
        &register_library_image(&e.core, &thumb).unwrap(),
    ));
    assert_eq!((t.status, t.content_type), (200, Some("image/jpeg")));
    assert_eq!(t.body, jpg);

    // The key follows the file; a vanished or broken file is a 404, never a crash.
    std::fs::write(&big, png_bytes(600, 600, &pattern(600, 600))).unwrap();
    assert_ne!(register_library_image(&e.core, &big).unwrap(), url);
    let gone = e.dir.path().join("gone.png");
    std::fs::write(&gone, png_bytes(700, 10, &pattern(700, 10))).unwrap();
    let gone_url = register_library_image(&e.core, &gone).unwrap();
    std::fs::write(&gone, b"not an image").unwrap();
    assert_eq!(e.core.serve_scheme(scheme_path(&gone_url)).status, 404);
    std::fs::remove_file(&gone).unwrap();
    assert_eq!(e.core.serve_scheme(scheme_path(&gone_url)).status, 404);
    assert!(register_library_image(&e.core, &gone).is_none());
    assert!(register_library_image(&e.core, &e.dir.path().join("x.gif")).is_none());
}

#[test]
fn fr_edt_10_d4_reopening_an_unsaved_project_reuses_the_pyramid() {
    let e = env();
    let snap = open_png(&e, "keep.png", 700, 500);
    assert_eq!(snap["original_image_id"], snap["layers"][0]["id"]);
    let first = tiles_open(&e, &snap, false);
    wait_built(&e, first["key"].as_str().unwrap());
    let path = snap["project_dir"].as_str().unwrap().to_owned();
    e.core.dispatch("project_close", json!({})).unwrap();
    let again = e
        .core
        .dispatch("project_open", json!({"path": path}))
        .unwrap();
    assert_eq!(again["original_image_id"], snap["original_image_id"]);
    let second = e
        .core
        .dispatch(
            "edt_tiles_open",
            json!({"project_id": again["project_id"], "image_id": again["original_image_id"]}),
        )
        .unwrap();
    assert_eq!(second["key"], first["key"]);
    assert!(
        second["levels"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l["complete"] == true)
    );
    let entries = std::fs::read_dir(cache_root(&e.core.ctx.data_dir))
        .unwrap()
        .count();
    assert_eq!(entries, 1, "no orphaned pyramid");
}

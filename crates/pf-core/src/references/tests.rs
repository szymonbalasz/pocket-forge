//! REF unit tests (§13), named after the requirements. Generated images only (KQ-22).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use pf_provider::ReferenceRole;
use serde_json::json;

use super::packs::{self, AddResult, ImageState, PackBadge};
use super::trash::{self, TrashKind};
use super::*;
use crate::context::{FaultAction, Faults, ManualClock};
use crate::create::SkipReason;
use crate::test_support::{Env, FsAttributes, T0, env, jpeg_bytes, png_bytes};

fn refs(e: &Env) -> PathBuf {
    e.lib().join(REFERENCES_DIR)
}

/// A pack folder made in Explorer: images, no `pack.json`.
fn explorer_pack(e: &Env, name: &str, files: &[(&str, Vec<u8>)]) -> PathBuf {
    let dir = refs(e).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    for (f, b) in files {
        std::fs::write(dir.join(f), b).unwrap();
    }
    dir
}

fn swatch(shade: u8) -> Vec<u8> {
    png_bytes(32, 24, shade)
}

fn create(e: &Env, name: &str, files: &[(&str, Vec<u8>)]) -> packs::CreateOutput {
    let paths: Vec<PathBuf> = files.iter().map(|(f, b)| e.outside(f, b)).collect();
    packs::create(&e.ctx, &e.settings, name, &paths, false).unwrap()
}

fn loc(e: &Env, id: ulid::Ulid) -> PackLoc {
    find_pack(&e.ctx, &e.settings, id).unwrap()
}

fn names(v: &packs::PackView) -> Vec<&str> {
    v.images.iter().map(|i| i.file.as_str()).collect()
}

#[test]
fn fr_prj_51_setting_a_root_creates_references() {
    let e = env();
    assert!(refs(&e).is_dir(), "the override root gets References/");
    let other = e.dir.path().join("other");
    std::fs::create_dir(&other).unwrap();
    crate::library::ensure_sections(&other).unwrap();
    assert!(other.join(REFERENCES_DIR).is_dir());
    let cards =
        crate::library::list_projects(&e.ctx, &e.settings, crate::manifest::Kind::Map).unwrap();
    assert!(cards.is_empty(), "References/ is not a section");
}

#[test]
fn fr_ref_01_images_are_supported_top_level_files() {
    let e = env();
    let dir = explorer_pack(
        &e,
        "Swords",
        &[
            ("b.png", swatch(1)),
            ("A.jpg", jpeg_bytes(16, 16)),
            ("notes.txt", b"hi".to_vec()),
            ("broken.png", b"\x89PNG\r\n\x1a\nxx".to_vec()),
        ],
    );
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/c.png"), swatch(2)).unwrap();
    let cards = packs::list_packs(&e.ctx, &e.settings);
    assert_eq!(cards[0].image_count, 3);
    let v = packs::view(&e.ctx, &loc(&e, cards[0].pack_id.unwrap()));
    assert_eq!(names(&v), ["A.jpg", "b.png", "broken.png"]);
    assert_eq!(v.images[2].state, ImageState::Corrupt);
    assert_eq!(v.images[1].state, ImageState::Ok);
    assert_eq!((v.images[1].w, v.images[1].h), (Some(32), Some(24)));
    assert!(
        dir.join("sub/c.png").exists() && dir.join("notes.txt").exists(),
        "never swept"
    );
}

#[test]
fn fr_ref_02_fr_ref_07_create_names() {
    let e = env();
    let out = create(&e, "Swords", &[("s1.png", swatch(1))]);
    assert_eq!(out.pack.name, "Swords");
    assert!(refs(&e).join("Swords/pack.json").is_file());
    let err = packs::create(&e.ctx, &e.settings, "swords", &[], false).unwrap_err();
    assert_eq!(err.code, ErrorCode::NameTaken, "case-insensitive");
    let err = packs::create(&e.ctx, &e.settings, "a:b", &[], false).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidName);
    let dropped = packs::create(&e.ctx, &e.settings, "Swords", &[], true).unwrap();
    assert_eq!(
        dropped.pack.name, "Swords (2)",
        "a dropped folder gets a suffix"
    );
    let leftovers: Vec<_> = std::fs::read_dir(refs(&e))
        .unwrap()
        .flatten()
        .filter(|d| {
            d.file_name()
                .to_string_lossy()
                .contains(crate::fsx::TEMP_MARKER)
        })
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn fr_ref_07_dropped_folder_copies_images_flat() {
    let e = env();
    let src = e.dir.path().join("outside/Crypt moods");
    std::fs::create_dir_all(src.join("deep")).unwrap();
    std::fs::write(src.join("a.png"), swatch(1)).unwrap();
    std::fs::write(src.join("deep/a.png"), swatch(2)).unwrap();
    std::fs::write(src.join("readme.txt"), b"x").unwrap();
    let out = packs::create(
        &e.ctx,
        &e.settings,
        "Crypt moods",
        std::slice::from_ref(&src),
        true,
    )
    .unwrap();
    assert_eq!(names(&out.pack), ["a (2).png", "a.png"]);
    assert!(src.join("deep/a.png").exists(), "sources untouched");
}

#[test]
fn fr_ref_07_interrupted_create_leaves_no_pack() {
    let mut e = env();
    let src = e.outside("s.png", &swatch(1));
    e.ctx.faults = Faults::parse("after_temp:pack.json", FaultAction::Error).unwrap();
    assert!(packs::create(&e.ctx, &e.settings, "Swords", &[src], false).is_err());
    assert!(!refs(&e).join("Swords").exists());
    assert_eq!(
        std::fs::read_dir(refs(&e)).unwrap().count(),
        0,
        "temp removed"
    );
    // A temp folder another run left behind is swept at startup when it is old.
    std::fs::create_dir(refs(&e).join(".Old.pf-tmp-01")).unwrap();
    super::sweep_startup(&e.ctx, &e.settings);
    assert!(
        refs(&e).join(".Old.pf-tmp-01").exists(),
        "fresh: may be in progress"
    );
}

#[test]
fn fr_ref_03_pack_json_shape_and_unknown_fields() {
    let e = env();
    let out = create(&e, "Swords", &[("s1.png", swatch(1))]);
    let file = refs(&e).join("Swords/pack.json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["pack_id"], out.pack.pack_id.to_string());
    assert!(v.get("name").is_none(), "OQ-REF-01 A: no name field");
    v["future"] = json!({"x": 1});
    v["images"]["gone.png"] = json!({"role": "style", "tint": 3});
    std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
    let l = loc(&e, out.pack.pack_id);
    packs::update(&e.ctx, &l, Some("hello"), &BTreeMap::new()).unwrap();
    let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(after["future"], json!({"x": 1}));
    assert_eq!(
        after["images"]["gone.png"]["tint"], 3,
        "entries for absent files are kept"
    );
    assert_eq!(after["notes"], "hello");
}

#[test]
fn fr_ref_04_adoption_and_not_ready() {
    let mut e = env();
    explorer_pack(&e, "Moods", &[("m.png", swatch(3))]);
    e.ctx.faults = Faults::parse("after_temp:pack.json", FaultAction::Error).unwrap();
    let cards = packs::list_packs(&e.ctx, &e.settings);
    assert_eq!(cards[0].badges, [PackBadge::NotReady]);
    assert_eq!(cards[0].pack_id, None);
    e.ctx.faults = Faults::none();
    let cards = packs::list_packs(&e.ctx, &e.settings);
    assert!(cards[0].badges.is_empty());
    let id = cards[0].pack_id.unwrap();
    assert_eq!(cards[0].notices[0].code, "adopted");
    let again = packs::list_packs(&e.ctx, &e.settings);
    assert_eq!(again[0].pack_id, Some(id), "adopted once");
    assert!(again[0].notices.is_empty());
}

#[test]
fn fr_ref_05_copied_folder_gets_a_new_id() {
    let e = env();
    let out = create(&e, "B pack", &[("s.png", swatch(1))]);
    let copy = refs(&e).join("A copy");
    std::fs::create_dir(&copy).unwrap();
    std::fs::copy(refs(&e).join("B pack/pack.json"), copy.join("pack.json")).unwrap();
    let cards = packs::list_packs(&e.ctx, &e.settings);
    let a = cards.iter().find(|c| c.name == "A copy").unwrap();
    let b = cards.iter().find(|c| c.name == "B pack").unwrap();
    assert_eq!(
        a.pack_id,
        Some(out.pack.pack_id),
        "first in name order keeps it"
    );
    assert_ne!(b.pack_id, Some(out.pack.pack_id));
    assert_eq!(b.notices[0].code, "new_id");
    let cards = packs::list_packs(&e.ctx, &e.settings);
    assert_ne!(cards[0].pack_id, cards[1].pack_id, "stable afterwards");
}

#[test]
fn fr_ref_06_cards_sorted_with_notes_and_offline_badge() {
    let e = env();
    let z = create(&e, "zeta", &[]);
    create(&e, "Alpha", &[("a.png", swatch(1)), ("b.png", swatch(2))]);
    packs::update(
        &e.ctx,
        &loc(&e, z.pack.pack_id),
        Some("first line\nsecond"),
        &BTreeMap::new(),
    )
    .unwrap();
    explorer_pack(&e, "Cloud", &[]);
    std::fs::write(refs(&e).join("Cloud/pack.json"), b"{}").unwrap();
    e.fs.cloud_only
        .lock()
        .unwrap()
        .insert(refs(&e).join("Cloud").join(PACK_FILE));
    e.fs.offline.store(true, Ordering::Relaxed);
    let cards = packs::list_packs(&e.ctx, &e.settings);
    let n: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(n, ["Alpha", "Cloud", "zeta"]);
    assert_eq!(cards[0].image_count, 2);
    assert_eq!(cards[2].notes_first_line, "first line");
    assert_eq!(cards[1].badges, [PackBadge::NotAvailableOffline]);
    assert!(
        refs(&e).join("Cloud/pack.json").exists(),
        "nothing is written for an offline pack"
    );
}

#[test]
fn fr_ref_06_listing_hydrates_only_pack_json() {
    let e = env();
    let out = create(&e, "Swords", &[("a.png", swatch(1))]);
    let img = refs(&e).join("Swords/a.png");
    e.fs.cloud_only.lock().unwrap().insert(img.clone());
    packs::list_packs(&e.ctx, &e.settings);
    let v = packs::view(&e.ctx, &loc(&e, out.pack.pack_id));
    assert_eq!(v.images[0].state, ImageState::CloudOnly);
    assert_eq!(e.fs.hydrate_calls.load(Ordering::Relaxed), 0);
    assert!(e.fs.is_cloud_only(&img));
}

#[test]
fn fr_ref_08_add_copies_dedupes_suffixes_and_skips() {
    let e = env();
    let out = create(&e, "Swords", &[("a.png", swatch(1))]);
    let l = loc(&e, out.pack.pack_id);
    let same = e.outside("again/a-copy.png", &swatch(1));
    let other = e.outside("b/a.png", &swatch(2));
    let bad = e.outside("trunc.png", &swatch(3)[..40]);
    let txt = e.outside("x.txt", b"hello");
    let before = std::fs::read(&other).unwrap();
    let items = packs::add_images(
        &e.ctx,
        &l,
        &[same.clone(), other.clone(), bad.clone(), txt.clone()],
    );
    assert_eq!(
        items,
        [
            AddResult::AlreadyInPack {
                source: same,
                file: "a.png".into()
            },
            AddResult::Added {
                source: other.clone(),
                file: "a (2).png".into()
            },
            AddResult::Skipped {
                source: bad,
                reason: SkipReason::Corrupt
            },
            AddResult::Skipped {
                source: txt,
                reason: SkipReason::Unsupported
            },
        ]
    );
    assert_eq!(std::fs::read(&other).unwrap(), before, "source untouched");
    assert!(e.events.names().iter().any(|n| n == "ref:add-progress"));
}

#[test]
fn fr_ref_09_files_added_in_explorer_appear() {
    let e = env();
    let out = create(&e, "Swords", &[("a.png", swatch(1))]);
    std::fs::write(refs(&e).join("Swords/z.webp"), b"RIFF").unwrap();
    std::fs::write(refs(&e).join("Swords/b.png"), swatch(4)).unwrap();
    let v = packs::view(&e.ctx, &loc(&e, out.pack.pack_id));
    assert_eq!(names(&v), ["a.png", "b.png", "z.webp"]);
}

#[test]
fn fr_ref_10_rename_keeps_id_and_contents() {
    let e = env();
    let a = create(&e, "Swords", &[("a.png", swatch(1))]);
    create(&e, "Axes", &[]);
    let l = loc(&e, a.pack.pack_id);
    let before = std::fs::read(refs(&e).join("Swords/pack.json")).unwrap();
    let renamed = packs::rename(&l, "Blades").unwrap();
    assert_eq!(renamed.name, "Blades");
    assert_eq!(
        std::fs::read(refs(&e).join("Blades/pack.json")).unwrap(),
        before
    );
    assert_eq!(loc(&e, a.pack.pack_id).name, "Blades");
    let l = loc(&e, a.pack.pack_id);
    assert_eq!(
        packs::rename(&l, "axes").unwrap_err().code,
        ErrorCode::NameTaken
    );
    assert_eq!(
        packs::rename(&l, "a/b").unwrap_err().code,
        ErrorCode::InvalidName
    );
    assert_eq!(
        packs::rename(&l, "blades").unwrap().name,
        "blades",
        "case-only rename"
    );
}

#[test]
fn fr_ref_11_update_sets_notes_and_roles() {
    let e = env();
    let out = create(&e, "Faces", &[("a.png", swatch(1)), ("b.png", swatch(2))]);
    let l = loc(&e, out.pack.pack_id);
    let roles = BTreeMap::from([
        ("a.png".to_owned(), Some(ReferenceRole::Identity)),
        ("b.png".to_owned(), Some(ReferenceRole::Style)),
    ]);
    let l = packs::update(&e.ctx, &l, Some("faces"), &roles).unwrap();
    assert_eq!(l.json.role_of("a.png"), Some(ReferenceRole::Identity));
    let l = packs::update(
        &e.ctx,
        &l,
        None,
        &BTreeMap::from([("b.png".to_owned(), None)]),
    )
    .unwrap();
    assert_eq!(l.json.role_of("b.png"), None);
    assert!(!l.json.images.contains_key("b.png"));
    assert_eq!(l.json.notes, "faces", "notes kept when not given");
    let err = packs::update(
        &e.ctx,
        &l,
        None,
        &BTreeMap::from([("nope.png".to_owned(), Some(ReferenceRole::Style))]),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(err.field.as_deref(), Some("roles.nope.png"));
    assert_eq!(
        loc(&e, out.pack.pack_id).json.notes,
        "faces",
        "a failed update writes nothing"
    );
}

#[test]
fn fr_ref_12_remove_images_and_restore_with_suffix() {
    let e = env();
    let out = create(&e, "Swords", &[("a.png", swatch(1)), ("b.png", swatch(2))]);
    let l = loc(&e, out.pack.pack_id);
    let id = trash::remove_images(&e.ctx, &l, &["a.png".into()]).unwrap();
    assert!(!refs(&e).join("Swords/a.png").exists());
    let item_dir = trash::trash_dir(&e.lib()).join(id.to_string());
    assert!(item_dir.join("a.png").is_file() && item_dir.join("item.json").is_file());
    let listed = trash::list(&e.ctx, &e.settings);
    assert_eq!(listed[0].kind, TrashKind::Images);
    assert_eq!(listed[0].pack_name, "Swords");
    assert_eq!(listed[0].files, ["a.png"]);
    std::fs::write(refs(&e).join("Swords/a.png"), swatch(9)).unwrap();
    let path = trash::restore(&e.ctx, &e.settings, id).unwrap();
    assert_eq!(path, refs(&e).join("Swords"));
    assert!(refs(&e).join("Swords/a (2).png").is_file(), "taken → (2)");
    assert!(!item_dir.exists());
    assert_eq!(
        trash::restore(&e.ctx, &e.settings, id).unwrap_err().code,
        ErrorCode::UnknownId
    );
    let err = trash::remove_images(&e.ctx, &l, &["zzz.png".into()]).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

#[test]
fn fr_ref_12_delete_pack_and_restore() {
    let e = env();
    let out = create(&e, "Swords", &[("a.png", swatch(1))]);
    let id = trash::delete_pack(&e.ctx, &loc(&e, out.pack.pack_id)).unwrap();
    assert!(!refs(&e).join("Swords").exists());
    assert!(find_pack(&e.ctx, &e.settings, out.pack.pack_id).is_none());
    assert!(
        packs::list_packs(&e.ctx, &e.settings).is_empty(),
        ".pforge isn't a pack"
    );
    create(&e, "swords", &[]);
    let path = trash::restore(&e.ctx, &e.settings, id).unwrap();
    assert_eq!(path, refs(&e).join("Swords (2)"));
    assert_eq!(loc(&e, out.pack.pack_id).name, "Swords (2)", "same pack id");
}

#[test]
fn fr_ref_12_purge_after_30_days_under_the_fake_clock() {
    let mut e = env();
    let out = create(&e, "Swords", &[("a.png", swatch(1))]);
    trash::delete_pack(&e.ctx, &loc(&e, out.pack.pack_id)).unwrap();
    e.ctx.clock = Box::new(ManualClock::new(
        crate::time::Timestamp(T0).plus(Duration::from_secs(30 * 86_400)),
    ));
    assert_eq!(
        trash::purge(&e.ctx, &e.settings).0,
        0,
        "exactly 30 days: kept"
    );
    e.ctx.clock = Box::new(ManualClock::new(
        crate::time::Timestamp(T0).plus(Duration::from_secs(30 * 86_400 + 1)),
    ));
    super::sweep_startup(&e.ctx, &e.settings);
    assert!(trash::list(&e.ctx, &e.settings).is_empty());
    let purged = e
        .events
        .0
        .lock()
        .unwrap()
        .iter()
        .find(|(n, _)| n == "ref:trash-purged")
        .map(|(_, p)| p.clone())
        .unwrap();
    assert_eq!(purged["count"], 1);
    assert!(purged["bytes"].as_u64().unwrap() > 0);
}

#[test]
fn fr_ref_12_interrupted_trash_move_rolls_forward() {
    let e = env();
    let out = create(&e, "Swords", &[("a.png", swatch(1)), ("b.png", swatch(2))]);
    let l = loc(&e, out.pack.pack_id);
    // A kill after item.json and the first move: b.png is still in the pack.
    let item = trash::TrashItem {
        schema_version: 1,
        id: e.ctx.new_id(),
        kind: TrashKind::Images,
        pack_id: Some(l.json.pack_id),
        from: "References/Swords".into(),
        files: vec!["a.png".into(), "b.png".into()],
        trashed_at: e.ctx.now(),
        restored_at: None,
        extra: Default::default(),
    };
    let item_dir = trash::trash_dir(&e.lib()).join(item.id.to_string());
    std::fs::create_dir_all(&item_dir).unwrap();
    std::fs::write(
        item_dir.join("item.json"),
        serde_json::to_vec(&item).unwrap(),
    )
    .unwrap();
    std::fs::rename(refs(&e).join("Swords/a.png"), item_dir.join("a.png")).unwrap();
    super::sweep_startup(&e.ctx, &e.settings);
    assert!(item_dir.join("b.png").is_file(), "rolled forward");
    assert!(!refs(&e).join("Swords/b.png").exists());
    // An interrupted restore is finished, not rolled back.
    let mut item = item;
    item.restored_at = Some(e.ctx.now());
    std::fs::write(
        item_dir.join("item.json"),
        serde_json::to_vec(&item).unwrap(),
    )
    .unwrap();
    std::fs::rename(item_dir.join("a.png"), refs(&e).join("Swords/a.png")).unwrap();
    super::sweep_startup(&e.ctx, &e.settings);
    assert!(!item_dir.exists());
    assert!(refs(&e).join("Swords/a.png").is_file() && refs(&e).join("Swords/b.png").is_file());
}

#[test]
fn fr_ref_13_thumbnail_mosaic_of_local_images() {
    let e = env();
    let out = create(
        &e,
        "Swords",
        &[("a.png", swatch(1)), ("b.png", swatch(200))],
    );
    let thumb = refs(&e).join("Swords/.pforge/thumb.jpg");
    assert!(thumb.is_file(), "built after create");
    let img = image::open(&thumb).unwrap();
    assert_eq!((img.width(), img.height()), (512, 512));
    assert!(e.events.names().iter().any(|n| n == "ref:thumb-updated"));
    let cards = packs::list_packs(&e.ctx, &e.settings);
    assert_eq!(cards[0].thumb.as_deref(), Some(thumb.as_path()));
    // Cloud-only images are never hydrated for it; with none local, no thumbnail.
    let l = loc(&e, out.pack.pack_id);
    for f in ["a.png", "b.png"] {
        e.fs.cloud_only.lock().unwrap().insert(l.dir.join(f));
    }
    thumb::request(&e.ctx, &l.dir, l.json.pack_id);
    assert!(!thumb.exists());
    assert_eq!(e.fs.hydrate_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn fr_ref_13_stale_thumbnail_is_rebuilt_by_a_listing() {
    let e = env();
    create(&e, "Swords", &[("a.png", swatch(1))]);
    let thumb = refs(&e).join("Swords/.pforge/thumb.jpg");
    std::fs::remove_file(&thumb).unwrap();
    packs::list_packs(&e.ctx, &e.settings);
    assert!(thumb.is_file(), "missing → rebuilt");
    let images = image_files(&refs(&e).join("Swords"));
    assert!(!thumb::needs_rebuild(&thumb, &images));
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&thumb)
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert!(
        thumb::needs_rebuild(&thumb, &images),
        "older than the newest image"
    );
}

#[test]
fn fr_ref_28_pack_json_conflict_copy_later_updated_at_wins() {
    let e = env();
    let out = create(&e, "Swords", &[("a.png", swatch(1))]);
    let dir = refs(&e).join("Swords");
    let mut newer = loc(&e, out.pack.pack_id).json;
    newer.notes = "from the laptop".into();
    newer.updated_at = newer.updated_at.plus(Duration::from_secs(60));
    std::fs::write(
        dir.join("pack-LAPTOP.json"),
        serde_json::to_vec(&newer).unwrap(),
    )
    .unwrap();
    let cards = packs::list_packs(&e.ctx, &e.settings);
    assert_eq!(cards[0].notices[0].code, "conflict_resolved");
    assert_eq!(cards[0].notes_first_line, "from the laptop");
    assert!(!dir.join("pack-LAPTOP.json").exists());
    let t = trash::list(&e.ctx, &e.settings);
    assert_eq!(t[0].files, ["pack.json"], "the older one is in the trash");
    // R61 (like R57): it is not restorable, so it can't loop back as a conflict copy.
    assert!(!t[0].restorable);
    let err = trash::restore(&e.ctx, &e.settings, t[0].id).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotRestorable);
    assert_eq!(trash::list(&e.ctx, &e.settings).len(), 1, "still in the trash");
    assert!(!dir.join("pack (2).json").exists());
    // An older conflict copy loses.
    let mut older = newer.clone();
    older.notes = "stale".into();
    older.updated_at = crate::time::Timestamp(0);
    std::fs::write(
        dir.join("pack (2).json"),
        serde_json::to_vec(&older).unwrap(),
    )
    .unwrap();
    let cards = packs::list_packs(&e.ctx, &e.settings);
    assert_eq!(cards[0].notes_first_line, "from the laptop");
    assert_eq!(cards[0].pack_id, Some(out.pack.pack_id));
    let t = trash::list(&e.ctx, &e.settings);
    assert_eq!(t[0].files, ["pack (2).json"]);
    assert!(!t[0].restorable);
    // Images and packs stay restorable.
    let l = loc(&e, out.pack.pack_id);
    let img = trash::remove_images(&e.ctx, &l, &["a.png".into()]).unwrap();
    let t = trash::list(&e.ctx, &e.settings);
    assert!(t.iter().find(|i| i.id == img).unwrap().restorable);
    trash::restore(&e.ctx, &e.settings, img).unwrap();
}

#[test]
fn fr_ref_29_commands_validate_and_round_trip() {
    let e = env();
    let mut ctx = crate::context::Context::deterministic(
        e.dir.path().join("data2"),
        crate::time::Timestamp(T0),
    );
    ctx.library_override = Some(e.lib());
    let core = crate::Core::new(ctx).unwrap();
    let src = e.outside("s.png", &swatch(5));
    let out = core
        .dispatch(
            "ref_pack_create",
            json!({"name": "Swords", "paths": [src.to_string_lossy()]}),
        )
        .unwrap();
    assert_eq!(out["items"][0]["status"], "added");
    let id = out["pack"]["pack_id"].as_str().unwrap().to_owned();
    let cards = core.dispatch("ref_list_packs", json!({})).unwrap();
    assert_eq!(cards[0]["id"], id.as_str());
    assert!(cards[0]["thumb_url"].as_str().unwrap().contains("/i/l-"));
    let got = core
        .dispatch("ref_pack_get", json!({"pack_id": id}))
        .unwrap();
    assert!(got["images"][0]["thumb_url"].as_str().is_some());
    for (cmd, input, field) in [
        ("ref_pack_get", json!({"pack_id": "nope"}), "input"),
        ("ref_pack_create", json!({"name": ""}), "name"),
        (
            "ref_pack_update",
            json!({"pack_id": id, "notes": "x".repeat(2001)}),
            "notes",
        ),
        (
            "ref_pack_remove_images",
            json!({"pack_id": id, "files": ["../x.png"]}),
            "files[0]",
        ),
        (
            "ref_pack_add_images",
            json!({"pack_id": id, "paths": []}),
            "paths",
        ),
    ] {
        let err = core.dispatch(cmd, input).unwrap_err();
        assert_eq!(
            (err.code, err.field.as_deref()),
            (ErrorCode::InvalidInput, Some(field)),
            "{cmd}"
        );
    }
    let unknown = json!({"pack_id": ulid::Ulid::from_parts(1, 1).to_string()});
    assert_eq!(
        core.dispatch("ref_pack_delete", unknown).unwrap_err().code,
        ErrorCode::UnknownId
    );
    let renamed = core
        .dispatch("ref_pack_rename", json!({"pack_id": id, "name": "Blades"}))
        .unwrap();
    assert_eq!(renamed["name"], "Blades");
    let del = core
        .dispatch("ref_pack_delete", json!({"pack_id": id}))
        .unwrap();
    let items = core.dispatch("ref_trash_list", json!({})).unwrap();
    assert_eq!(items[0]["id"], del["item_id"]);
    let restored = core
        .dispatch("ref_trash_restore", json!({"id": del["item_id"]}))
        .unwrap();
    assert!(Path::new(restored["path"].as_str().unwrap()).ends_with("Blades"));
}

/// FR-REF-14 (walk R6, R61): the pack view's and the block's image thumbnails are ≤ 256 px,
/// small files included; the packs-home mosaic keeps the 512 px library key.
#[test]
fn fr_ref_14_image_thumbnails_are_256_px() {
    let e = env();
    let mut ctx = crate::context::Context::deterministic(
        e.dir.path().join("data2"),
        crate::time::Timestamp(T0),
    );
    ctx.library_override = Some(e.lib());
    let core = crate::Core::new(ctx).unwrap();
    let src = e.outside("s.png", &crate::test_support::png_bytes(320, 300, 7));
    let out = core
        .dispatch(
            "ref_pack_create",
            json!({"name": "Swords", "paths": [src.to_string_lossy()]}),
        )
        .unwrap();
    let id = out["pack"]["pack_id"].as_str().unwrap().to_owned();
    let size = |url: &str| {
        let path = &url[url.find("/i/").unwrap()..];
        let res = core.serve_scheme(path);
        assert_eq!(res.status, 200, "{url}");
        let img = image::load_from_memory(&res.body).unwrap();
        (img.width(), img.height())
    };
    let got = core
        .dispatch("ref_pack_get", json!({"pack_id": id}))
        .unwrap();
    assert_eq!(size(got["images"][0]["thumb_url"].as_str().unwrap()), (256, 240));

    // The References block over a project with the same file in references/.
    let dir = e.project("Crypt");
    std::fs::copy(&src, dir.join("references").join("s.png")).unwrap();
    core.dispatch("project_open", json!({"path": dir})).unwrap();
    let r = core
        .dispatch(
            "ref_resolve",
            json!({"model_id": "fake-white-edit", "references": null}),
        )
        .unwrap();
    assert_eq!(size(r["items"][0]["thumb_url"].as_str().unwrap()), (256, 240));
}

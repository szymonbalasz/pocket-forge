//! REF slice 4 tests: resolve (FR-REF-15 … 19) and prepare for send (FR-REF-21 … 25).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use pf_provider::{Provider, ReferenceRole};
use serde_json::json;

use super::packs;
use super::prepare::{self, RefSession, SENT_MAX_SIDE, estimate_inputs};
use super::resolve::{self, DraftItem, DraftPack, DraftReferences, ItemState, Slot, Source};
use super::*;
use crate::test_support::{Env, env, png_bytes};

fn slot(name: &str, role: Option<ReferenceRole>, min: u32, max: u32, cost: bool) -> Slot {
    Slot {
        name: name.into(),
        role,
        min,
        max,
        cost_affecting: cost,
        array: max > 1,
    }
}

/// Like `fixtures/rgn/capabilities-refs.json`: style ≤ 2 cost-affecting, identity ≤ 1.
fn style_identity() -> Vec<Slot> {
    vec![
        slot("style_refs", Some(ReferenceRole::Style), 0, 2, true),
        slot("identity_ref", Some(ReferenceRole::Identity), 0, 1, false),
    ]
}

fn one_slot(max: u32) -> Vec<Slot> {
    vec![slot("style_refs", None, 0, max, false)]
}

fn swatch(shade: u8) -> Vec<u8> {
    png_bytes(32, 24, shade)
}

/// A Maps project with `references/` files.
fn project_with_refs(e: &Env, files: &[(&str, Vec<u8>)]) -> PathBuf {
    let dir = e.project("Crypt");
    for (f, b) in files {
        std::fs::write(dir.join("references").join(f), b).unwrap();
    }
    dir
}

fn pack(e: &Env, name: &str, files: &[(&str, Vec<u8>)]) -> PackLoc {
    let paths: Vec<PathBuf> = files
        .iter()
        .map(|(f, b)| e.outside(&format!("{name}/{f}"), b))
        .collect();
    let out = packs::create(&e.ctx, &e.settings, name, &paths, false).unwrap();
    find_pack(&e.ctx, &e.settings, out.pack.pack_id).unwrap()
}

fn picked(l: &PackLoc) -> DraftPack {
    DraftPack {
        pack_id: l.json.pack_id,
        name: l.name.clone(),
        extra: Default::default(),
    }
}

fn resolve(
    e: &Env,
    s: &RefSession,
    dir: &Path,
    slots: &[Slot],
    prev: Option<&DraftReferences>,
) -> resolve::Resolved {
    resolve::resolve(&e.ctx, &e.settings, &s.probes, dir, slots, prev)
}

fn on_keys(r: &resolve::Resolved) -> Vec<&str> {
    r.items
        .iter()
        .filter(|i| i.on)
        .map(|i| i.key.as_str())
        .collect()
}

fn prep(
    e: &Env,
    s: &RefSession,
    dir: &Path,
    slots: &[Slot],
    prev: Option<&DraftReferences>,
) -> CoreResult<prepare::Prepared> {
    prepare::prepare(
        &e.ctx,
        s,
        &e.settings,
        dir,
        slots,
        SENT_MAX_SIDE,
        u64::MAX,
        prev,
    )
}

#[test]
fn fr_ref_15_no_slots_hides_the_block_and_sends_nothing() {
    let e = env();
    let dir = project_with_refs(&e, &[("a.png", swatch(1))]);
    let s = RefSession::default();
    let r = resolve(&e, &s, &dir, &[], None);
    assert!(!r.visible && r.items.is_empty() && r.slots.is_empty());
    assert!(prep(&e, &s, &dir, &[], None).unwrap().sent.is_empty());
}

#[test]
fn fr_ref_16_available_items_order_and_keys() {
    let e = env();
    let dir = project_with_refs(&e, &[("b.png", swatch(1)), ("A.png", swatch(2))]);
    let z = pack(&e, "Zeds", &[("z2.png", swatch(3)), ("Z1.png", swatch(4))]);
    let a = pack(&e, "Axes", &[("x.png", swatch(5))]);
    let prev = DraftReferences {
        packs: vec![picked(&z), picked(&a)],
        ..Default::default()
    };
    let r = resolve(&e, &RefSession::default(), &dir, &one_slot(10), Some(&prev));
    let keys: Vec<String> = r.items.iter().map(|i| i.key.clone()).collect();
    assert_eq!(
        keys,
        [
            "project:A.png".to_owned(),
            "project:b.png".into(),
            format!("pack:{}:Z1.png", z.json.pack_id),
            format!("pack:{}:z2.png", z.json.pack_id),
            format!("pack:{}:x.png", a.json.pack_id),
        ],
        "project first, then packs in pick order, names case-insensitive"
    );
    assert_eq!(r.items[2].source, Source::Pack);
    assert_eq!(r.items[2].pack_name.as_deref(), Some("Zeds"));
}

#[test]
fn fr_ref_17_defaults_keep_state_and_drop_missing() {
    let e = env();
    let dir = project_with_refs(
        &e,
        &[
            ("a.png", swatch(1)),
            ("b.png", swatch(2)),
            ("c.png", swatch(3)),
        ],
    );
    let s = RefSession::default();
    let r = resolve(&e, &s, &dir, &one_slot(2), None);
    assert_eq!(
        on_keys(&r),
        ["project:a.png", "project:b.png"],
        "first N ticked"
    );
    assert_eq!(r.slots[0].on, 2);
    // The user swaps b for c; a new file arrives; a file disappears.
    let mut prev = r.references.clone();
    prev.items[1].on = false;
    prev.items[2].on = true;
    prev.items[0].on = false;
    std::fs::write(dir.join("references/d.png"), swatch(4)).unwrap();
    std::fs::remove_file(dir.join("references/b.png")).unwrap();
    let r = resolve(&e, &s, &dir, &one_slot(2), Some(&prev));
    assert_eq!(
        on_keys(&r),
        ["project:c.png", "project:d.png"],
        "kept, new one has room"
    );
    assert_eq!(r.notices[0].code, "dropped");
    assert_eq!(r.notices[0].key.as_deref(), Some("project:b.png"));
    assert!(!r.references.items.iter().any(|i| i.key == "project:b.png"));
}

#[test]
fn fr_ref_17_default_role_from_pack_json_or_first_slot() {
    let e = env();
    let dir = project_with_refs(&e, &[("mood.png", swatch(1))]);
    let p = pack(
        &e,
        "Faces",
        &[("face.png", swatch(2)), ("hat.png", swatch(3))],
    );
    packs::update(
        &e.ctx,
        &p,
        None,
        &BTreeMap::from([
            ("face.png".to_owned(), Some(ReferenceRole::Identity)),
            ("hat.png".to_owned(), Some(ReferenceRole::Structure)),
        ]),
    )
    .unwrap();
    let prev = DraftReferences {
        packs: vec![picked(&p)],
        ..Default::default()
    };
    let r = resolve(
        &e,
        &RefSession::default(),
        &dir,
        &style_identity(),
        Some(&prev),
    );
    let role = |f: &str| r.items.iter().find(|i| i.file == f).unwrap().role;
    assert_eq!(
        role("face.png"),
        Some(ReferenceRole::Identity),
        "pack role kept"
    );
    assert_eq!(
        role("hat.png"),
        Some(ReferenceRole::Style),
        "no structure slot → first slot"
    );
    assert_eq!(role("mood.png"), Some(ReferenceRole::Style));
    let face = r.items.iter().find(|i| i.file == "face.png").unwrap();
    assert_eq!(face.slot, "identity_ref");
}

#[test]
fn fr_ref_18_limits_model_change_and_min() {
    let e = env();
    let dir = project_with_refs(
        &e,
        &[
            ("a.png", swatch(1)),
            ("b.png", swatch(2)),
            ("c.png", swatch(3)),
        ],
    );
    let s = RefSession::default();
    let r = resolve(&e, &s, &dir, &one_slot(3), None);
    assert_eq!(r.slots[0].on, 3);
    // A model with a lower limit: the last ticked items in list order go off.
    let r = resolve(&e, &s, &dir, &one_slot(1), Some(&r.references));
    assert_eq!(on_keys(&r), ["project:a.png"]);
    assert_eq!(
        r.notices
            .iter()
            .filter(|n| n.code == "limit_lowered")
            .count(),
        2
    );
    // A required slot under its minimum disables submit and names the slot.
    let mut none = r.references.clone();
    for i in &mut none.items {
        i.on = false;
    }
    let req = vec![slot(
        "character",
        Some(ReferenceRole::Identity),
        1,
        1,
        false,
    )];
    let r = resolve(&e, &s, &dir, &req, Some(&none));
    assert!(!r.ready);
    let err = prep(&e, &s, &dir, &req, Some(&none)).unwrap_err();
    assert_eq!(err.code, ErrorCode::ReferencesRequired);
    assert_eq!(err.detail, json!({"slot": "character", "min": 1}));
    assert_eq!(s.prepared.misses.load(Ordering::Relaxed), 0, "nothing read");
}

#[test]
fn fr_ref_19_roles_route_items_to_slots() {
    let e = env();
    let dir = project_with_refs(&e, &[("a.png", swatch(1)), ("b.png", swatch(2))]);
    let s = RefSession::default();
    let prev = DraftReferences {
        packs: vec![],
        items: vec![
            DraftItem {
                key: "project:a.png".into(),
                on: true,
                role: Some(ReferenceRole::Identity),
                extra: Default::default(),
            },
            DraftItem {
                key: "project:b.png".into(),
                on: true,
                role: Some(ReferenceRole::Structure),
                extra: Default::default(),
            },
        ],
        extra: Default::default(),
    };
    let r = resolve(&e, &s, &dir, &style_identity(), Some(&prev));
    assert_eq!(r.items[0].slot, "identity_ref");
    assert_eq!(
        r.items[1].role,
        Some(ReferenceRole::Style),
        "role not offered → reset"
    );
    assert_eq!((r.slots[0].on, r.slots[1].on), (1, 1));
    // One slot without a role: items carry no role.
    let r = resolve(&e, &s, &dir, &one_slot(4), Some(&prev));
    assert!(r.items.iter().all(|i| i.role.is_none()));
}

#[test]
fn fr_ref_21_prepare_hydrates_offline_corrupt_and_dedupes() {
    let e = env();
    let dir = project_with_refs(
        &e,
        &[
            ("a.png", swatch(1)),
            ("b.png", swatch(1)),
            ("c.png", swatch(3)),
        ],
    );
    let s = RefSession::default();
    let cloud = dir.join("references/c.png");
    e.fs.cloud_only.lock().unwrap().insert(cloud.clone());
    let out = prep(&e, &s, &dir, &one_slot(3), None).unwrap();
    assert_eq!(out.sent.len(), 2, "identical bytes are sent once");
    assert_eq!(out.notices[0].code, "duplicate");
    assert_eq!(out.notices[0].key.as_deref(), Some("project:b.png"));
    assert_eq!(
        e.fs.hydrate_calls.load(Ordering::Relaxed),
        1,
        "cloud-only hydrated"
    );
    // Offline: a cloud-only item fails before anything is sent.
    e.fs.cloud_only.lock().unwrap().insert(cloud);
    e.fs.offline.store(true, Ordering::Relaxed);
    let err = prep(&e, &s, &dir, &one_slot(3), None).unwrap_err();
    assert_eq!(err.code, ErrorCode::ReferenceUnavailable);
    assert_eq!(
        err.detail,
        json!({"key": "project:c.png", "reason": "not_available_offline"})
    );
    e.fs.offline.store(false, Ordering::Relaxed);
    // A file that broke between the resolve and the read (same size and mtime, so the header
    // probe is remembered as fine).
    let a = dir.join("references/a.png");
    let s = RefSession::default();
    let r = resolve(&e, &s, &dir, &one_slot(3), None);
    let meta = std::fs::metadata(&a).unwrap();
    std::fs::write(&a, vec![0u8; meta.len() as usize]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&a)
        .unwrap()
        .set_modified(meta.modified().unwrap())
        .unwrap();
    let err = prep(&e, &s, &dir, &one_slot(3), Some(&r.references)).unwrap_err();
    assert_eq!(
        err.detail,
        json!({"key": "project:a.png", "reason": "corrupt"})
    );
    // A corrupt file found by the resolve can't be ticked.
    std::fs::write(&a, &swatch(1)[..30]).unwrap();
    let r = resolve(&e, &s, &dir, &one_slot(3), Some(&r.references));
    assert_eq!(r.items[0].badges, ["corrupt"]);
    assert!(!r.items[0].on && !r.items[0].checkable);
}

#[test]
fn fr_ref_21_duplicate_badge_from_the_session_cache() {
    let e = env();
    let dir = project_with_refs(&e, &[("a.png", swatch(1)), ("b.png", swatch(1))]);
    let s = RefSession::default();
    let out = prep(&e, &s, &dir, &one_slot(2), None).unwrap();
    assert_eq!(out.sent.len(), 1);
    let mut r = resolve(&e, &s, &dir, &one_slot(2), None);
    resolve::mark_duplicates(&mut r.items, |p| s.prepared.content_hash_of(p));
    assert_eq!(r.items[1].badges, ["duplicate"]);
    assert!(r.items[0].badges.is_empty());
}

#[test]
fn fr_ref_22_sent_bytes_rule_with_a_downscale_golden() {
    let e = env();
    // Left half red, right half blue: a decoded-pixel golden (OQ-IMG-07).
    let mut big = Vec::new();
    image::RgbImage::from_fn(4096, 2048, |x, _| {
        if x < 2048 {
            image::Rgb([255, 0, 0])
        } else {
            image::Rgb([0, 0, 255])
        }
    })
    .write_to(&mut std::io::Cursor::new(&mut big), image::ImageFormat::Png)
    .unwrap();
    let small = swatch(7);
    let dir = project_with_refs(&e, &[("big.png", big), ("small.png", small.clone())]);
    let s = RefSession::default();
    let out = prep(&e, &s, &dir, &one_slot(2), None).unwrap();
    let (b, sm) = (&out.sent[0], &out.sent[1]);
    assert_eq!(
        (b.row.sent.w, b.row.sent.h, b.row.sent.resized),
        (2048, 1024, true)
    );
    let px = image::load_from_memory(&b.bytes).unwrap().into_rgb8();
    assert_eq!(px.get_pixel(100, 500).0, [255, 0, 0]);
    assert_eq!(px.get_pixel(1950, 500).0, [0, 0, 255]);
    assert_ne!(b.row.sent.hash, b.row.content_hash);
    assert!(!sm.row.sent.resized);
    assert_eq!(*sm.bytes, small, "small files are sent as-is");
    assert_eq!(sm.row.sent.hash, sm.row.content_hash);
    // Deterministic: a fresh session gives the same sent bytes.
    let again = prep(&e, &RefSession::default(), &dir, &one_slot(2), None).unwrap();
    assert_eq!(again.sent[0].hash, b.hash);
    // The model's max px and max bytes also cap it.
    let capped = prepare::prepare(
        &e.ctx,
        &RefSession::default(),
        &e.settings,
        &dir,
        &one_slot(2),
        16,
        u64::MAX,
        None,
    )
    .unwrap();
    assert_eq!(capped.sent[1].row.sent.w, 16);
    assert!(capped.sent[1].row.sent.resized);
}

#[test]
fn fr_ref_23_cache_hits_by_path_size_mtime() {
    let e = env();
    let dir = project_with_refs(&e, &[("a.png", swatch(1)), ("b.png", swatch(2))]);
    let s = RefSession::default();
    let first = prep(&e, &s, &dir, &one_slot(2), None).unwrap();
    assert_eq!(s.prepared.misses.load(Ordering::Relaxed), 2);
    let second = prep(&e, &s, &dir, &one_slot(2), None).unwrap();
    assert_eq!(s.prepared.hits.load(Ordering::Relaxed), 2, "warm: no reads");
    assert_eq!(first, second);
    std::fs::write(dir.join("references/a.png"), png_bytes(40, 30, 9)).unwrap();
    prep(&e, &s, &dir, &one_slot(2), None).unwrap();
    assert_eq!(
        s.prepared.misses.load(Ordering::Relaxed),
        3,
        "changed file re-read"
    );
    assert!(s.prepared.bytes() > 0 && s.prepared.bytes() <= prepare::CACHE_MAX_BYTES);
}

#[test]
fn fr_ref_24_estimate_inputs_only_cost_affecting_slots() {
    let e = env();
    let dir = project_with_refs(&e, &[("a.png", swatch(1)), ("b.png", swatch(2))]);
    let p = pack(&e, "Faces", &[("face.png", swatch(3))]);
    packs::update(
        &e.ctx,
        &p,
        None,
        &BTreeMap::from([("face.png".to_owned(), Some(ReferenceRole::Identity))]),
    )
    .unwrap();
    let prev = DraftReferences {
        packs: vec![picked(&p)],
        ..Default::default()
    };
    let s = RefSession::default();
    let slots = style_identity();
    let out = prep(&e, &s, &dir, &slots, Some(&prev)).unwrap();
    let inputs = estimate_inputs(&out.sent, &slots);
    assert_eq!(inputs.len(), 2, "the identity ref doesn't affect cost");
    assert!(inputs.iter().all(|(slot, _)| slot == "style_refs"));
    assert_eq!(inputs[0].1, out.sent[0].hash);
    // Swapping the identity image leaves the estimate inputs unchanged.
    std::fs::write(p.dir.join("face.png"), swatch(4)).unwrap();
    let out2 = prep(&e, &s, &dir, &slots, Some(&prev)).unwrap();
    assert_eq!(estimate_inputs(&out2.sent, &slots), inputs);
    assert_ne!(out2.sent[2].hash, out.sent[2].hash);
}

#[test]
fn fr_ref_25_job_rows() {
    let e = env();
    let dir = project_with_refs(&e, &[("mood.png", swatch(1))]);
    let p = pack(&e, "Swords", &[("long.png", swatch(2))]);
    let prev = DraftReferences {
        packs: vec![picked(&p)],
        ..Default::default()
    };
    let out = prep(&e, &RefSession::default(), &dir, &one_slot(4), Some(&prev)).unwrap();
    let rows = serde_json::to_value(out.rows()).unwrap();
    assert_eq!(rows[0]["source"], "project");
    assert_eq!(rows[0]["path"], "references/mood.png");
    assert_eq!(rows[0]["pack_id"], serde_json::Value::Null);
    assert_eq!(rows[1]["source"], "pack");
    assert_eq!(rows[1]["path"], "References/Swords/long.png");
    assert_eq!(rows[1]["pack_id"], p.json.pack_id.to_string());
    assert_eq!(rows[1]["pack_name"], "Swords");
    assert_eq!(rows[1]["slot"], "style_refs");
    assert_eq!(
        rows[1]["content_hash"].as_str().unwrap(),
        crate::fsx::hash_file(&p.dir.join("long.png")).unwrap()
    );
    // Declared field order (schema §1).
    let text = serde_json::to_string(&out.rows()[1]).unwrap();
    let pos = |k: &str| text.find(&format!("\"{k}\":")).unwrap();
    let order = [
        "source",
        "pack_id",
        "pack_name",
        "path",
        "content_hash",
        "slot",
        "role",
        "sent",
        "w",
        "h",
        "bytes",
        "hash",
        "resized",
    ];
    assert!(order.windows(2).all(|w| pos(w[0]) < pos(w[1])), "{text}");
}

#[test]
fn fr_ref_26_renamed_and_deleted_packs() {
    let e = env();
    let dir = project_with_refs(&e, &[]);
    let p = pack(&e, "Blades", &[("a.png", swatch(1))]);
    let s = RefSession::default();
    let prev = DraftReferences {
        packs: vec![picked(&p)],
        ..Default::default()
    };
    let r = resolve(&e, &s, &dir, &one_slot(2), Some(&prev));
    let draft = r.references.clone();
    // Renamed in Explorer: found by id.
    std::fs::rename(&p.dir, p.dir.with_file_name("Swords")).unwrap();
    let r = resolve(&e, &s, &dir, &one_slot(2), Some(&draft));
    assert_eq!(on_keys(&r).len(), 1);
    assert_eq!(r.references.packs[0].name, "Swords");
    // Deleted: its items show as missing and stay in the draft; restored: they resolve again.
    let l = find_pack(&e.ctx, &e.settings, p.json.pack_id).unwrap();
    let item = trash::delete_pack(&e.ctx, &l).unwrap();
    let r = resolve(&e, &s, &dir, &one_slot(2), Some(&draft));
    assert_eq!(r.items[0].state, ItemState::Missing);
    assert!(!r.items[0].on && !r.items[0].checkable);
    assert_eq!(r.notices[0].code, "pack_unavailable");
    assert!(r.references.items[0].on, "kept for when it comes back");
    trash::restore(&e.ctx, &e.settings, item).unwrap();
    let r = resolve(&e, &s, &dir, &one_slot(2), Some(&r.references));
    assert_eq!(r.items[0].state, ItemState::Ok);
    assert!(r.items[0].on);
}

#[test]
fn fr_ref_29_ref_resolve_command() {
    let e = env();
    let dir = project_with_refs(&e, &[("a.png", swatch(1))]);
    let mut ctx = crate::context::Context::deterministic(
        e.dir.path().join("data2"),
        crate::time::Timestamp(0),
    );
    ctx.library_override = Some(e.lib());
    let core = crate::Core::new(ctx).unwrap();
    let input = json!({"model_id": "fake-white-edit", "references": null});
    assert_eq!(
        core.dispatch("ref_resolve", input.clone())
            .unwrap_err()
            .code,
        ErrorCode::NoProject
    );
    core.dispatch("project_open", json!({"path": dir})).unwrap();
    let out = core.dispatch("ref_resolve", input).unwrap();
    assert_eq!(out["read_only"], false);
    assert!(out["visible"].is_boolean());
    let err = core
        .dispatch("ref_resolve", json!({"model_id": "no-such-model"}))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownModel);
    let err = core
        .dispatch("ref_resolve", json!({"model_id": " "}))
        .unwrap_err();
    assert_eq!(err.field.as_deref(), Some("model_id"));
}

fn png_of(img: image::DynamicImage) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// The loop with the fake provider (§13): resolve against the fake's real slots, prepare, and
/// check the fake received exactly the sent hashes, with the cost-affecting refs in its estimate.
#[test]
fn fr_ref_21_fr_ref_24_fake_provider_receives_the_sent_hashes() {
    let e = env();
    let dir = project_with_refs(&e, &[("a.png", swatch(1)), ("b.png", swatch(2))]);
    let p = pack(&e, "Faces", &[("face.png", swatch(3))]);
    packs::update(
        &e.ctx,
        &p,
        None,
        &BTreeMap::from([("face.png".to_owned(), Some(ReferenceRole::Identity))]),
    )
    .unwrap();
    let mut ctx = crate::context::Context::deterministic(
        e.dir.path().join("data2"),
        crate::time::Timestamp(0),
    );
    ctx.library_override = Some(e.lib());
    let core = crate::Core::new(ctx).unwrap();
    core.dispatch("project_open", json!({"path": dir})).unwrap();
    let draft = json!({"packs": [{"pack_id": p.json.pack_id, "name": "Faces"}], "items": []});

    // fake-alpha-edit has no slots: the block is hidden.
    let hidden = core
        .dispatch(
            "ref_resolve",
            json!({"model_id": "fake-alpha-edit", "references": draft}),
        )
        .unwrap();
    assert_eq!(hidden["visible"], false);

    let r = core
        .dispatch(
            "ref_resolve",
            json!({"model_id": "fake-black-edit", "references": draft}),
        )
        .unwrap();
    assert_eq!(r["visible"], true);
    let slots: Vec<(&str, u64, bool)> = r["slots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap(),
                s["max"].as_u64().unwrap(),
                s["cost_affecting"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(slots, [("style_refs", 2, true), ("identity_ref", 1, false)]);
    let face = r["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["file"] == "face.png")
        .unwrap();
    assert_eq!(
        (face["slot"].as_str(), face["on"].as_bool()),
        (Some("identity_ref"), Some(true))
    );
    assert!(face["thumb_url"].as_str().unwrap().contains("/i/l-"));

    let fake = core.ctx.provider.as_fake().unwrap();
    let caps =
        pf_provider::block_on(core.ctx.provider.get_capabilities("fake-black-edit")).unwrap();
    let refs: DraftReferences = serde_json::from_value(r["references"].clone()).unwrap();
    let prepared = super::prepare_for_send(&core, &caps, Some(&refs)).unwrap();
    assert_eq!(prepared.sent.len(), 3);
    let region = png_of(image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        64,
        64,
        image::Rgb([90, 90, 90]),
    )));
    let mask = png_of(image::DynamicImage::ImageLuma8(image::GrayImage::from_fn(
        64,
        64,
        |x, _| image::Luma([if x < 32 { 255 } else { 0 }]),
    )));
    let request = |references| pf_provider::JobRequest {
        operation: pf_provider::Operation::EditMasked,
        model_id: "fake-black-edit".into(),
        final_prompt: "a mossy flagstone".into(),
        controls: Default::default(),
        references,
        region_png: region.clone(),
        mask_png: mask.clone(),
        candidate_count: 3,
        seed: Some(7),
    };
    let with = pf_provider::block_on(
        core.ctx
            .provider
            .estimate(&request(prepared.provider_references())),
    )
    .unwrap();
    let sent: Vec<(String, String)> = prepared
        .sent
        .iter()
        .map(|s| (s.slot.clone(), s.hash.clone()))
        .collect();
    assert_eq!(fake.last_reference_hashes(), sent);
    let without = pf_provider::block_on(core.ctx.provider.estimate(&request(vec![]))).unwrap();
    let units = |e: &pf_provider::Estimate| match e.cost.amount {
        pf_provider::CostAmount::Known(v) => v,
        pf_provider::CostAmount::Unknown => f64::NAN,
    };
    assert_eq!(
        units(&with) - units(&without),
        2.0,
        "+1 per style ref, identity free"
    );
    assert_eq!(estimate_inputs(&prepared.sent, &caps.slots).len(), 2);
}

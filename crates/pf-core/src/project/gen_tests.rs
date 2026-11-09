//! GEN slice 2 (PRJ side): projects with no image, the `base` layer, decoded `layers[0]`, the
//! pick commit and `draft.generate` ([new-asset-generation](../../../../docs/specs/new-asset-generation.md)).

use serde_json::json;

use super::*;
use crate::context::{FaultAction, Faults};
use crate::core::Core;
use crate::create::{create_empty_project, suggest_name};
use crate::error::ErrorCode;
use crate::library::{self, CardBadge};
use crate::test_support::*;

fn core_with_empty(section: &str, name: &str) -> (tempfile::TempDir, Core, String) {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = crate::context::Context::deterministic(dir.path().join("data"), Timestamp(T0));
    ctx.library_override = Some(dir.path().join("lib"));
    let core = Core::new(ctx).unwrap();
    let out = core
        .dispatch(
            "gen_project_create",
            json!({"section": section, "name": name}),
        )
        .unwrap();
    let path = out["path"].as_str().unwrap().to_owned();
    (dir, core, path)
}

// ---------------------------------------------------------------- FR-GEN-02 creation

#[test]
fn fr_gen_02_project_with_no_image_has_every_folder_null_original_and_r0() {
    let e = env();
    let dir = e.empty_project(Kind::Token, "Goblin archer");
    assert_eq!(dir, e.lib().join("Tokens/Goblin archer"));
    for d in PROJECT_DIRS {
        assert!(dir.join(d).is_dir(), "{d}");
    }
    assert!(
        std::fs::read_dir(dir.join(WORKING_DIR))
            .unwrap()
            .next()
            .is_none(),
        "no image is copied"
    );
    let v: Value =
        serde_json::from_slice(&std::fs::read(dir.join(MANIFEST_FILE)).unwrap()).unwrap();
    assert_eq!(v["kind"], "token");
    assert_eq!(v["original"], Value::Null);
    assert_eq!(v["layers"], json!([]));
    let p = e.open(&dir);
    assert_eq!(p.manifest.revisions.len(), 1);
    let r0 = read_revision(&p, p.manifest.head_revision).unwrap();
    assert_eq!((r0.seq, r0.reason), (0, RevisionReason::Create));
    assert!(r0.layers.is_empty());
    // Nothing temp is left behind.
    assert!(
        std::fs::read_dir(e.lib().join("Tokens"))
            .unwrap()
            .flatten()
            .all(|en| !en.file_name().to_string_lossy().contains(fsx::TEMP_MARKER))
    );
}

#[test]
fn fr_gen_02_creation_is_atomic_temp_folder_then_rename() {
    let mut e = env();
    e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Error).unwrap();
    let err = create_empty_project(&e.ctx, &e.settings, Kind::Map, "Crypt", None).unwrap_err();
    assert_eq!(err.code, ErrorCode::Io);
    assert_eq!(
        tree(&e.lib().join("Maps")),
        Vec::<String>::new(),
        "no partial project under any name"
    );
}

#[test]
fn fr_gen_02_name_rules_invalid_name_and_name_taken() {
    let e = env();
    let bad = create_empty_project(&e.ctx, &e.settings, Kind::Map, "a/b", None).unwrap_err();
    assert_eq!(bad.code, ErrorCode::InvalidName);
    assert_eq!(bad.field.as_deref(), Some("name"));
    e.project("Crypt");
    let taken = create_empty_project(&e.ctx, &e.settings, Kind::Map, "crypt", None).unwrap_err();
    assert_eq!(
        taken.code,
        ErrorCode::NameTaken,
        "case-insensitive (FR-PRJ-44)"
    );
    // Another section is free.
    create_empty_project(&e.ctx, &e.settings, Kind::Item, "Crypt", None).unwrap();
}

#[test]
fn fr_gen_01_prefilled_name_per_section_with_suffix_on_collision() {
    let e = env();
    assert_eq!(
        suggest_name(&e.ctx, &e.settings, Kind::Map, None),
        "New map"
    );
    assert_eq!(
        suggest_name(&e.ctx, &e.settings, Kind::Token, None),
        "New token"
    );
    assert_eq!(
        suggest_name(&e.ctx, &e.settings, Kind::Item, None),
        "New item"
    );
    e.empty_project(Kind::Map, "New map");
    assert_eq!(
        suggest_name(&e.ctx, &e.settings, Kind::Map, None),
        "New map (2)"
    );
    assert_eq!(
        suggest_name(&e.ctx, &e.settings, Kind::Map, Some("Crypt: lower?")),
        "Crypt- lower-"
    );
}

#[test]
fn fr_gen_02_commands_create_and_suggest() {
    let (_d, core, path) = core_with_empty("maps", "Crypt");
    assert!(path.ends_with("Crypt"));
    let again = core
        .dispatch(
            "gen_project_create",
            json!({"section": "map", "name": "Crypt"}),
        )
        .unwrap_err();
    assert_eq!(again.code, ErrorCode::NameTaken);
    let s = core
        .dispatch(
            "gen_project_suggest_name",
            json!({"section": "maps", "name": "Crypt"}),
        )
        .unwrap();
    assert_eq!(s["name"], "Crypt (2)");
    let unknown = core
        .dispatch(
            "gen_project_create",
            json!({"section": "maps", "name": "x", "extra": 1}),
        )
        .unwrap_err();
    assert_eq!(unknown.code, ErrorCode::InvalidInput);
}

// ---------------------------------------------------------------- FR-GEN-03 no image

#[test]
fn fr_gen_03_open_with_no_image_is_writable_and_snapshot_says_so() {
    let e = env();
    let dir = e.empty_project(Kind::Map, "Crypt");
    let p = e.open(&dir);
    assert_eq!(p.read_only, None, "no original to check");
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let s = p.snapshot();
    assert!(!s.has_image);
    assert_eq!((s.image_path, s.original_image_id), (None, None));
    assert_eq!((s.width, s.height, s.format), (0, 0, None));
    assert!(s.layers.is_empty());
    let v = serde_json::to_value(p.snapshot()).unwrap();
    assert_eq!(v["original_image_id"], Value::Null);
    assert_eq!(v["has_image"], false);
}

#[test]
fn fr_gen_03_region_edit_masks_and_export_fail_with_no_image() {
    let (_d, core, path) = core_with_empty("maps", "Crypt");
    let snap = core
        .dispatch("project_open", json!({ "path": path }))
        .unwrap();
    let code = |name: &str, input: Value| core.dispatch(name, input).unwrap_err().code;
    assert_eq!(
        code("project_prefetch_original", json!({})),
        ErrorCode::NoImage
    );
    assert_eq!(
        code(
            "rgn_region_fit",
            json!({"project_id": snap["project_id"], "inner": {"x": 0, "y": 0, "w": 32, "h": 32}})
        ),
        ErrorCode::NoImage
    );
    assert_eq!(
        code(
            "edt_mask_submit",
            json!({"project_id": snap["project_id"], "rect": {"x": 0, "y": 0, "w": 2, "h": 2},
                "data_b64": "/////w=="})
        ),
        ErrorCode::NoImage
    );
    let settings = json!({"settings": {"grid": {"mode": "cells", "cols": 10, "rows": 10},
        "target_px_per_square": 70, "format": "png"}});
    assert_eq!(code("export_plan", settings.clone()), ErrorCode::NoImage);
    assert_eq!(code("export_start", settings.clone()), ErrorCode::NoImage);
    assert_eq!(code("export_set_settings", settings), ErrorCode::NoImage);
    // Store-level: the decoded handle says the same.
    let st = core.state();
    let p = st.project.as_ref().unwrap();
    assert_eq!(p.require_image().unwrap_err().code, ErrorCode::NoImage);
    assert_eq!(
        store::decoded_original(&core.ctx, p).unwrap_err().code,
        ErrorCode::NoImage
    );
}

#[test]
fn fr_gen_03_home_card_has_no_thumbnail_and_a_no_image_badge() {
    let e = env();
    let dir = e.empty_project(Kind::Item, "Sword");
    // A stale thumbnail (e.g. from before undoing the first pick) is not shown.
    std::fs::write(dir.join(THUMB_FILE), b"jpeg").unwrap();
    let cards = library::list_projects(&e.ctx, &e.settings, Kind::Item).unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].name, "Sword");
    assert_eq!(cards[0].thumb, None);
    assert_eq!(cards[0].badges, [CardBadge::NoImage]);
    assert_eq!(cards[0].iterations, 0);
    assert_eq!(
        serde_json::to_value(CardBadge::NoImage).unwrap(),
        "no_image"
    );
}

#[test]
fn fr_gen_03_rebuild_from_history_keeps_original_null() {
    let e = env();
    let dir = e.empty_project(Kind::Map, "Crypt");
    std::fs::remove_file(dir.join(MANIFEST_FILE)).unwrap();
    let p = e.open(&dir);
    assert!(p.warnings.iter().any(|w| w.code == "recovered"));
    assert_eq!(p.manifest.original, None);
    assert!(p.manifest.layers.is_empty());
}

// ---------------------------------------------------------------- FR-GEN-19 draft.generate

fn rgn_draft(prompt: &str) -> DraftInput {
    DraftInput {
        final_prompt: prompt.into(),
        model_id: Some("fake-inpaint".into()),
        ..Default::default()
    }
}

#[test]
fn fr_gen_19_new_image_save_and_region_edit_save_leave_each_other_alone() {
    let e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    put_draft(&e.ctx, e.install(), &mut p, rgn_draft("moss"), None).unwrap();
    let g = json!({"model_id": "fake-gen-wh", "size": {"mode": "grid", "cols": 30, "rows": 40, "pps": 70}});
    put_draft(
        &e.ctx,
        e.install(),
        &mut p,
        DraftInput::generate_only(Some(g.clone())),
        None,
    )
    .unwrap();
    let d = p.manifest.draft.clone().unwrap();
    assert_eq!(d.final_prompt, "moss", "the region-edit draft is untouched");
    assert_eq!(d.model_id.as_deref(), Some("fake-inpaint"));
    assert_eq!(d.generate.as_ref(), Some(&g));
    // A region-edit save (no `generate`) keeps the New image draft.
    put_draft(&e.ctx, e.install(), &mut p, rgn_draft("ivy"), None).unwrap();
    let d = p.manifest.draft.clone().unwrap();
    assert_eq!(
        (d.final_prompt.as_str(), d.generate.as_ref()),
        ("ivy", Some(&g))
    );
    // Durable, and `null` clears it.
    let p2 = e.open(&dir);
    assert_eq!(p2.manifest.draft.unwrap().generate, Some(g));
    put_draft(
        &e.ctx,
        e.install(),
        &mut p,
        DraftInput::generate_only(None),
        None,
    )
    .unwrap();
    let d = p.manifest.draft.clone().unwrap();
    assert_eq!((d.final_prompt.as_str(), d.generate), ("ivy", None));
}

#[test]
fn fr_gen_19_a_locked_new_image_save_survives_a_later_region_edit_save() {
    let mut e = env();
    let dir = e.project("Crypt");
    let mut p = e.open(&dir);
    let g = json!({"model_id": "fake-gen-wh"});
    e.ctx.faults = Faults::parse("after_temp:project.json", FaultAction::Locked).unwrap();
    let err = put_draft(
        &e.ctx,
        e.install(),
        &mut p,
        DraftInput::generate_only(Some(g.clone())),
        None,
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
    e.ctx.faults = Faults::none();
    put_draft(&e.ctx, e.install(), &mut p, rgn_draft("moss"), None).unwrap();
    let d = p.manifest.draft.clone().unwrap();
    assert_eq!(
        (d.final_prompt.as_str(), d.generate.as_ref()),
        ("moss", Some(&g))
    );
    assert!(p.pending_draft.is_none());
}

#[test]
fn fr_gen_19_put_draft_command_takes_generate_alone() {
    let (d, core, path) = core_with_empty("tokens", "Goblin");
    let _ = d;
    core.dispatch("project_open", json!({ "path": path }))
        .unwrap();
    core.dispatch(
        "project_put_draft",
        json!({"generate": {"model_id": "fake-gen-wh", "size": {"mode": "aspect", "aspect": "1:1"}}}),
    )
    .unwrap();
    let snap = core.dispatch("project_get", json!({})).unwrap();
    assert_eq!(snap["draft"]["generate"]["model_id"], "fake-gen-wh");
    assert_eq!(snap["draft"]["final_prompt"], "");
    let bad = core
        .dispatch("project_put_draft", json!({"generate": [1, 2]}))
        .unwrap_err();
    assert_eq!(
        (bad.code, bad.field.as_deref()),
        (ErrorCode::InvalidInput, Some("generate"))
    );
    // A region-edit save keeps it.
    core.dispatch("project_put_draft", json!({"draft": {"final_prompt": "x"}}))
        .unwrap();
    let snap = core.dispatch("project_get", json!({})).unwrap();
    assert_eq!(snap["draft"]["generate"]["model_id"], "fake-gen-wh");
    assert_eq!(snap["draft"]["final_prompt"], "x");
}

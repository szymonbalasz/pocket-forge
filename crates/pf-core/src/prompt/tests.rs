//! PRM slice 1 tests (helpers core): pure compose/dedupe/retry, and the commands over a real
//! data dir with the deterministic context. No provider is involved (FR-PRM-31).

use std::sync::Arc;

use serde_json::{Value, json};

use super::helpers::{self, Helper, STARTER, StoredHelper};
use super::*;
use crate::context::{Context, RecordedEvents};
use crate::core::Core;
use crate::error::ErrorCode;
use crate::settings::settings_path;
use crate::time::Timestamp;

struct Env {
    dir: tempfile::TempDir,
    core: Core,
    events: Arc<RecordedEvents>,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(1_800_000_000_000));
    ctx.library_override = Some(dir.path().join("lib"));
    let events = Arc::new(RecordedEvents::default());
    ctx.events = Arc::new(events.clone());
    let core = Core::new(ctx).unwrap();
    Env { dir, core, events }
}

impl Env {
    fn run(&self, name: &str, input: Value) -> Result<Value, crate::CoreError> {
        self.core.dispatch(name, input)
    }
    fn list(&self) -> Vec<Value> {
        self.run("prm_helpers_list", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .clone()
    }
    fn file(&self) -> Value {
        serde_json::from_slice(
            &std::fs::read(settings_path(&self.dir.path().join("data"))).unwrap(),
        )
        .unwrap()
    }
}

fn user(id: &str, label: &str, text: &str, kinds: &[Kind]) -> Helper {
    Helper {
        id: id.into(),
        label: label.into(),
        text: text.into(),
        kinds: kinds.to_vec(),
        default_on: false,
        builtin: false,
        hidden: false,
        modified: false,
    }
}

fn ids(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn ticks(t: &[&str]) -> Vec<String> {
    t.iter().map(|s| (*s).to_owned()).collect()
}

const U1: &str = "01HZZZZZZZZZZZZZZZZZZZZZZ1";
const U2: &str = "01HZZZZZZZZZZZZZZZZZZZZZZ2";

// ---------------------------------------------------------------- FR-PRM-01

#[test]
fn fr_prm_01_save_validates_fields_ids_and_the_limit() {
    let e = env();
    let mut rows = e.list();
    let check = |rows: &Vec<Value>, code: ErrorCode, field: &str| {
        let err = e
            .run("prm_helpers_save", json!({"helpers": rows}))
            .unwrap_err();
        assert_eq!(err.code, code, "{err:?}");
        assert_eq!(err.field.as_deref(), Some(field), "{err:?}");
    };

    let mut bad = rows.clone();
    bad[0]["label"] = json!("x".repeat(41));
    check(&bad, ErrorCode::InvalidInput, "helpers[0].label");
    bad[0]["label"] = json!("   ");
    check(&bad, ErrorCode::InvalidInput, "helpers[0].label");

    let mut bad = rows.clone();
    bad[1]["text"] = json!("é".repeat(501));
    check(&bad, ErrorCode::InvalidInput, "helpers[1].text");
    bad[1]["text"] = json!("");
    check(&bad, ErrorCode::InvalidInput, "helpers[1].text");

    let mut bad = rows.clone();
    bad[2]["kinds"] = json!([]);
    check(&bad, ErrorCode::InvalidInput, "helpers[2].kinds");

    let mut bad = rows.clone();
    bad.push(json!({"id": "builtin:nope", "label": "a", "text": "b", "kinds": ["map"]}));
    check(
        &bad,
        ErrorCode::InvalidInput,
        &format!("helpers[{}].id", rows.len()),
    );
    bad.pop();
    bad.push(json!({"id": "not-a-ulid", "label": "a", "text": "b", "kinds": ["map"]}));
    check(
        &bad,
        ErrorCode::InvalidInput,
        &format!("helpers[{}].id", rows.len()),
    );

    let mut bad = rows.clone();
    bad.push(bad[0].clone());
    check(
        &bad,
        ErrorCode::InvalidInput,
        &format!("helpers[{}].id", rows.len()),
    );

    // Limits are inclusive: 40 / 500 chars, 60 helpers.
    let mut ok = rows.clone();
    ok[0]["label"] = json!("é".repeat(40));
    ok[0]["text"] = json!("é".repeat(500));
    while ok.len() < helpers::MAX_HELPERS {
        ok.push(json!({"label": "Mine", "text": "Mine.", "kinds": ["item"]}));
    }
    let saved = e.run("prm_helpers_save", json!({"helpers": ok})).unwrap();
    assert_eq!(saved.as_array().unwrap().len(), 60);
    ok.push(json!({"label": "One more", "text": "x", "kinds": ["item"]}));
    check(&ok, ErrorCode::TooMany, "helpers");

    // A failed save leaves the settings file unchanged.
    let before = e.file();
    rows[0]["label"] = json!("");
    assert!(e.run("prm_helpers_save", json!({"helpers": rows})).is_err());
    assert_eq!(e.file(), before);
}

#[test]
fn fr_prm_01_new_helpers_get_a_ulid_and_kinds_are_canonical() {
    let e = env();
    let mut rows = e.list();
    rows.push(json!({"label": " Moody ", "text": " Moody lighting. ", "kinds": ["item", "map", "item"], "default_on": true}));
    let saved = e.run("prm_helpers_save", json!({"helpers": rows})).unwrap();
    let new = saved.as_array().unwrap().last().unwrap();
    assert!(ulid::Ulid::from_string(new["id"].as_str().unwrap()).is_ok());
    assert_eq!(new["label"], "Moody");
    assert_eq!(new["text"], "Moody lighting.");
    assert_eq!(new["kinds"], json!(["map", "item"]));
    assert_eq!(new["builtin"], false);
    assert_eq!(new["default_on"], true);
}

// ---------------------------------------------------------------- FR-PRM-02

#[test]
fn fr_prm_02_absent_section_is_the_starter_set_all_off() {
    let e = env();
    assert!(
        e.file().get("prompt_helpers").is_none(),
        "not written until a save"
    );
    let rows = e.list();
    let got: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    let want: Vec<&str> = STARTER.iter().map(|s| s.id).collect();
    assert_eq!(got, want);
    assert!(rows.iter().all(|r| r["default_on"] == false
        && r["builtin"] == true
        && r["hidden"] == false
        && r["modified"] == false));
    let maps = e.run("prm_helpers_list", json!({"kind": "map"})).unwrap();
    assert_eq!(maps.as_array().unwrap().len(), 4);
    let tokens = e.run("prm_helpers_list", json!({"kind": "token"})).unwrap();
    let t: Vec<&str> = tokens
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        t,
        [
            "builtin:token-single",
            "builtin:token-plain-bg",
            "builtin:no-text"
        ]
    );
}

#[test]
fn fr_prm_02_built_ins_are_stored_as_overrides_only_and_order_is_kept() {
    let e = env();
    let mut rows = e.list();
    rows[2]["text"] = json!("No people at all.");
    rows[3]["hidden"] = json!(true);
    rows.swap(0, 1);
    rows.push(json!({"label": "Mine", "text": "Mine.", "kinds": ["map"]}));
    let saved = e.run("prm_helpers_save", json!({"helpers": rows})).unwrap();
    assert_eq!(saved[0]["id"], "builtin:match-style");
    assert_eq!(saved[2]["modified"], true);
    assert_eq!(saved[3]["hidden"], true);
    assert_eq!(saved[3]["modified"], false, "hiding isn't an edit");

    let stored = &e.file()["prompt_helpers"]["helpers"];
    assert_eq!(stored[0], json!({"id": "builtin:match-style"}));
    assert_eq!(
        stored[2],
        json!({"id": "builtin:no-people", "text": "No people at all."})
    );
    assert_eq!(
        stored[3],
        json!({"id": "builtin:no-grid-text", "hidden": true})
    );
    let mine = &stored[9];
    assert_eq!(mine["label"], "Mine");
    assert_eq!(mine["kinds"], json!(["map"]));
    assert_eq!(mine["default_on"], false);
    assert_eq!(e.events.names(), [EVENT_HELPERS_CHANGED]);
    assert_eq!(
        e.list(),
        saved.as_array().unwrap().clone(),
        "list = what was saved"
    );
}

#[test]
fn fr_prm_02_a_built_in_cant_be_deleted() {
    let e = env();
    let mut rows = e.list();
    rows.remove(4);
    let err = e
        .run("prm_helpers_save", json!({"helpers": rows}))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(err.detail["id"], "builtin:token-single");
}

#[test]
fn fr_prm_02_built_ins_missing_from_the_stored_list_follow_at_the_end() {
    // A section written before a built-in existed (or by hand) still shows every built-in.
    let section = helpers::PromptHelperSettings {
        helpers: vec![
            StoredHelper {
                id: "builtin:no-text".into(),
                ..Default::default()
            },
            StoredHelper {
                id: "builtin:from-the-future".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let list = helpers::resolve(&section).unwrap();
    assert_eq!(list.len(), STARTER.len());
    assert_eq!(list[0].id, "builtin:no-text");
    assert_eq!(list[1].id, "builtin:top-down-map");
}

#[test]
fn fr_prm_02_an_invalid_section_falls_back_to_the_starter_set_with_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let c = Context::deterministic(data.clone(), Timestamp(1_800_000_000_000));
    let id = c.new_id();
    let text = format!(
        r#"{{"schema_version":1,"install_id":"{id}","spend":{{"cap":3.0}},"prompt_helpers":{{"helpers":[{{"id":"{U1}","label":"","text":"x","kinds":["map"]}}]}}}}"#
    );
    std::fs::write(settings_path(&data), &text).unwrap();
    let core = Core::new(c).unwrap();
    let state = core.state();
    assert_eq!(
        state.settings.spend.cap,
        Some(3.0),
        "the rest of the file is kept"
    );
    assert_eq!(state.settings_warnings.len(), 1);
    assert_eq!(state.settings_warnings[0].code, helpers::WARNING_INVALID);
    assert_eq!(helpers::effective(&state.settings), helpers::starter_set());
    drop(state);
    assert_eq!(std::fs::read_to_string(settings_path(&data)).unwrap(), text);
}

#[test]
fn fr_prm_02_settings_put_section_uses_the_same_rules() {
    let e = env();
    let mut state = e.core.state();
    let bad = json!({"helpers": [{"id": "builtin:no-text", "label": ""}]});
    let err = crate::settings::section_put(&e.core.ctx, &mut state.settings, "prompt_helpers", bad)
        .unwrap_err();
    assert_eq!(err.field.as_deref(), Some("helpers[0].label"));
    let good = json!({"helpers": [{"id": "builtin:no-text", "label": "Bare"}]});
    crate::settings::section_put(&e.core.ctx, &mut state.settings, "prompt_helpers", good).unwrap();
    assert_eq!(helpers::effective(&state.settings)[0].label, "Bare");
    crate::settings::section_put(
        &e.core.ctx,
        &mut state.settings,
        "prompt_helpers",
        Value::Null,
    )
    .unwrap();
    assert_eq!(helpers::effective(&state.settings), helpers::starter_set());
}

// ---------------------------------------------------------------- FR-PRM-03

#[test]
fn fr_prm_03_reset_one_built_in_restores_it_in_place_and_unhides_it() {
    let e = env();
    let mut rows = e.list();
    rows[1]["label"] = json!("Style");
    rows[1]["text"] = json!("Worse wording.");
    rows[1]["kinds"] = json!(["map", "token"]);
    rows[1]["default_on"] = json!(true);
    rows[1]["hidden"] = json!(true);
    rows.swap(0, 1);
    e.run("prm_helpers_save", json!({"helpers": rows})).unwrap();

    let out = e
        .run("prm_helpers_reset", json!({"id": "builtin:match-style"}))
        .unwrap();
    let h = &out[0];
    assert_eq!(h["id"], "builtin:match-style", "keeps its position");
    assert_eq!(h["label"], "Match surrounding style");
    assert_eq!(h["text"], STARTER[1].text);
    assert_eq!(h["kinds"], json!(["map"]));
    assert_eq!(h["default_on"], false);
    assert_eq!(h["hidden"], false);
    assert_eq!(h["modified"], false);
    assert_eq!(
        e.file()["prompt_helpers"]["helpers"][0],
        json!({"id": "builtin:match-style"})
    );
}

#[test]
fn fr_prm_03_reset_all_restores_the_starter_set_and_keeps_user_helpers() {
    let e = env();
    let mut rows = e.list();
    rows[0]["text"] = json!("Edited.");
    rows.insert(
        0,
        json!({"label": "Mine", "text": "Mine.", "kinds": ["map"]}),
    );
    rows.swap(3, 5);
    e.run("prm_helpers_save", json!({"helpers": rows})).unwrap();

    let out = e.run("prm_helpers_reset", json!({"all": true})).unwrap();
    let out = out.as_array().unwrap();
    let got: Vec<&str> = out.iter().map(|r| r["id"].as_str().unwrap()).collect();
    let mut want: Vec<&str> = STARTER.iter().map(|s| s.id).collect();
    want.push(got[9]);
    assert_eq!(got, want);
    assert_eq!(out[9]["label"], "Mine");
    assert!(out[..9].iter().all(|r| r["modified"] == false));
}

#[test]
fn fr_prm_03_reset_errors_and_user_helpers_can_be_deleted() {
    let e = env();
    let mut rows = e.list();
    rows.push(json!({"id": U1, "label": "Mine", "text": "Mine.", "kinds": ["map"]}));
    e.run("prm_helpers_save", json!({"helpers": rows})).unwrap();

    let err = e.run("prm_helpers_reset", json!({"id": U1})).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput, "not a built-in");
    let err = e.run("prm_helpers_reset", json!({"id": U2})).unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    let err = e.run("prm_helpers_reset", json!({})).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    let err = e
        .run("prm_helpers_reset", json!({"id": U1, "all": true}))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);

    rows.pop();
    let out = e.run("prm_helpers_save", json!({"helpers": rows})).unwrap();
    assert!(out.as_array().unwrap().iter().all(|r| r["id"] != U1));
}

// ---------------------------------------------------------------- FR-PRM-05

#[test]
fn fr_prm_05_null_ticks_are_the_defaults_for_the_kind() {
    let mut list = helpers::starter_set();
    list[2].default_on = true; // no-people (map)
    list[8].default_on = true; // no-text (token, item)
    list[3].default_on = true; // no-grid-text (map), hidden below
    list[3].hidden = true;
    assert_eq!(
        helpers::ticked_ids(Kind::Map, None, &list),
        ["builtin:no-people"]
    );
    assert_eq!(
        helpers::ticked_ids(Kind::Item, None, &list),
        ["builtin:no-text"]
    );
    assert!(helpers::ticked_ids(Kind::Map, Some(&[]), &list).is_empty());
}

#[test]
fn fr_prm_05_draft_ticks_round_trip_through_the_draft_save() {
    let e = env();
    let src = e.dir.path().join("map.png");
    std::fs::write(&src, crate::test_support::png_bytes(40, 30, 90)).unwrap();
    let out = e
        .run(
            "project_create",
            json!({"section": "maps", "paths": [src.to_string_lossy()]}),
        )
        .unwrap();
    let path = out["items"][0]["path"].as_str().unwrap().to_owned();
    e.run("project_open", json!({"path": path})).unwrap();

    e.run(
        "project_put_draft",
        json!({"draft": {"final_prompt": "a well", "prompt_helpers": ["builtin:no-people"]}}),
    )
    .unwrap();
    let snap = e.run("project_get", json!({})).unwrap();
    assert_eq!(
        snap["draft"]["prompt_helpers"],
        json!(["builtin:no-people"])
    );

    e.run(
        "project_put_draft",
        json!({"draft": {"final_prompt": "a well", "prompt_helpers": null}}),
    )
    .unwrap();
    let snap = e.run("project_get", json!({})).unwrap();
    assert!(
        snap["draft"].get("prompt_helpers").is_none(),
        "null = the defaults, not written"
    );

    let err = e
        .run(
            "project_put_draft",
            json!({"draft": {"final_prompt": "x", "prompt_helpers": [""]}}),
        )
        .unwrap_err();
    assert_eq!(err.field.as_deref(), Some("draft.prompt_helpers"));
}

// ---------------------------------------------------------------- FR-PRM-06

#[test]
fn fr_prm_06_order_separators_and_empty_parts() {
    let list = helpers::starter_set();
    let t = ticks(&["builtin:no-grid-text", "builtin:no-people"]);
    // Helpers in settings (helpers-row) order, not tick order; one space between them.
    let f = compose(
        Kind::Map,
        "a kitchen  \n",
        Some(&t),
        &list,
        Some("  Blend it. "),
    );
    assert_eq!(
        f.text,
        "a kitchen\n\nNo people, creatures or tokens. No grid lines, labels or text.\n\nBlend it."
    );
    let sources: Vec<PartSource> = f.parts.iter().map(|p| p.source).collect();
    assert_eq!(
        sources,
        [
            PartSource::Typed,
            PartSource::Helper,
            PartSource::Helper,
            PartSource::Blend
        ]
    );
    assert_eq!(f.parts[1].id.as_deref(), Some("builtin:no-people"));
    assert_eq!(
        f.sent,
        [
            SentHelper {
                id: "builtin:no-people".into(),
                label: "No people or tokens".into(),
                text: "No people, creatures or tokens.".into()
            },
            SentHelper {
                id: "builtin:no-grid-text".into(),
                label: "No grid or text".into(),
                text: "No grid lines, labels or text.".into()
            }
        ]
    );

    // Empty parts are left out, with no stray separators.
    assert_eq!(
        compose(Kind::Map, "", Some(&t), &list, None).text,
        f.sent
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );
    assert_eq!(
        compose(Kind::Map, "  ", Some(&[]), &list, Some("B.")).text,
        "B."
    );
    assert_eq!(
        compose(Kind::Map, "a", Some(&[]), &list, Some("   ")).text,
        "a"
    );
    assert_eq!(compose(Kind::Map, "", Some(&[]), &list, None).text, "");
    // Leading whitespace of the typed prompt is the user's.
    assert_eq!(compose(Kind::Map, "  a", None, &list, None).text, "  a");
}

#[test]
fn fr_prm_06_unticked_hidden_deleted_and_wrong_kind_helpers_are_never_added() {
    let mut list = helpers::starter_set();
    list[1].hidden = true; // match-style
    let t = ticks(&[
        "builtin:match-style",        // hidden
        "builtin:token-single",       // wrong kind for a map
        "01HZZZZZZZZZZZZZZZZZZZZZZ9", // deleted
        "builtin:no-people",
    ]);
    let f = compose(Kind::Map, "x", Some(&t), &list, None);
    assert_eq!(f.text, "x\n\nNo people, creatures or tokens.");
    assert!(f.skipped.is_empty(), "never added is not 'skipped'");
    // A token sees its own helpers only.
    let f = compose(Kind::Token, "x", Some(&t), &list, None);
    assert_eq!(
        f.text,
        "x\n\nA single creature, centred and fully in frame, seen from above."
    );
}

// ---------------------------------------------------------------- FR-PRM-07

#[test]
fn fr_prm_07_dedupe_against_earlier_helpers() {
    let mut list = helpers::starter_set();
    list.push(user(
        U1,
        "Mine",
        "  no PEOPLE,   creatures or tokens ",
        &[Kind::Map],
    ));
    list.push(user(U2, "Other", "No people", &[Kind::Map]));
    let t = ticks(&["builtin:no-people", U1, U2]);
    let f = compose(
        Kind::Map,
        "No people, creatures or tokens.",
        Some(&t),
        &list,
        None,
    );
    assert_eq!(
        ids(&f.sent.iter().map(|s| s.id.clone()).collect::<Vec<_>>()),
        ["builtin:no-people", U2]
    );
    assert_eq!(
        f.skipped,
        [Skipped {
            id: U1.into(),
            reason: SkipReason::Duplicate
        }]
    );
    // The typed prompt is never deduplicated against.
    assert!(
        f.text
            .starts_with("No people, creatures or tokens.\n\nNo people, creatures or tokens.")
    );
}

#[test]
fn fr_prm_07_a_helper_in_the_blend_instruction_is_skipped_maps_included() {
    let list = helpers::starter_set();
    let rgn = crate::settings::RegionEditSettings::default();
    let blend = crate::region_edit::instruction::instruction_for(&rgn, Kind::Map);
    let t = ticks(&["builtin:top-down-map", "builtin:no-people"]);
    let f = compose(Kind::Map, "a kitchen", Some(&t), &list, Some(&blend));
    assert_eq!(
        f.skipped,
        [Skipped {
            id: "builtin:top-down-map".into(),
            reason: SkipReason::InBlendInstruction
        }]
    );
    assert_eq!(
        f.text,
        format!("a kitchen\n\nNo people, creatures or tokens.\n\n{blend}")
    );
    assert_eq!(f.text.matches("Top-down battlemap view.").count(), 1);

    // Without the blend instruction (off, or GEN) the same helper is sent.
    let f = compose(Kind::Map, "a kitchen", Some(&t), &list, None);
    assert!(f.skipped.is_empty());
    assert_eq!(f.sent[0].id, "builtin:top-down-map");

    // Containment is on word boundaries: "view" is not inside "overview".
    let mut list = list;
    list.push(user(U1, "View", "View.", &[Kind::Map]));
    let f = compose(
        Kind::Map,
        "",
        Some(&ticks(&[U1])),
        &list,
        Some("An overview of it."),
    );
    assert!(f.skipped.is_empty());
}

#[test]
fn fr_prm_07_normalise() {
    assert_eq!(
        normalise("  Top-down   Battlemap\tview. "),
        "top-down battlemap view"
    );
    assert_eq!(normalise("a.."), "a.");
    assert_eq!(normalise("No text"), "no text");
}

// ---------------------------------------------------------------- FR-PRM-08

#[test]
fn fr_prm_08_preview_returns_parts_skipped_and_the_char_count() {
    let e = env();
    let rgn = crate::settings::RegionEditSettings::default();
    let blend = crate::region_edit::instruction::instruction_for(&rgn, Kind::Map);
    let out = e
        .run(
            "prm_compose_preview",
            json!({
                "kind": "map",
                "typed_prompt": "a café",
                "helper_ids": ["builtin:top-down-map", "builtin:no-people"],
                "blend_instruction": blend,
            }),
        )
        .unwrap();
    let text = out["text"].as_str().unwrap();
    assert!(text.starts_with("a café\n\nNo people, creatures or tokens.\n\n"));
    assert_eq!(out["chars"], text.chars().count());
    assert_eq!(
        out["parts"][0],
        json!({"source": "typed", "text": "a café"})
    );
    assert_eq!(out["parts"][1]["source"], "helper");
    assert_eq!(out["parts"][1]["id"], "builtin:no-people");
    assert_eq!(out["parts"][2]["source"], "blend");
    assert_eq!(
        out["skipped"],
        json!([{"id": "builtin:top-down-map", "reason": "in_blend_instruction"}])
    );

    // `null` ticks = the defaults (none: the starter set is all off, #60).
    let out = e
        .run(
            "prm_compose_preview",
            json!({"kind": "token", "typed_prompt": "a wolf", "helper_ids": null, "blend_instruction": null}),
        )
        .unwrap();
    assert_eq!(out["text"], "a wolf");
    assert_eq!(out["chars"], 6);

    let err = e
        .run(
            "prm_compose_preview",
            json!({"kind": "map", "typed_prompt": "x".repeat(20_001)}),
        )
        .unwrap_err();
    assert_eq!(err.field.as_deref(), Some("typed_prompt"));
    let err = e
        .run("prm_compose_preview", json!({"kind": "dragon"}))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
}

// ---------------------------------------------------------------- FR-PRM-09

#[test]
fn fr_prm_09_the_job_records_the_sent_helpers_and_an_old_job_reads_as_empty() {
    let job: crate::job::JobFile = serde_json::from_value(json!({
        "schema_version": 1, "id": ulid::Ulid::from_parts(0, 1),
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
        "state": "requested"
    }))
    .unwrap();
    assert!(job.prompt_helpers().is_empty(), "a job from before PRM");

    let list = helpers::starter_set();
    let t = ticks(&["builtin:top-down-map", "builtin:no-people"]);
    let f = compose(
        Kind::Map,
        "x",
        Some(&t),
        &list,
        Some("Top-down battlemap view."),
    );
    let mut job = job;
    record_on_job(&mut job.extra, &f.sent);
    let file = serde_json::to_value(&job).unwrap();
    assert_eq!(
        file["prompt_helpers"],
        json!([{"id": "builtin:no-people", "label": "No people or tokens", "text": "No people, creatures or tokens."}]),
        "skipped helpers are excluded"
    );
    let back: crate::job::JobFile = serde_json::from_value(file).unwrap();
    assert_eq!(back.prompt_helpers(), f.sent);
}

// ---------------------------------------------------------------- FR-PRM-10

#[test]
fn fr_prm_10_retry_ticks_flags_changed_and_missing() {
    let mut list = helpers::starter_set();
    list[2].text = "No people or monsters.".into(); // no-people, edited since
    list[3].hidden = true; // no-grid-text, hidden since
    list.push(user(U1, "Mine", "Mine.", &[Kind::Map]));
    let sent = |id: &str, text: &str| SentHelper {
        id: id.into(),
        label: "L".into(),
        text: text.into(),
    };
    let recorded = [
        sent("builtin:no-people", "No people, creatures or tokens."),
        sent("builtin:no-grid-text", "No grid lines, labels or text."),
        sent(U1, "Mine."),
        sent(U2, "Deleted since."),
    ];
    let r = retry_ticks(Kind::Map, &recorded, &list);
    assert_eq!(ids(&r.ticks), ["builtin:no-people", U1]);
    assert_eq!(ids(&r.changed), ["builtin:no-people"]);
    assert_eq!(r.missing, [recorded[1].clone(), recorded[3].clone()]);
    // The current text is what a retry sends.
    let f = compose(Kind::Map, "x", Some(&r.ticks), &list, None);
    assert_eq!(f.text, "x\n\nNo people or monsters. Mine.");
    // A job from before PRM: nothing ticked.
    assert_eq!(retry_ticks(Kind::Map, &[], &list), RetryTicks::default());
}

// ---------------------------------------------------------------- FR-PRM-12

#[test]
fn fr_prm_12_settings_editing_needs_no_project() {
    let e = env();
    let rows = e.list();
    assert!(e.run("prm_helpers_save", json!({"helpers": rows})).is_ok());
    assert!(e.run("prm_helpers_reset", json!({"all": true})).is_ok());
}

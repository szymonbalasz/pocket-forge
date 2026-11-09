//! PRM slice 3: prompt helpers in region edit (prompt-helper FR-PRM-05..10, 24; region-edit
//! FR-RGN-09 / FR-RGN-21 as amended by #60). Same lab as [`super::job_tests`].

use serde_json::{Value, json};
use ulid::Ulid;

use super::chat::RgnChatContext;
use super::instruction::DEFAULT_BLEND_INSTRUCTION;
use super::job_tests::Lab;
use crate::error::ErrorCode;
use crate::manifest::Kind;
use crate::prompt::context::{ChatContextSource, ChatHost};

const NO_PEOPLE: &str = "builtin:no-people";
const NO_PEOPLE_TEXT: &str = "No people, creatures or tokens.";
const NO_GRID: &str = "builtin:no-grid-text";
const NO_GRID_TEXT: &str = "No grid lines, labels or text.";
const TOP_DOWN: &str = "builtin:top-down-map";

fn map_instruction() -> String {
    format!("{DEFAULT_BLEND_INSTRUCTION} Top-down battlemap view.")
}

fn ticked(lab: &Lab, prompt: &str, ticks: &[&str]) -> Value {
    let mut d = lab.draft(prompt);
    d["prompt_helpers"] = json!(ticks);
    d
}

/// The current helper list (settings order).
fn helpers(lab: &Lab) -> Vec<Value> {
    lab.core
        .dispatch("prm_helpers_list", json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn save(lab: &Lab, list: Vec<Value>) -> Vec<Value> {
    lab.core
        .dispatch("prm_helpers_save", json!({ "helpers": list }))
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn edit(lab: &Lab, id: &str, f: impl Fn(&mut Value)) {
    let mut list = helpers(lab);
    f(list.iter_mut().find(|h| h["id"] == id).unwrap());
    save(lab, list);
}

#[test]
fn fr_prm_06_fr_rgn_09_submit_sends_typed_then_helpers_then_the_blend_instruction() {
    let lab = Lab::new();
    // Ticked out of settings order: the helpers row (settings) order is what counts.
    let id = lab
        .submit_draft(ticked(&lab, "a stone well  ", &[NO_GRID, NO_PEOPLE]))
        .unwrap();
    let job = lab.job(id);
    let sent = format!(
        "a stone well\n\n{NO_PEOPLE_TEXT} {NO_GRID_TEXT}\n\n{}",
        map_instruction()
    );
    assert_eq!(job.final_prompt, sent);
    assert_eq!(job.provider["request"]["prompt"], json!(sent));
    // #55: the typed prompt is still recorded as typed (Retry, iteration name).
    assert_eq!(job.extra["typed_prompt"], json!("a stone well  "));
    lab.drive();
    assert_eq!(
        lab.job(id).iteration.unwrap().dir,
        "results/001 a stone well"
    );
}

#[test]
fn fr_prm_06_blend_off_sends_typed_and_helpers_only() {
    let lab = Lab::new();
    let mut d = ticked(&lab, "a well", &[NO_PEOPLE]);
    d["blend_instruction"] = json!(false);
    let job = lab.job(lab.submit_draft(d).unwrap());
    assert_eq!(job.final_prompt, format!("a well\n\n{NO_PEOPLE_TEXT}"));
}

#[test]
fn fr_prm_05_no_ticks_means_the_defaults_and_wrong_kind_helpers_are_never_sent() {
    let lab = Lab::new();
    // Starter helpers are all off by default (#60): nothing is added.
    let job = lab.job(lab.submit("a well"));
    assert_eq!(job.final_prompt, format!("a well\n\n{}", map_instruction()));
    assert_eq!(job.extra["prompt_helpers"], json!([]));
    lab.drive();

    edit(&lab, NO_PEOPLE, |h| h["default_on"] = json!(true));
    let job = lab.job(lab.submit("a well"));
    assert!(job.final_prompt.contains(NO_PEOPLE_TEXT), "default on");
    lab.drive();

    // A token helper ticked on a Maps project, an unknown id: not sent.
    let job = lab.job(
        lab.submit_draft(ticked(&lab, "a well", &["builtin:token-single", "nope"]))
            .unwrap(),
    );
    assert_eq!(job.final_prompt, format!("a well\n\n{}", map_instruction()));
}

#[test]
fn fr_prm_07_top_down_helper_is_skipped_while_the_maps_instruction_says_it() {
    let lab = Lab::new();
    let job = lab.job(
        lab.submit_draft(ticked(&lab, "a well", &[TOP_DOWN]))
            .unwrap(),
    );
    assert_eq!(
        job.final_prompt.matches("Top-down battlemap view.").count(),
        1
    );
    assert_eq!(
        job.extra["prompt_helpers"],
        json!([]),
        "skipped, not recorded"
    );
    lab.drive();

    let mut d = ticked(&lab, "a well", &[TOP_DOWN]);
    d["blend_instruction"] = json!(false);
    let job = lab.job(lab.submit_draft(d).unwrap());
    assert_eq!(job.final_prompt, "a well\n\nTop-down battlemap view.");
}

#[test]
fn fr_prm_09_the_job_records_the_helpers_sent_in_order() {
    let lab = Lab::new();
    let id = lab
        .submit_draft(ticked(&lab, "a well", &[NO_GRID, NO_PEOPLE]))
        .unwrap();
    assert_eq!(
        lab.job(id).extra["prompt_helpers"],
        json!([
            {"id": NO_PEOPLE, "label": "No people or tokens", "text": NO_PEOPLE_TEXT},
            {"id": NO_GRID, "label": "No grid or text", "text": NO_GRID_TEXT},
        ])
    );
    lab.drive();
    // Written with `requested`, never changed: a later helper edit leaves the job as it was.
    edit(&lab, NO_PEOPLE, |h| h["text"] = json!("Empty rooms."));
    assert_eq!(lab.job(id).prompt_helpers()[0].text, NO_PEOPLE_TEXT);
}

#[test]
fn fr_prm_08_helpers_never_change_the_estimate_id() {
    let lab = Lab::new();
    let a = lab.estimate(&lab.draft("a well"));
    let b = lab.estimate(&ticked(&lab, "a well", &[NO_PEOPLE, NO_GRID]));
    assert_eq!(a["estimate_id"], b["estimate_id"]);
}

#[test]
fn fr_prm_05_invalid_ticks_are_refused_at_the_boundary() {
    let lab = Lab::new();
    let mut d = lab.draft("a well");
    d["prompt_helpers"] = json!([""]);
    let e = lab
        .core
        .dispatch("rgn_job_estimate", json!({ "draft": d }))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidInput);
    assert_eq!(e.field.as_deref(), Some("draft.prompt_helpers"));
}

#[test]
fn fr_prm_10_fr_rgn_21_retry_restores_ticks_and_badges_changed_and_missing() {
    let lab = Lab::new();
    let mut list = helpers(&lab);
    list.push(json!({"label": "Lanterns", "text": "Lit by lanterns.", "kinds": ["map"]}));
    let user = save(&lab, list)
        .iter()
        .find(|h| h["label"] == "Lanterns")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let id = lab
        .submit_draft(ticked(&lab, "a well", &[NO_PEOPLE, NO_GRID, &user]))
        .unwrap();
    lab.drive();

    // A plain retry: the job's ticks, nothing to badge.
    let r = lab
        .core
        .dispatch("rgn_job_retry_draft", json!({ "job_id": id }))
        .unwrap();
    assert_eq!(r["prompt_helpers"], json!([NO_PEOPLE, NO_GRID, user]));
    assert!(r.get("prompt_retry").is_none());

    // Edit one, hide one, delete the user helper.
    let mut list = helpers(&lab);
    list.retain(|h| h["id"] != user.as_str());
    for h in &mut list {
        if h["id"] == NO_PEOPLE {
            h["text"] = json!("Empty rooms.");
        }
        if h["id"] == NO_GRID {
            h["hidden"] = json!(true);
        }
    }
    save(&lab, list);
    let r = lab
        .core
        .dispatch("rgn_job_retry_draft", json!({ "job_id": id }))
        .unwrap();
    assert_eq!(r["prompt_helpers"], json!([NO_PEOPLE]));
    assert_eq!(
        r["prompt_retry"],
        json!({
            "since": "001",
            "changed": [NO_PEOPLE],
            "missing": [
                {"id": NO_GRID, "label": "No grid or text", "text": NO_GRID_TEXT},
                {"id": user, "label": "Lanterns", "text": "Lit by lanterns."},
            ],
        })
    );
    // The current text is sent.
    let mut d = r.clone();
    d.as_object_mut().unwrap().remove("prompt_retry");
    let job = lab.job(lab.submit_draft(d).unwrap());
    assert!(job.final_prompt.contains("Empty rooms."));
    assert!(!job.final_prompt.contains("Lit by lanterns."));
}

#[test]
fn fr_prm_09_10_a_job_from_before_prm_reads_as_no_helpers() {
    let lab = Lab::new();
    let id = lab.run("a well");
    // Strip the field, as a pre-PRM job file has none.
    let path = lab
        .project_dir
        .join(crate::manifest::job_dir(id))
        .join(crate::job::JOB_FILE);
    let mut v: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    v.as_object_mut().unwrap().remove("prompt_helpers");
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    lab.open();
    let r = lab
        .core
        .dispatch("rgn_job_retry_draft", json!({ "job_id": id }))
        .unwrap();
    assert_eq!(r["prompt_helpers"], json!([]));
    assert!(r.get("prompt_retry").is_none());
}

#[test]
fn fr_prm_24_41_rgn_chat_context_stub_reports_the_text_side_of_the_job() {
    let lab = Lab::new();
    let pid: Ulid = lab.open()["project_id"].as_str().unwrap().parse().unwrap();
    let mut d = ticked(&lab, "a well", &[NO_PEOPLE]);
    d["controls"] = json!({"num_samples": 2});
    let ctx = RgnChatContext.build(&lab.core, pid, &d).unwrap();
    assert_eq!(RgnChatContext.host(), ChatHost::Rgn);
    assert_eq!(ctx.host, ChatHost::Rgn);
    assert_eq!(ctx.kind, Kind::Map);
    // PRM slice 5: the overlay (FR-PRM-24).
    assert_eq!(ctx.images.len(), 1);
    assert_eq!(ctx.images[0].id, "rgn:overlay");
    assert_eq!(ctx.typed_prompt, "a well");
    assert_eq!(ctx.helpers.len(), 1);
    assert_eq!(ctx.helpers[0].id, NO_PEOPLE);
    let blend = ctx.blend_instruction.as_ref().unwrap();
    assert!(blend.on);
    assert_eq!(blend.text, map_instruction());
    assert_eq!(ctx.model.id, "fake-white-edit");
    assert_eq!(ctx.controls[0].name, "num_samples");
    assert_eq!(ctx.controls[0].kind, "number");
    assert!(ctx.facts[0].starts_with("region "));

    // The final prompt is exactly what submit sends.
    let job = lab.job(lab.submit_draft(d.clone()).unwrap());
    assert_eq!(ctx.final_prompt, job.final_prompt);

    let other = RgnChatContext
        .build(&lab.core, Ulid::nil(), &d)
        .unwrap_err();
    assert_eq!(other.code, ErrorCode::NoProject);
    let bad = RgnChatContext
        .build(&lab.core, pid, &json!({"region": 1}))
        .unwrap_err();
    assert_eq!(bad.code, ErrorCode::InvalidInput);
}

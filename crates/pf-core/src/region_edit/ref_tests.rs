//! REF slice 6 / RGN slice 7 integration tests (FR-RGN-41–45, FR-REF-20 … 27): references in
//! estimate, submit, the job record, Retry and the tracker. pf-core + a real asset project on
//! disk + the **fake provider**; generated swatches only (KQ-22). No real provider, no spend.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use ulid::Ulid;

use super::job_tests::Lab;
use crate::context::{FakeFsAttributes, FsAttributes};
use crate::error::ErrorCode;
use crate::job::JobState;
use crate::test_support::png_bytes;

/// The fake fs-attributes seam, shared with the test (cloud-only files, offline).
struct SharedFs(Arc<FakeFsAttributes>);

impl FsAttributes for SharedFs {
    fn is_cloud_only(&self, path: &Path) -> bool {
        self.0.is_cloud_only(path)
    }
    fn hydrate(&self, path: &Path) -> std::io::Result<()> {
        self.0.hydrate(path)
    }
}

fn swatch(shade: u8) -> Vec<u8> {
    png_bytes(32, 24, shade)
}

/// A lab whose project has `references/` files, with the fs seam exposed.
fn lab_with(files: &[(&str, u8)]) -> (Lab, Arc<FakeFsAttributes>) {
    let mut lab = Lab::new();
    let fs = Arc::new(FakeFsAttributes::default());
    // The context is shared (`Arc`) with background threads; none run in these tests.
    std::sync::Arc::get_mut(&mut lab.core.ctx)
        .expect("the context is not shared")
        .fs = Box::new(SharedFs(fs.clone()));
    for (f, shade) in files {
        std::fs::write(lab.project_dir.join("references").join(f), swatch(*shade)).unwrap();
    }
    (lab, fs)
}

/// A pack made through the command, from files written outside the library.
fn make_pack(lab: &Lab, name: &str, files: &[(&str, u8)]) -> Ulid {
    let paths: Vec<String> = files
        .iter()
        .map(|(f, shade)| {
            let p = lab.dir.path().join("outside").join(name).join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, swatch(*shade)).unwrap();
            p.to_string_lossy().into_owned()
        })
        .collect();
    let out = lab
        .core
        .dispatch("ref_pack_create", json!({"name": name, "paths": paths}))
        .unwrap();
    out["pack"]["pack_id"].as_str().unwrap().parse().unwrap()
}

fn pack_dir(lab: &Lab, name: &str) -> PathBuf {
    lab.dir.path().join("lib").join("References").join(name)
}

fn draft(lab: &Lab, model: &str, refs: Value) -> Value {
    let mut d = lab.draft("kitchen");
    d["model_id"] = json!(model);
    d["references"] = refs;
    d
}

fn resolve(lab: &Lab, model: &str, refs: &Value) -> Value {
    lab.core
        .dispatch(
            "ref_resolve",
            json!({"model_id": model, "references": refs}),
        )
        .unwrap()
}

fn sha(bytes: &[u8]) -> String {
    crate::references::prepare::sha256(bytes)
}

fn retry(lab: &Lab, job: Ulid) -> Value {
    lab.core
        .dispatch("rgn_job_retry_draft", json!({"job_id": job}))
        .unwrap()
}

fn item<'a>(r: &'a Value, key: &str) -> &'a Value {
    r["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == key)
        .unwrap_or_else(|| panic!("no item {key} in {r}"))
}

fn on_keys(r: &Value) -> Vec<String> {
    r["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["on"] == true)
        .map(|i| i["key"].as_str().unwrap().to_owned())
        .collect()
}

// ---------------------------------------------------------------- estimate and submit

#[test]
fn fr_rgn_41_estimate_and_submit_send_the_prepared_references() {
    let (lab, _) = lab_with(&[("a.png", 10), ("b.png", 20)]);
    let d = draft(&lab, "fake-white-edit", Value::Null);
    let est = lab.estimate(&d);
    assert_eq!(est["references"], json!(2), "REF's defaults: both ticked");
    let want = vec![
        ("style_refs".to_owned(), sha(&swatch(10))),
        ("style_refs".to_owned(), sha(&swatch(20))),
    ];
    assert_eq!(lab.fake().last_reference_hashes(), want, "estimate");
    let id = lab.submit_draft(d).unwrap();
    assert_eq!(lab.fake().last_reference_hashes(), want, "submit");
    // FR-RGN-43 / FR-REF-25: the job record lists each file sent.
    let job = lab.job(id);
    assert_eq!(job.references.len(), 2);
    let r = &job.references[0];
    assert_eq!(r["source"], "project");
    assert_eq!(r["path"], "references/a.png");
    assert_eq!(r["content_hash"], json!(sha(&swatch(10))));
    assert_eq!(r["slot"], "style_refs");
    assert_eq!(r["role"], "style");
    assert_eq!(r["pack_id"], Value::Null);
    assert_eq!(r["sent"]["w"], json!(32));
    assert_eq!(r["sent"]["resized"], json!(false));
    assert_eq!(r["sent"]["hash"], r["content_hash"], "sent as-is");
}

#[test]
fn fr_rgn_41_untick_and_pick_a_pack_reach_the_provider() {
    let (lab, _) = lab_with(&[("a.png", 10), ("b.png", 20)]);
    let pack = make_pack(&lab, "Swords", &[("s1.png", 30), ("s2.png", 40)]);
    let refs = json!({
        "packs": [{"pack_id": pack, "name": "Swords"}],
        "items": [
            {"key": "project:a.png", "on": false, "role": null},
            {"key": "project:b.png", "on": true, "role": null},
            {"key": format!("pack:{pack}:s1.png"), "on": false, "role": null},
            {"key": format!("pack:{pack}:s2.png"), "on": true, "role": null},
        ],
    });
    let id = lab
        .submit_draft(draft(&lab, "fake-white-edit", refs))
        .unwrap();
    let hashes: Vec<String> = lab
        .fake()
        .last_reference_hashes()
        .into_iter()
        .map(|(_, h)| h)
        .collect();
    assert_eq!(hashes, [sha(&swatch(20)), sha(&swatch(40))]);
    let job = lab.job(id);
    assert_eq!(job.references[1]["source"], "pack");
    assert_eq!(job.references[1]["pack_id"], json!(pack));
    assert_eq!(job.references[1]["pack_name"], "Swords");
    assert_eq!(job.references[1]["path"], "References/Swords/s2.png");
}

#[test]
fn fr_rgn_43_references_are_in_the_requested_commit_before_submit() {
    let (lab, _) = lab_with(&[("a.png", 10)]);
    lab.set_case("submit_failure");
    let r = lab.submit_draft(draft(&lab, "fake-white-edit", Value::Null));
    let err = r.unwrap_err();
    let job_id: Ulid = err.detail["job_id"].as_str().unwrap().parse().unwrap();
    let job = lab.job(job_id);
    assert_eq!(
        job.state,
        JobState::Failed,
        "the provider refused the submit"
    );
    assert_eq!(
        job.references.len(),
        1,
        "written with `requested`, before the provider call"
    );
    assert_eq!(job.references[0]["path"], "references/a.png");
}

#[test]
fn fr_rgn_41_reference_errors_fail_before_any_provider_call() {
    let (lab, fs) = lab_with(&[("a.png", 10), ("b.png", 20), ("c.png", 30), ("d.png", 40)]);
    // A draft ticking more than the slot's max (FR-REF-18): the reconcile inside prepare keeps
    // the first `max` and never sends more.
    let items: Vec<Value> = ["a", "b", "c", "d"]
        .iter()
        .map(|f| json!({"key": format!("project:{f}.png"), "on": true, "role": null}))
        .collect();
    let d = draft(
        &lab,
        "fake-white-edit",
        json!({"packs": [], "items": items}),
    );
    let r = lab.core.dispatch("rgn_job_estimate", json!({ "draft": d }));
    assert_eq!(r.unwrap()["references"], json!(3));
    assert_eq!(lab.fake().last_reference_hashes().len(), 3);

    // Offline and cloud-only (REF §11): reference_unavailable(not_available_offline), no call.
    let before = lab.fake().last_reference_hashes();
    let c = lab.project_dir.join("references").join("c.png");
    fs.cloud_only.lock().unwrap().insert(c);
    fs.offline.store(true, Ordering::Relaxed);
    let only_c =
        json!({"packs": [], "items": [{"key": "project:c.png", "on": true, "role": null}]});
    let d = draft(&lab, "fake-white-edit", only_c);
    let err = lab
        .core
        .dispatch("rgn_job_estimate", json!({ "draft": d }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ReferenceUnavailable);
    assert_eq!(err.detail["reason"], "not_available_offline");
    assert_eq!(
        lab.fake().last_reference_hashes(),
        before,
        "no provider call"
    );
    let est_err = lab
        .core
        .dispatch(
            "rgn_job_submit",
            json!({"draft": d, "estimate_id": "est-x", "confirmations": []}),
        )
        .unwrap_err();
    assert_eq!(est_err.code, ErrorCode::ReferenceUnavailable);
    assert!(
        lab.manifest()["jobs"].as_array().unwrap().is_empty(),
        "no job recorded"
    );
    fs.offline.store(false, Ordering::Relaxed);
}

/// Walkthrough R1: a ticked item deleted or moved after the estimate is never dropped
/// silently. Estimate and submit fail with `reference_unavailable(missing)` before any
/// provider call; no job, no spend.
#[test]
fn fr_ref_21_a_ticked_reference_gone_before_submit_fails_without_spend() {
    let (lab, _) = lab_with(&[("a.png", 10), ("b.png", 20)]);
    let pack = make_pack(&lab, "Blades", &[("s1.png", 30)]);
    let refs = json!({
        "packs": [{"pack_id": pack, "name": "Blades"}],
        "items": [
            {"key": "project:a.png", "on": true, "role": null},
            {"key": "project:b.png", "on": true, "role": null},
            {"key": format!("pack:{pack}:s1.png"), "on": true, "role": null},
        ],
    });
    let d = draft(&lab, "fake-white-edit", refs);
    let est = lab.estimate(&d);
    assert_eq!(est["references"], json!(3));
    let spend = lab.core.dispatch("rgn_spend_get", json!({})).unwrap()["spend"].clone();
    let calls = lab.fake().last_reference_hashes();
    let submit = |d: &Value| {
        lab.core
            .dispatch(
                "rgn_job_submit",
                json!({"draft": d, "estimate_id": est["estimate_id"], "confirmations": est["needs"]}),
            )
            .unwrap_err()
    };
    let no_spend = |what: &str| {
        assert!(
            lab.manifest()["jobs"].as_array().unwrap().is_empty(),
            "{what}: no job recorded"
        );
        assert_eq!(
            lab.core.dispatch("rgn_spend_get", json!({})).unwrap()["spend"],
            spend,
            "{what}: no spend"
        );
        assert_eq!(lab.fake().last_reference_hashes(), calls, "{what}: no provider call");
    };

    // Moved out of the pack in Explorer.
    let moved = lab.dir.path().join("moved-s1.png");
    std::fs::rename(pack_dir(&lab, "Blades").join("s1.png"), &moved).unwrap();
    let err = submit(&d);
    assert_eq!(err.code, ErrorCode::ReferenceUnavailable);
    assert_eq!(
        err.detail,
        json!({"key": format!("pack:{pack}:s1.png"), "reason": "missing"})
    );
    no_spend("moved");
    let err = lab
        .core
        .dispatch("rgn_job_estimate", json!({ "draft": d }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ReferenceUnavailable, "estimate too");
    no_spend("estimate");

    // Deleted from the project's references/.
    std::fs::rename(&moved, pack_dir(&lab, "Blades").join("s1.png")).unwrap();
    std::fs::remove_file(lab.project_dir.join("references").join("b.png")).unwrap();
    let err = submit(&d);
    assert_eq!(
        err.detail,
        json!({"key": "project:b.png", "reason": "missing"})
    );
    no_spend("deleted");

    // Now corrupt: it can't be ticked any more, and is not dropped silently either.
    std::fs::write(lab.project_dir.join("references").join("a.png"), b"not a png").unwrap();
    std::fs::write(lab.project_dir.join("references").join("b.png"), swatch(20)).unwrap();
    let err = submit(&d);
    assert_eq!(
        err.detail,
        json!({"key": "project:a.png", "reason": "corrupt"})
    );
    no_spend("corrupt");

    // The block shows it, and its reconciled draft (what the UI saves) submits cleanly.
    let r = resolve(&lab, "fake-white-edit", &d["references"]);
    assert_eq!(item(&r, "project:a.png")["badges"], json!(["corrupt"]));
    let id = lab
        .submit_draft(draft(&lab, "fake-white-edit", r["references"].clone()))
        .unwrap();
    assert_eq!(lab.job(id).references.len(), 2);
    lab.drive();

    // A deleted pack is shown (`pack_unavailable`, items `missing`, off) and its ticks are kept
    // for its return; it doesn't block the send.
    let r = resolve(&lab, "fake-white-edit", &d["references"]);
    lab.core
        .dispatch("ref_pack_delete", json!({"pack_id": pack}))
        .unwrap();
    let r = resolve(&lab, "fake-white-edit", &r["references"]);
    assert!(r["notices"].as_array().unwrap().iter().any(|n| n["code"] == "pack_unavailable"));
    let id = lab
        .submit_draft(draft(&lab, "fake-white-edit", r["references"].clone()))
        .unwrap();
    assert_eq!(lab.job(id).references.len(), 1);
}

#[test]
fn fr_rgn_41_a_model_without_slots_sends_nothing() {
    let (lab, _) = lab_with(&[("a.png", 10)]);
    let est = lab.estimate(&draft(&lab, "fake-alpha-edit", Value::Null));
    assert_eq!(est["references"], json!(0));
    assert!(lab.fake().last_reference_hashes().is_empty());
    let r = resolve(&lab, "fake-alpha-edit", &Value::Null);
    assert_eq!(r["visible"], json!(false), "FR-REF-15: block hidden");
}

// ---------------------------------------------------------------- cost (FR-REF-24, FR-RGN-42)

#[test]
fn fr_ref_24_rgn_42_only_cost_affecting_slots_change_the_estimate() {
    let (lab, _) = lab_with(&[("a.png", 10), ("b.png", 20), ("c.png", 30)]);
    let ident = |on_c: bool, on_b: bool| {
        json!({"packs": [], "items": [
            {"key": "project:a.png", "on": true, "role": "style"},
            {"key": "project:b.png", "on": on_b, "role": "style"},
            {"key": "project:c.png", "on": on_c, "role": "identity"},
        ]})
    };
    let est = |refs: Value| lab.estimate(&draft(&lab, "fake-black-edit", refs));
    let base = est(ident(false, false));
    // The identity slot isn't cost-affecting: same id, same cost.
    let with_identity = est(ident(true, false));
    assert_eq!(with_identity["estimate_id"], base["estimate_id"]);
    assert_eq!(with_identity["cost"], base["cost"]);
    // A second style ref (cost-affecting, +1 unit on the fake): new id and cost.
    let two_styles = est(ident(false, true));
    assert_ne!(two_styles["estimate_id"], base["estimate_id"]);
    assert_ne!(two_styles["cost"], base["cost"]);
    // Editing a sent style file in place makes the estimate stale (its hash is in the id).
    let d = draft(&lab, "fake-black-edit", ident(false, true));
    let est_id = two_styles["estimate_id"].clone();
    std::fs::write(lab.project_dir.join("references/b.png"), swatch(99)).unwrap();
    let err = lab
        .core
        .dispatch(
            "rgn_job_submit",
            json!({"draft": d, "estimate_id": est_id, "confirmations": ["threshold", "unknown_cost"]}),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::EstimateStale);
}

// ---------------------------------------------------------------- Retry (FR-REF-26, FR-RGN-44)

#[test]
fn fr_ref_26_rgn_44_retry_restores_the_references_and_flags_changes() {
    let (lab, _) = lab_with(&[("a.png", 10), ("b.png", 20)]);
    let pack = make_pack(&lab, "Swords", &[("s1.png", 30), ("s2.png", 40)]);
    // Sent: b (project) and s1 (pack); a and s2 unticked.
    let refs = json!({
        "packs": [{"pack_id": pack, "name": "Swords"}],
        "items": [
            {"key": "project:a.png", "on": false, "role": null},
            {"key": "project:b.png", "on": true, "role": null},
            {"key": format!("pack:{pack}:s1.png"), "on": true, "role": null},
            {"key": format!("pack:{pack}:s2.png"), "on": false, "role": null},
        ],
    });
    let id = lab
        .submit_draft(draft(&lab, "fake-white-edit", refs))
        .unwrap();
    lab.drive();
    assert_eq!(lab.job(id).iteration.as_ref().unwrap().n, 1);

    // Unchanged: the same items come back ticked, the rest off, no badges.
    let d = retry(&lab, id);
    let r = resolve(&lab, "fake-white-edit", &d["references"]);
    let s1 = format!("pack:{pack}:s1.png");
    assert_eq!(on_keys(&r), ["project:b.png".to_owned(), s1.clone()]);
    assert_eq!(
        item(&r, &format!("pack:{pack}:s2.png"))["on"],
        false,
        "every other item off"
    );
    assert!(item(&r, &s1)["badges"].as_array().unwrap().is_empty());

    // Repaint s1 in Explorer, rename the pack: found by id, still on, flagged.
    std::fs::write(pack_dir(&lab, "Swords").join("s1.png"), swatch(77)).unwrap();
    lab.core
        .dispatch(
            "ref_pack_rename",
            json!({"pack_id": pack, "name": "Blades"}),
        )
        .unwrap();
    let d = retry(&lab, id);
    assert_eq!(d["references"]["packs"][0]["name"], "Blades");
    let r = resolve(&lab, "fake-white-edit", &d["references"]);
    assert_eq!(item(&r, &s1)["on"], true, "changed but still on");
    assert_eq!(item(&r, &s1)["badges"], json!(["changed_since:001"]));
    assert_eq!(item(&r, "project:b.png")["badges"], json!([]));
    // The current bytes are sent (the estimate is reused: this slot isn't cost-affecting).
    let again = lab
        .submit_draft(draft(&lab, "fake-white-edit", r["references"].clone()))
        .unwrap();
    assert_eq!(lab.fake().last_reference_hashes()[1].1, sha(&swatch(77)));
    let row = &lab.job(again).references[1];
    assert_eq!(row["path"], "References/Blades/s1.png");
    assert_eq!(row["content_hash"], json!(sha(&swatch(77))));
    lab.drive();

    // Delete the project file: missing, off, kept for the Retry.
    std::fs::remove_file(lab.project_dir.join("references/b.png")).unwrap();
    let d = retry(&lab, id);
    let r = resolve(&lab, "fake-white-edit", &d["references"]);
    let b = item(&r, "project:b.png");
    assert_eq!(
        (b["on"].clone(), b["state"].clone()),
        (json!(false), json!("missing"))
    );
    assert_eq!(b["badges"], json!(["missing"]));
    assert_eq!(on_keys(&r), [s1]);
}

#[test]
fn fr_ref_26_retry_after_the_pack_is_deleted_and_restored() {
    let (lab, _) = lab_with(&[]);
    let pack = make_pack(&lab, "Blades", &[("s1.png", 30)]);
    let refs = json!({"packs": [{"pack_id": pack, "name": "Blades"}], "items": []});
    let id = lab
        .submit_draft(draft(&lab, "fake-white-edit", refs))
        .unwrap();
    lab.drive();
    let trash = lab
        .core
        .dispatch("ref_pack_delete", json!({"pack_id": pack}))
        .unwrap();
    let d = retry(&lab, id);
    let r = resolve(&lab, "fake-white-edit", &d["references"]);
    let key = format!("pack:{pack}:s1.png");
    assert_eq!(item(&r, &key)["state"], "missing");
    assert_eq!(item(&r, &key)["on"], false);
    assert!(
        r["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["code"] == "pack_unavailable")
    );
    // Restored: it resolves again (the draft still names it).
    let trash_id = trash["item_id"].clone();
    lab.core
        .dispatch("ref_trash_restore", json!({"id": trash_id}))
        .unwrap();
    let r = resolve(&lab, "fake-white-edit", &r["references"]);
    assert_eq!(item(&r, &key)["state"], "ok");
}

#[test]
fn fr_rgn_44_retry_without_references_keeps_the_block_defaults_off() {
    let (lab, _) = lab_with(&[]);
    let id = lab.run("kitchen");
    // The model has slots but nothing was sent: Retry sends nothing either.
    std::fs::write(lab.project_dir.join("references/new.png"), swatch(5)).unwrap();
    let d = retry(&lab, id);
    let r = resolve(&lab, "fake-white-edit", &d["references"]);
    assert!(on_keys(&r).is_empty(), "{r}");
    // A model without slots: no selection at all.
    let id = lab
        .submit_draft(draft(&lab, "fake-alpha-edit", Value::Null))
        .unwrap();
    lab.drive();
    assert_eq!(retry(&lab, id)["references"], Value::Null);
}

// ---------------------------------------------------------------- draft, read-only, tracker

#[test]
fn fr_ref_20_the_selection_is_saved_with_the_draft() {
    let (lab, _) = lab_with(&[("a.png", 10)]);
    let refs = json!({"packs": [], "items": [{"key": "project:a.png", "on": false, "role": null}]});
    lab.core
        .dispatch(
            "project_put_draft",
            json!({"draft": {"final_prompt": "x", "model_id": "fake-white-edit", "references": refs}}),
        )
        .unwrap();
    lab.close();
    let snap = lab.open();
    assert_eq!(
        snap["draft"]["references"], refs,
        "survives close and reopen"
    );
}

#[test]
fn fr_ref_27_read_only_shows_the_block_and_refuses_estimates() {
    let (lab, _) = lab_with(&[("a.png", 10)]);
    lab.core.state().project.as_mut().unwrap().read_only =
        Some(crate::project::ReadOnlyReason::OriginalChanged);
    let r = resolve(&lab, "fake-white-edit", &Value::Null);
    assert_eq!(
        (r["visible"].clone(), r["read_only"].clone()),
        (json!(true), json!(true))
    );
    let err = lab
        .core
        .dispatch(
            "rgn_job_estimate",
            json!({"draft": draft(&lab, "fake-white-edit", Value::Null)}),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ReadOnly);
    assert!(lab.fake().last_reference_hashes().is_empty());
}

#[test]
fn fr_rgn_45_the_tracker_lists_the_references_sent() {
    let (lab, _) = lab_with(&[("a.png", 10)]);
    let pack = make_pack(&lab, "Swords", &[("s1.png", 30)]);
    let refs = json!({"packs": [{"pack_id": pack, "name": "Swords"}], "items": []});
    lab.submit_draft(draft(&lab, "fake-black-edit", refs))
        .unwrap();
    lab.drive();
    let rows = lab.core.dispatch("rgn_iterations_list", json!({})).unwrap();
    assert_eq!(rows[0]["references"], json!(2));
    assert_eq!(
        rows[0]["reference_files"],
        json!([
            {"file": "a.png", "pack_name": null, "role": "style"},
            {"file": "s1.png", "pack_name": "Swords", "role": "style"},
        ])
    );
}

#[test]
fn fr_ref_17_r53_default_roles_follow_the_model_chosen_roles_stay() {
    // Appendix A steps 5–6: a pack image marked identity, picked under fake-white-edit (one
    // style slot), defaults to identity once fake-black-edit (style + identity) is picked.
    let (lab, _) = lab_with(&[]);
    let pack = make_pack(&lab, "Blades", &[("face.png", 30), ("s1.png", 40)]);
    lab.core
        .dispatch(
            "ref_pack_update",
            json!({"pack_id": pack, "roles": {"face.png": "identity"}}),
        )
        .unwrap();
    let picked = json!({"packs": [{"pack_id": pack, "name": "Blades"}], "items": []});
    let white = resolve(&lab, "fake-white-edit", &picked);
    let face = format!("pack:{pack}:face.png");
    assert_eq!(
        item(&white, &face)["role"],
        "style",
        "white has only a style slot"
    );
    // The draft saves no default role.
    let saved = white["references"].clone();
    assert!(
        saved["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["role"].is_null())
    );
    let black = resolve(&lab, "fake-black-edit", &saved);
    assert_eq!(item(&black, &face)["role"], "identity");
    assert_eq!(item(&black, &face)["slot"], "identity_ref");
    // A role the user chose is kept across the model change.
    let mut chosen = saved.clone();
    for i in chosen["items"].as_array_mut().unwrap() {
        if i["key"] == json!(face) {
            i["role"] = json!("style");
        }
    }
    let white = resolve(&lab, "fake-white-edit", &chosen);
    let black = resolve(&lab, "fake-black-edit", &white["references"]);
    assert_eq!(item(&black, &face)["role"], "style");
}

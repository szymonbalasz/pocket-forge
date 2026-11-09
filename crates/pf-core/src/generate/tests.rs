//! GEN slice 3 tests (`fr_gen_NN_*`): the size fit (pure), and pf-core + a real asset project
//! on disk + the **fake provider** (`fake-gen-wh`, `fake-gen-aspect`) for controls, estimate,
//! submit through RGN's runner, the job record, the sketch and Retry. Generated images only
//! (KQ-22). No real provider, no spend.

use std::path::PathBuf;

use pf_provider::OutputSize;
use serde_json::{Value, json};
use ulid::Ulid;

use super::size::{GenSize, fit};
use crate::error::{CoreError, ErrorCode};
use crate::job::{JOB_FILE, JobState};
use crate::manifest::job_dir;
use crate::project::ReadOnlyReason;
use crate::region_edit::job_tests::Lab;
use crate::test_support::png_bytes;

const WH: &str = "fake-gen-wh";
const ASPECT: &str = "fake-gen-aspect";

fn wh() -> OutputSize {
    OutputSize::Wh {
        min: 64,
        max: 2048,
        step: 16,
    }
}

fn aspect() -> OutputSize {
    OutputSize::Aspect {
        allowed: ["1:1", "3:4", "4:3", "2:3"].map(String::from).to_vec(),
    }
}

fn grid(cols: u32, rows: u32, pps: u32) -> GenSize {
    GenSize::Grid { cols, rows, pps }
}

fn ratio(a: &str, long_side: u32) -> GenSize {
    GenSize::Ratio {
        aspect: a.into(),
        long_side,
    }
}

fn code<T: std::fmt::Debug>(r: Result<T, CoreError>) -> ErrorCode {
    r.unwrap_err().code
}

// ---------------------------------------------------------------- size fit (pure)

#[test]
fn fr_gen_08_wh_grid_lowers_pps_at_the_model_limit_then_snaps_to_step() {
    // Walkthrough step 2: 30 × 40 at 70 → pps 51, 1520 × 2032.
    let f = fit(&wh(), &grid(30, 40, 70)).unwrap();
    assert_eq!(
        (f.mode, f.w, f.h, f.pps),
        ("wh", Some(1520), Some(2032), Some(51))
    );
    assert_eq!(f.notices.len(), 1);
    assert_eq!(f.notices[0].code, "model_limit");
    // Within the limit: cells × pps rounded down to the step, no notice.
    let f = fit(&wh(), &grid(10, 7, 70)).unwrap();
    assert_eq!((f.w, f.h, f.pps), (Some(688), Some(480), Some(70)));
    assert!(f.notices.is_empty());
    // At least `min`.
    let f = fit(&wh(), &grid(1, 2, 16)).unwrap();
    assert_eq!((f.w, f.h), (Some(64), Some(64)));
}

#[test]
fn fr_gen_09_aspect_models_get_the_nearest_ratio_and_a_one_percent_warning() {
    // Walkthrough step 9: 25 × 40 → 2:3, off by 6.7 %.
    let f = fit(&aspect(), &grid(25, 40, 70)).unwrap();
    assert_eq!((f.mode, f.aspect.as_deref()), ("aspect", Some("2:3")));
    assert_eq!(f.notices[0].code, "cells_not_square");
    assert_eq!((f.w, f.h, f.pps), (None, None, Some(70)));
    // 30 × 40 is exactly 3:4: no warning.
    let f = fit(&aspect(), &grid(30, 40, 70)).unwrap();
    assert_eq!(f.aspect.as_deref(), Some("3:4"));
    assert!(f.notices.is_empty());
    // Non-ratio values are never picked; no ratio at all → invalid.
    let odd = OutputSize::Aspect {
        allowed: vec!["auto".into(), "16:9".into()],
    };
    assert_eq!(
        fit(&odd, &grid(1, 1, 70)).unwrap().aspect.as_deref(),
        Some("16:9")
    );
    let none = OutputSize::Aspect {
        allowed: vec!["auto".into()],
    };
    assert_eq!(code(fit(&none, &grid(1, 1, 70))), ErrorCode::InvalidInput);
    // A ratio size picks its own ratio exactly, with no pps and no warning.
    let f = fit(&aspect(), &ratio("4:3", 1024)).unwrap();
    assert_eq!((f.aspect.as_deref(), f.pps), (Some("4:3"), None));
    assert!(f.notices.is_empty());
}

#[test]
fn fr_gen_10_ratio_and_long_side_on_wh_models() {
    let f = fit(&wh(), &ratio("1:1", 1024)).unwrap();
    assert_eq!((f.w, f.h, f.pps), (Some(1024), Some(1024), None));
    let f = fit(&wh(), &ratio("16:9", 1000)).unwrap();
    assert_eq!(
        (f.w, f.h),
        (Some(992), Some(560)),
        "rounded down to the step"
    );
    let f = fit(&wh(), &ratio("2:3", 9000)).unwrap();
    assert_eq!((f.w, f.h), (Some(1360), Some(2048)), "long side within max");
}

#[test]
fn fr_gen_07_size_bounds_are_checked_at_the_boundary() {
    for bad in [
        grid(0, 10, 70),
        grid(201, 10, 70),
        grid(10, 10, 15),
        grid(10, 10, 513),
        ratio("auto", 1024),
        ratio("0:1", 1024),
        ratio("1:1", 0),
    ] {
        assert!(bad.validate("size").is_err(), "{bad:?}");
    }
    assert!(grid(200, 1, 512).validate("size").is_ok());
}

// ---------------------------------------------------------------- the lab

/// RGN's lab (a Maps project with an image, a spend cap, the fake provider) driving GEN.
struct G {
    lab: Lab,
}

impl G {
    fn new() -> G {
        G { lab: Lab::new() }
    }

    /// A Generate new project (no image) in the same library, open (FR-GEN-02).
    fn empty(section: &str, name: &str) -> G {
        let mut lab = Lab::new();
        let out = lab
            .core
            .dispatch(
                "gen_project_create",
                json!({"section": section, "name": name}),
            )
            .unwrap();
        lab.close();
        lab.project_dir = PathBuf::from(out["path"].as_str().unwrap());
        lab.open();
        G { lab }
    }

    fn draft(&self, prompt: &str) -> Value {
        json!({
            "model_id": WH,
            "controls": {},
            "seed": null,
            "size": {"mode": "grid", "cols": 30, "rows": 40, "pps": 70},
            "structure": null,
            "references": null,
            "typed_prompt": prompt,
            "final_prompt": prompt,
        })
    }

    fn dispatch(&self, name: &str, input: Value) -> Result<Value, CoreError> {
        self.lab.core.dispatch(name, input)
    }

    fn estimate(&self, draft: &Value) -> Result<Value, CoreError> {
        self.dispatch("gen_job_estimate", json!({ "draft": draft }))
    }

    fn submit(&self, draft: &Value) -> Result<Ulid, CoreError> {
        self.lab.clock.advance(std::time::Duration::from_secs(60));
        let est = self.estimate(draft)?;
        self.dispatch(
            "gen_job_submit",
            json!({"draft": draft, "estimate_id": est["estimate_id"], "confirmations": est["needs"]}),
        )
        .map(|v| v["job_id"].as_str().unwrap().parse().unwrap())
    }

    fn run(&self, draft: &Value) -> Ulid {
        let id = self.submit(draft).unwrap();
        self.lab.drive();
        id
    }

    fn job_json(&self, id: Ulid) -> Value {
        let path = self.lab.project_dir.join(job_dir(id)).join(JOB_FILE);
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn sketch(&self, file: &str, bytes: &[u8]) {
        std::fs::write(self.lab.project_dir.join("structure").join(file), bytes).unwrap();
    }

    fn retry(&self, id: Ulid) -> Result<Value, CoreError> {
        self.dispatch("gen_job_retry_draft", json!({ "job_id": id }))
    }
}

/// A 768 × 1024 line-art room layout (the sketch fixture's shape, generated).
fn layout() -> Vec<u8> {
    let img = image::GrayImage::from_fn(96, 128, |x, y| {
        image::Luma([if x % 32 == 0 || y % 32 == 0 { 0 } else { 255 }])
    });
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

// ---------------------------------------------------------------- commands

#[test]
fn fr_gen_08_size_fit_command_on_the_fake_models() {
    let g = G::new();
    let f = g
        .dispatch(
            "gen_size_fit",
            json!({"model_id": WH, "kind": "map", "size": {"mode": "grid", "cols": 30, "rows": 40, "pps": 70}}),
        )
        .unwrap();
    assert_eq!(
        (
            f["mode"].clone(),
            f["w"].clone(),
            f["h"].clone(),
            f["pps"].clone()
        ),
        (json!("wh"), json!(1520), json!(2032), json!(51))
    );
    assert_eq!(f["notices"][0]["code"], "model_limit");
    let f = g
        .dispatch(
            "gen_size_fit",
            json!({"model_id": ASPECT, "kind": "map", "size": {"mode": "grid", "cols": 25, "rows": 40, "pps": 70}}),
        )
        .unwrap();
    assert_eq!(f["aspect"], "2:3");
    assert!(f.get("w").is_none());
    // An edit-only model has no generate row.
    let e = g.dispatch(
        "gen_size_fit",
        json!({"model_id": "fake-white-edit", "kind": "map", "size": {"mode": "ratio", "aspect": "1:1", "long_side": 1024}}),
    );
    assert_eq!(code(e), ErrorCode::UnknownModel);
    let e = g.dispatch(
        "gen_size_fit",
        json!({"model_id": WH, "kind": "map", "size": {"mode": "grid", "cols": 0, "rows": 40, "pps": 70}}),
    );
    assert_eq!(code(e), ErrorCode::InvalidInput);
}

#[test]
fn fr_gen_06_and_11_controls_from_generate_capabilities_without_size_or_image_inputs() {
    let g = G::new();
    let e = g
        .dispatch(
            "gen_controls_evaluate",
            json!({"model_id": WH, "values": {"background": "transparent"}}),
        )
        .unwrap();
    let names: Vec<&str> = e["controls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"num_samples") && names.contains(&"background"));
    for hidden in ["refs", "width", "height", "image", "mask"] {
        assert!(!names.contains(&hidden), "{hidden} is never a control");
    }
    let bg = e["controls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "background")
        .unwrap();
    assert_eq!(bg["value"], "transparent");
    assert_eq!(bg["role"], "background");
    let e = g.dispatch(
        "gen_controls_evaluate",
        json!({"model_id": "nope", "values": {}}),
    );
    assert_eq!(code(e), ErrorCode::UnknownModel);
}

#[test]
fn fr_gen_20_estimate_id_hashes_only_cost_affecting_inputs() {
    let g = G::new();
    let id = |d: &Value| g.estimate(d).unwrap()["estimate_id"].clone();
    let base = g.draft("flooded crypt");
    let a = id(&base);
    let mut d = base.clone();
    d["typed_prompt"] = json!("a dry crypt");
    assert_eq!(id(&d), a, "the prompt never re-estimates");
    let mut d = base.clone();
    d["controls"] = json!({"background": "transparent"});
    assert_eq!(id(&d), a, "background isn't cost-affecting");
    let mut d = base.clone();
    d["controls"] = json!({"num_samples": 3});
    assert_ne!(id(&d), a, "the count is");
    let mut d = base.clone();
    d["size"] = json!({"mode": "grid", "cols": 30, "rows": 30, "pps": 50});
    assert_ne!(id(&d), a, "the sent size is");
    let mut d = base.clone();
    d["model_id"] = json!(ASPECT);
    assert_ne!(id(&d), a);
    // The sketch's slot is cost-affecting on fake-gen-wh: its bytes are in the id.
    g.sketch("rooms.png", &layout());
    let mut d = base.clone();
    d["structure"] = json!({"file": "rooms.png"});
    let with = id(&d);
    assert_ne!(with, a);
    g.sketch("rooms.png", &png_bytes(64, 64, 9));
    assert_ne!(id(&d), with, "new sketch bytes re-estimate");
    // The estimate's shape (R77 `GenEstimate`).
    let e = g.estimate(&base).unwrap();
    assert_eq!(
        e["output"],
        json!({"mode": "wh", "w": 1520, "h": 2032, "grid": {"cols": 30, "rows": 40, "pps": 51}})
    );
    assert_eq!(e["candidate_count"], 1, "the model default");
    assert_eq!(e["cost"]["amount"], 1.0);
    assert!(e.get("fit").is_none());
    assert_eq!(e["references"], 0);
}

#[test]
fn fr_gen_22_job_record_has_no_region_fit_mask_or_inputs() {
    let g = G::new();
    g.sketch("rooms.png", &layout());
    let mut d = g.draft("Flooded crypt, torchlight");
    d["structure"] = json!({"file": "rooms.png"});
    d["controls"] = json!({"num_samples": 1});
    let id = g.submit(&d).unwrap();
    let j = g.job_json(id);
    assert_eq!(j["operation"], "generate");
    assert_eq!(j["state"], "submitted");
    for f in ["region", "fit", "mask_id", "inputs"] {
        assert_eq!(j[f], Value::Null, "{f} is null for generate");
    }
    let files: Vec<String> = std::fs::read_dir(g.lab.project_dir.join(job_dir(id)))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, vec![JOB_FILE.to_owned()], "no input PNGs");
    assert_eq!(j["typed_prompt"], "Flooded crypt, torchlight");
    assert_eq!(j["final_prompt"], "Flooded crypt, torchlight");
    assert_eq!(j["blend_instruction"], false);
    assert_eq!(j["prompt_helpers"], json!([]));
    assert_eq!(j["candidate_count"], 1);
    assert_eq!(j["output"]["w"], 1520);
    assert_eq!(j["output"]["grid"]["pps"], 51);
    assert_eq!(j["size"]["pps"], 70, "the size as asked");
    assert_eq!(j["references"][0]["source"], "structure");
    assert_eq!(j["references"][0]["path"], "structure/rooms.png");
    assert!(j["seed"]["requested"].is_u64(), "a random seed");
    assert_eq!(
        j["estimate"]["amount"], 2.0,
        "1 image + 1 cost-affecting ref"
    );
    assert_eq!(j["capability_snapshot"]["model_id"], WH);
}

#[test]
fn fr_gen_22_prompt_helpers_through_prm_compose_never_the_blend_instruction() {
    let g = G::new();
    let h = crate::prompt::helpers::starter_set()
        .into_iter()
        .find(|h| h.id == "builtin:top-down-map")
        .unwrap();
    let mut d = g.draft("crypt");
    d["prompt_helpers"] = json!([h.id]);
    let id = g.submit(&d).unwrap();
    let j = g.job_json(id);
    let text = h.text.trim();
    assert_eq!(j["final_prompt"], json!(format!("crypt\n\n{text}")));
    assert_eq!(j["typed_prompt"], "crypt");
    assert_eq!(j["prompt_helpers"][0]["id"], json!(h.id));
    assert_eq!(j["blend_instruction"], false);
    let blend = g.dispatch("rgn_blend_instruction_get", json!({})).unwrap();
    let blend = blend["text"].as_str().unwrap();
    assert!(!j["final_prompt"].as_str().unwrap().contains(blend));
}

#[test]
fn fr_gen_23_runs_through_rgns_runner_into_an_iteration_folder() {
    let g = G::empty("maps", "Crypt");
    let mut d = g.draft("Flooded crypt");
    d["size"] = json!({"mode": "ratio", "aspect": "4:3", "long_side": 512});
    d["controls"] = json!({"num_samples": 2});
    let id = g.run(&d);
    let job = g.lab.job(id);
    assert_eq!(job.state, JobState::Fetched);
    let it = job.iteration.clone().unwrap();
    assert_eq!(it.dir, "results/001 flooded crypt");
    let cands = g.lab.candidates(id);
    assert_eq!(cands.len(), 2);
    for c in &cands {
        assert_eq!((c.width, c.height), (512, 384), "at the sent size");
        assert!(!c.extra.contains_key("aspect_warning"));
        assert!(g.lab.project_dir.join(&c.file).is_file());
    }
    // FR-GEN-25 / R77 G6: the tracker row.
    let rows = g.dispatch("rgn_iterations_list", json!({})).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["job_id"] == json!(id))
        .unwrap();
    assert_eq!(row["operation"], "generate");
    assert_eq!(row["output"], json!({"mode": "wh", "w": 512, "h": 384}));
    assert_eq!(row["n"], 1);
}

#[test]
fn fr_gen_24_an_empty_slug_is_new_and_a_size_mismatch_is_badged_not_resized() {
    let g = G::new();
    let id = g.run(&g.draft("!!!"));
    assert_eq!(g.lab.job(id).iteration.unwrap().dir, "results/001 new");
    g.lab.set_case("aspect_mismatch(10)");
    let id = g.run(&g.draft("crypt"));
    let c = &g.lab.candidates(id)[0];
    assert_eq!(c.extra.get("aspect_warning"), Some(&json!(true)));
    assert_ne!((c.width, c.height), (1520, 2032), "kept as returned");
}

#[test]
fn fr_gen_22_one_active_job_is_shared_with_region_edit() {
    let g = G::new();
    g.lab.submit("a kitchen");
    assert_eq!(code(g.submit(&g.draft("crypt"))), ErrorCode::JobActive);
    g.lab.drive();
    let id = g.submit(&g.draft("crypt")).unwrap();
    assert_eq!(
        code(g.lab.submit_draft(g.lab.draft("again"))),
        ErrorCode::JobActive
    );
    g.lab.drive();
    assert_eq!(g.lab.job(id).state, JobState::Fetched);
}

#[test]
fn fr_gen_23_a_job_survives_an_app_restart() {
    let g = G::new();
    g.lab.set_case("slow(3)");
    let mut d = g.draft("crypt");
    d["controls"] = json!({"num_samples": 2});
    let id = g.submit(&d).unwrap();
    let lab = g.lab.restart();
    lab.open();
    lab.drive();
    assert_eq!(lab.job(id).state, JobState::Fetched);
    assert_eq!(lab.candidates(id).len(), 2);
}

#[test]
fn fr_gen_21_spend_cap_and_confirmations_as_rgn() {
    let g = G::new();
    g.dispatch("rgn_spend_set", json!({"cap": 1, "threshold": 1}))
        .unwrap();
    let mut d = g.draft("crypt");
    d["controls"] = json!({"num_samples": 3});
    let est = g.estimate(&d).unwrap();
    assert_eq!(est["over_cap"], true);
    let e = g.dispatch(
        "gen_job_submit",
        json!({"draft": d, "estimate_id": est["estimate_id"], "confirmations": ["threshold"]}),
    );
    assert_eq!(code(e), ErrorCode::OverSpendCap);
    g.dispatch("rgn_spend_set", json!({"cap": 100, "threshold": 1}))
        .unwrap();
    let e = g.dispatch(
        "gen_job_submit",
        json!({"draft": d, "estimate_id": est["estimate_id"], "confirmations": []}),
    );
    assert_eq!(code(e), ErrorCode::ConfirmationMissing);
    let e = g.dispatch(
        "gen_job_submit",
        json!({"draft": d, "estimate_id": "est-0000", "confirmations": ["threshold"]}),
    );
    assert_eq!(code(e), ErrorCode::EstimateStale);
    let mut empty = d.clone();
    empty["typed_prompt"] = json!("  ");
    let e = g.dispatch(
        "gen_job_submit",
        json!({"draft": empty, "estimate_id": est["estimate_id"], "confirmations": ["threshold"]}),
    );
    assert_eq!(code(e), ErrorCode::InvalidControl, "prompt_required");
    assert!(
        g.lab.manifest()["jobs"].as_array().unwrap().is_empty(),
        "nothing recorded"
    );
}

#[test]
fn fr_gen_12_transparent_background_is_sent_and_alpha_kept() {
    let g = G::empty("tokens", "Goblin archer");
    let mut d = g.draft("goblin archer");
    d["controls"] = json!({"background": "transparent", "num_samples": 1});
    d["size"] = json!({"mode": "ratio", "aspect": "1:1", "long_side": 256});
    let id = g.run(&d);
    assert_eq!(g.lab.job(id).controls["background"], "transparent");
    let c = &g.lab.candidates(id)[0];
    let img = image::open(g.lab.project_dir.join(&c.file)).unwrap();
    assert!(img.color().has_alpha(), "alpha kept in results/");
}

// ---------------------------------------------------------------- sketch (FR-GEN-13/15)

#[test]
fn fr_gen_15_the_sketch_is_sent_first_in_its_slot() {
    let g = G::new();
    g.sketch("rooms.png", &layout());
    std::fs::write(
        g.lab.project_dir.join("references/a.png"),
        png_bytes(32, 24, 40),
    )
    .unwrap();
    let mut d = g.draft("crypt");
    d["structure"] = json!({"file": "rooms.png"});
    let est = g.estimate(&d).unwrap();
    assert_eq!(est["references"], 1, "the sketch is not counted");
    let id = g.submit(&d).unwrap();
    let sent = g.lab.fake().last_reference_hashes();
    assert_eq!(sent.len(), 2);
    let j = g.job_json(id);
    assert_eq!(sent[0].0, "refs");
    assert_eq!(json!(sent[0].1), j["references"][0]["sent"]["hash"]);
    assert_eq!(j["references"][0]["source"], "structure");
    assert_eq!(j["references"][1]["path"], "references/a.png");
    // FR-GEN-25: the chip counts references, not the sketch.
    g.lab.drive();
    let rows = g.dispatch("rgn_iterations_list", json!({})).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["job_id"] == json!(id))
        .unwrap();
    assert_eq!(row["references"], 1);
}

#[test]
fn fr_gen_15_a_missing_or_corrupt_sketch_fails_before_any_provider_call() {
    let g = G::new();
    let mut d = g.draft("crypt");
    d["structure"] = json!({"file": "gone.png"});
    let e = g.estimate(&d).unwrap_err();
    assert_eq!(e.code, ErrorCode::ReferenceUnavailable);
    assert_eq!(e.detail["key"], "structure:gone.png");
    assert_eq!(e.detail["reason"], "missing");
    g.sketch("bad.png", b"not a png");
    d["structure"] = json!({"file": "bad.png"});
    let e = g.estimate(&d).unwrap_err();
    assert_eq!(e.detail["reason"], "corrupt");
    d["structure"] = json!({"file": "../project.json"});
    assert_eq!(code(g.estimate(&d)), ErrorCode::InvalidInput);
    // A model without a sketch slot ignores the choice (nothing sent, nothing charged).
    d["model_id"] = json!(ASPECT);
    d["structure"] = json!({"file": "gone.png"});
    assert!(g.estimate(&d).is_ok());
}

#[test]
fn fr_gen_13_the_sketch_slot_gives_up_one_place_only_when_chosen() {
    let g = G::new();
    let max = |structure: Value, model: &str| {
        let r = g
            .dispatch(
                "ref_resolve",
                json!({"model_id": model, "references": null, "operation": "generate", "structure": structure}),
            )
            .unwrap();
        r["slots"][0]["max"].clone()
    };
    assert_eq!(max(Value::Null, WH), 4);
    assert_eq!(max(json!("rooms.png"), WH), 3);
    assert_eq!(
        max(json!("rooms.png"), ASPECT),
        2,
        "no sketch slot: unchanged"
    );
    // Prepare enforces the reduced max: 4 references + a sketch is too many.
    g.sketch("rooms.png", &layout());
    for (i, f) in ["a", "b", "c", "d"].iter().enumerate() {
        std::fs::write(
            g.lab.project_dir.join(format!("references/{f}.png")),
            png_bytes(32, 24, 10 * i as u8 + 1),
        )
        .unwrap();
    }
    let mut d = g.draft("crypt");
    assert_eq!(
        g.estimate(&d).unwrap()["references"],
        4,
        "the defaults fill 4 places"
    );
    d["structure"] = json!({"file": "rooms.png"});
    assert_eq!(
        g.estimate(&d).unwrap()["references"],
        3,
        "the sketch takes one"
    );
    // Four ticked references and a sketch: REF's reconcile keeps three on (FR-REF-18).
    let on: Vec<Value> = ["a", "b", "c", "d"]
        .iter()
        .map(|f| json!({"key": format!("project:{f}.png"), "on": true, "role": null}))
        .collect();
    d["references"] = json!({"packs": [], "items": on});
    assert_eq!(g.estimate(&d).unwrap()["references"], 3);
}

// ---------------------------------------------------------------- retry (FR-GEN-32)

#[test]
fn fr_gen_32_retry_draft_prefills_and_flags_the_sketch() {
    let g = G::new();
    g.sketch("rooms.png", &layout());
    let mut d = g.draft("Flooded crypt");
    d["structure"] = json!({"file": "rooms.png"});
    d["controls"] = json!({"num_samples": 3, "background": "transparent"});
    let id = g.run(&d);
    let r = g.retry(id).unwrap();
    assert_eq!(r["model_id"], WH);
    assert_eq!(r["typed_prompt"], "Flooded crypt");
    assert_eq!(
        r["size"],
        json!({"mode": "grid", "cols": 30, "rows": 40, "pps": 70})
    );
    assert_eq!(r["controls"]["num_samples"], 3);
    assert_eq!(r["controls"]["background"], "transparent");
    assert_eq!(r["seed"], Value::Null, "a new random seed");
    assert_eq!(r["structure"], json!({"file": "rooms.png"}));
    assert_eq!(r["prompt_helpers"], json!([]));
    // Edited in Explorer → `changed since 001`; deleted → `missing`.
    g.sketch("rooms.png", &png_bytes(64, 64, 3));
    assert_eq!(
        g.retry(id).unwrap()["structure"],
        json!({"file": "rooms.png", "changed_since": "001"})
    );
    std::fs::remove_file(g.lab.project_dir.join("structure/rooms.png")).unwrap();
    assert_eq!(
        g.retry(id).unwrap()["structure"],
        json!({"file": "rooms.png", "missing": true})
    );
    // A pinned seed is kept.
    let mut p = g.draft("pinned");
    p["seed"] = json!(42);
    let pid = g.run(&p);
    assert_eq!(g.retry(pid).unwrap()["seed"], 42);
    assert_eq!(g.retry(pid).unwrap()["structure"], Value::Null);
}

#[test]
fn fr_gen_32_retry_draft_errors() {
    let g = G::new();
    let rgn = g.lab.run("a kitchen");
    assert_eq!(code(g.retry(rgn)), ErrorCode::WrongOperation);
    assert_eq!(code(g.retry(Ulid::from_parts(1, 1))), ErrorCode::NotFound);
}

// ---------------------------------------------------------------- read-only, no project

#[test]
fn fr_gen_34_read_only_gives_no_estimate_and_no_submit() {
    let g = G::new();
    let d = g.draft("crypt");
    let est = g.estimate(&d).unwrap();
    g.lab.core.state().project.as_mut().unwrap().read_only = Some(ReadOnlyReason::NewerVersion);
    assert_eq!(code(g.estimate(&d)), ErrorCode::ReadOnly);
    let e = g.dispatch(
        "gen_job_submit",
        json!({"draft": d, "estimate_id": est["estimate_id"], "confirmations": est["needs"]}),
    );
    assert_eq!(code(e), ErrorCode::ReadOnly);
}

#[test]
fn fr_gen_35_commands_need_an_open_project_and_valid_input() {
    let g = G::new();
    let d = g.draft("crypt");
    g.lab.close();
    assert_eq!(code(g.estimate(&d)), ErrorCode::NoProject);
    let mut bad = d.clone();
    bad["region"] = json!({});
    assert_eq!(code(g.estimate(&bad)), ErrorCode::InvalidInput);
}

// ---------------------------------------------------------------- slice 4: preview, pick, undo

impl G {
    fn cands(&self, job: Ulid) -> Vec<Ulid> {
        self.lab.candidates(job).iter().map(|c| c.id).collect()
    }

    fn preview(&self, job: Ulid, cand: Ulid) -> Result<Value, CoreError> {
        self.dispatch(
            "gen_candidate_preview",
            json!({"job_id": job, "candidate_id": cand}),
        )
    }

    fn pick(&self, job: Ulid, cand: Ulid) -> Result<Value, CoreError> {
        self.dispatch(
            "gen_candidate_pick",
            json!({"job_id": job, "candidate_id": cand}),
        )
    }

    fn snapshot(&self) -> Value {
        self.dispatch("project_get", json!({})).unwrap()
    }

    fn two(&self, prompt: &str) -> Value {
        let mut d = self.draft(prompt);
        d["size"] = json!({"mode": "ratio", "aspect": "4:3", "long_side": 256});
        d["controls"] = json!({"num_samples": 2});
        d
    }
}

#[test]
fn fr_gen_26_preview_registers_the_candidate_alone_or_for_compare() {
    let g = G::empty("maps", "Crypt");
    let job = g.run(&g.two("crypt"));
    let c = g.cands(job);
    let p = g.preview(job, c[0]).unwrap();
    assert!(
        p["url"]
            .as_str()
            .unwrap()
            .contains(p["key"].as_str().unwrap())
    );
    assert_eq!(p["rect"], json!({"x": 0, "y": 0, "w": 256, "h": 192}));
    assert_eq!(p["same_size"], false, "no image yet: shown alone");
    g.pick(job, c[0]).unwrap();
    let again = g.run(&g.two("crypt again"));
    let p = g.preview(again, g.cands(again)[1]).unwrap();
    assert_eq!(
        p["same_size"], true,
        "same size as the working image: compare"
    );
    // In a project whose image is another size: alone.
    let g = G::new();
    let job = g.run(&g.two("crypt"));
    assert_eq!(g.preview(job, g.cands(job)[0]).unwrap()["same_size"], false);
}

#[test]
fn fr_gen_12_a_transparent_candidate_previews_over_the_flatten_colour() {
    let rgba = image::RgbaImage::from_fn(4, 4, |x, _| {
        if x < 2 {
            image::Rgba([0, 0, 0, 0])
        } else {
            image::Rgba([200, 10, 10, 255])
        }
    });
    let mut png = Vec::new();
    rgba.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let out = super::commands::flattened_if_alpha(png).unwrap();
    let img = image::load_from_memory(&out).unwrap();
    assert!(!img.color().has_alpha(), "an opaque preview");
    let rgb = img.into_rgb8();
    assert_eq!(rgb.get_pixel(0, 0).0, [0x80, 0x80, 0x80]);
    assert_eq!(rgb.get_pixel(3, 0).0, [200, 10, 10]);
    let opaque = png_bytes(4, 4, 7);
    assert_eq!(
        super::commands::flattened_if_alpha(opaque.clone()).unwrap(),
        opaque,
        "an opaque candidate is registered as is"
    );
}

#[test]
fn fr_gen_27_pick_command_makes_the_base_image_in_one_accept_revision() {
    use crate::manifest::CandidateStatus::{Accepted, Dismissed};
    let g = G::empty("maps", "Crypt");
    let job = g.run(&g.two("Flooded crypt"));
    let c = g.cands(job);
    let out = g.pick(job, c[1]).unwrap();
    assert!(out["layer_id"].is_string() && out["revision_id"].is_string());
    assert_eq!(out["file"], "working/001 flooded crypt 02.png");
    let s = g.snapshot();
    assert_eq!(s["has_image"], true);
    assert_eq!(s["layers"].as_array().unwrap().len(), 1);
    assert_eq!(s["layers"][0]["kind"], "base");
    assert_eq!(
        (s["width"].clone(), s["height"].clone()),
        (json!(256), json!(192))
    );
    assert_eq!(s["head_revision"], out["revision_id"]);
    assert!(
        g.lab
            .project_dir
            .join("working/001 flooded crypt 02.png")
            .is_file()
    );
    assert_eq!(g.lab.job(job).state, JobState::Closed);
    let statuses: Vec<_> = g.lab.candidates(job).iter().map(|c| c.status).collect();
    assert_eq!(
        statuses,
        vec![Dismissed, Accepted],
        "the other one is kept (#56)"
    );
    // FR-GEN-25: the tracker's tick.
    let rows = g.dispatch("rgn_iterations_list", json!({})).unwrap();
    assert_eq!(rows[0]["accepted_in_stack"], true);
}

#[test]
fn fr_gen_29_undo_of_the_first_pick_returns_to_no_image_and_redo_brings_it_back() {
    let g = G::empty("maps", "Crypt");
    let parent = g.snapshot()["head_revision"].clone();
    let job = g.run(&g.two("crypt"));
    let picked = g.pick(job, g.cands(job)[0]).unwrap();
    g.dispatch("project_restore_revision", json!({ "rev_id": parent }))
        .unwrap();
    let s = g.snapshot();
    assert_eq!(s["has_image"], false);
    assert_eq!(s["image_path"], Value::Null);
    assert!(s["layers"].as_array().unwrap().is_empty());
    g.dispatch(
        "project_restore_revision",
        json!({ "rev_id": picked["revision_id"] }),
    )
    .unwrap();
    assert_eq!(g.snapshot()["has_image"], true);
}

#[test]
fn fr_gen_30_pick_from_an_older_iteration_without_spend() {
    use crate::manifest::CandidateStatus::Accepted;
    let g = G::empty("maps", "Crypt");
    let first = g.run(&g.two("crypt"));
    g.pick(first, g.cands(first)[0]).unwrap();
    let second = g.run(&g.two("another crypt"));
    let spent = g.dispatch("rgn_spend_get", json!({})).unwrap()["spend"].clone();
    // Re-pick #2 of the older 001, with a newer iteration in the project.
    let out = g.pick(first, g.cands(first)[1]).unwrap();
    assert_eq!(out["file"], "working/001 crypt 02.png");
    assert_eq!(g.lab.job(first).state, JobState::Closed, "stays closed");
    assert_eq!(
        g.dispatch("rgn_spend_get", json!({})).unwrap()["spend"],
        spent,
        "no spend"
    );
    // The newer one can be picked too; the earlier accepted ones keep their status.
    g.pick(second, g.cands(second)[0]).unwrap();
    assert!(g.lab.candidates(first).iter().all(|c| c.status == Accepted));
}

#[test]
fn fr_gen_31_star_discard_and_discard_the_rest_on_generate_candidates() {
    let g = G::empty("maps", "Crypt");
    let mut d = g.two("crypt");
    d["controls"] = json!({"num_samples": 4});
    let job = g.run(&d);
    let c = g.cands(job);
    g.dispatch(
        "rgn_candidate_star",
        json!({"job_id": job, "candidate_id": c[3], "starred": true}),
    )
    .unwrap();
    g.dispatch(
        "rgn_candidate_discard",
        json!({"job_id": job, "candidate_id": c[2]}),
    )
    .unwrap();
    g.pick(job, c[0]).unwrap();
    let out = g
        .dispatch("rgn_job_discard_rest", json!({ "job_id": job }))
        .unwrap();
    assert_eq!(
        out["trashed"], 1,
        "only #2: #1 is accepted, #4 starred, #3 already discarded"
    );
    assert_eq!(code(g.pick(job, c[2])), ErrorCode::InvalidCandidate);
}

#[test]
fn fr_gen_27_pick_and_preview_errors() {
    let g = G::new();
    let rgn = g.lab.run("a kitchen");
    let rc = g.lab.candidates(rgn)[0].id;
    let job = g.run(&g.two("crypt"));
    let c = g.cands(job);
    g.pick(job, c[0]).unwrap();
    assert_eq!(code(g.pick(rgn, rc)), ErrorCode::WrongOperation);
    assert_eq!(code(g.preview(rgn, rc)), ErrorCode::WrongOperation);
    // RGN's candidate commands refuse a generate job.
    for cmd in ["rgn_candidate_preview", "rgn_candidate_accept"] {
        let e = g.dispatch(cmd, json!({"job_id": job, "candidate_id": c[1]}));
        assert_eq!(code(e), ErrorCode::WrongOperation, "{cmd}");
    }
    assert_eq!(
        code(g.pick(Ulid::from_parts(1, 1), c[0])),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(g.pick(job, rc)),
        ErrorCode::InvalidCandidate,
        "another job's"
    );
    assert_eq!(
        code(g.preview(job, Ulid::from_parts(1, 2))),
        ErrorCode::NotFound
    );
    g.lab.core.state().project.as_mut().unwrap().read_only = Some(ReadOnlyReason::NewerVersion);
    assert_eq!(code(g.pick(job, c[1])), ErrorCode::ReadOnly);
    assert!(
        g.preview(job, c[1]).is_ok(),
        "preview still works read-only"
    );
}

#[test]
fn fr_gen_06_the_ui_fixture_matches_the_fake_generate_capabilities() {
    // `fixtures/gen/capabilities-generate.json` is the UI's copy (Vitest, e2e): it must equal
    // what `provider_get_capabilities{operation: generate}` serves for the fake models.
    let g = G::new();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/gen/capabilities-generate.json"
    );
    let mut served = serde_json::Map::new();
    for model in [WH, ASPECT] {
        let caps = g
            .dispatch(
                "provider_get_capabilities",
                json!({"model_id": model, "operation": "generate"}),
            )
            .unwrap();
        served.insert(model.to_owned(), caps);
    }
    // `PF_REGEN_FIXTURES=1 cargo test -p pf-core fr_gen_06_the_ui_fixture` rewrites it.
    if std::env::var_os("PF_REGEN_FIXTURES").is_some() {
        let mut text = serde_json::to_string_pretty(&Value::Object(served.clone())).unwrap();
        text.push('\n');
        std::fs::write(path, text).unwrap();
    }
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for model in [WH, ASPECT] {
        assert_eq!(
            fixture[model], served[model],
            "{model}: regenerate the fixture"
        );
    }
}

// ---------------------------------------------------------------- slice 6: chat context

use crate::prompt::context::{
    CHAT_IMAGE_MAX_SIDE, CHAT_MAX_BYTES, CHAT_MAX_IMAGES, ChatContextSource, ChatHost, ChatImage,
    ChatImageRole, dropped_fact, limit_images,
};

const NO_PEOPLE: &str = "builtin:no-people";

impl G {
    fn project_id(&self) -> Ulid {
        self.lab
            .core
            .state()
            .project
            .as_ref()
            .unwrap()
            .manifest
            .project_id
    }

    fn reference(&self, file: &str, bytes: &[u8]) {
        std::fs::write(self.lab.project_dir.join("references").join(file), bytes).unwrap();
    }

    /// The tab's usual calls this session: models (names) and an estimate (capabilities).
    fn warm(&self, draft: &Value) {
        self.dispatch("provider_list_models", json!({"operation": "generate"}))
            .unwrap();
        self.estimate(draft).unwrap();
    }

    fn context(&self, draft: &Value) -> crate::prompt::context::ChatContext {
        super::chat::GenContextSource
            .build(&self.lab.core, self.project_id(), draft)
            .unwrap()
    }
}

fn facts_text(ctx: &crate::prompt::context::ChatContext) -> String {
    ctx.facts.join(" | ")
}

#[test]
fn fr_gen_17_fr_prm_42_context_text_side_of_the_new_image_job() {
    let g = G::new();
    g.sketch("rooms.png", &layout());
    let mut d = g.draft("Flooded crypt, torchlight");
    d["structure"] = json!({"file": "rooms.png"});
    d["controls"] = json!({"num_samples": 3, "background": "transparent"});
    d["prompt_helpers"] = json!([NO_PEOPLE]);
    g.warm(&d);
    let ctx = g.context(&d);
    assert_eq!(super::chat::GenContextSource.host(), ChatHost::Gen);
    assert_eq!(ctx.host, ChatHost::Gen);
    assert_eq!(ctx.kind, crate::manifest::Kind::Map);
    assert_eq!(ctx.typed_prompt, "Flooded crypt, torchlight");
    assert_eq!(
        ctx.final_prompt,
        "Flooded crypt, torchlight\n\nNo people, creatures or tokens."
    );
    assert_eq!(ctx.helpers.len(), 1);
    assert_eq!(ctx.blend_instruction, None, "never the blend instruction");
    assert_eq!(ctx.model.id, WH);
    assert_ne!(ctx.model.name, WH, "the display name from the model list");
    let names: Vec<&str> = ctx.controls.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["num_samples", "background"],
        "no prompt, seed or image inputs"
    );
    let count = &ctx.controls[0];
    assert_eq!(
        (count.value.clone(), count.min, count.max),
        (json!(3), Some(1.0), Some(4.0))
    );
    assert_eq!(
        ctx.controls[1].allowed,
        Some(vec![json!("opaque"), json!("transparent")])
    );
    let f = facts_text(&ctx);
    assert!(
        f.contains("size: grid 30 × 40 at 70 px per square; sent 1520 × 2032 (51 px per square, model limit)"),
        "{f}"
    );
    assert!(f.contains("3 images per job"), "{f}");
    assert!(f.contains("sketch: rooms.png"), "{f}");
    // The final prompt is exactly what submit sends.
    let id = g.submit(&d).unwrap();
    assert_eq!(json!(ctx.final_prompt), g.job_json(id)["final_prompt"]);
}

#[test]
fn fr_gen_17_fr_prm_42_images_are_the_sketch_then_the_on_references_in_send_order() {
    let g = G::new();
    g.sketch("rooms.png", &layout());
    g.reference("a.png", &png_bytes(32, 24, 10));
    g.reference("big.png", &png_bytes(2000, 1000, 20));
    let mut d = g.draft("crypt");
    d["structure"] = json!({"file": "rooms.png"});
    g.warm(&d);
    let ctx = g.context(&d);
    let ids: Vec<&str> = ctx.images.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "structure:structure/rooms.png",
            "ref:project:a.png",
            "ref:project:big.png"
        ]
    );
    assert_eq!(ctx.images[0].role, ChatImageRole::Structure);
    assert_eq!(ctx.images[0].label, "sketch rooms.png");
    assert_eq!(ctx.images[1].role, ChatImageRole::Reference);
    assert_eq!(ctx.images[1].label, "a.png (project)");
    // FR-PRM-25: JPEG, long side ≤ 1568.
    let big = &ctx.images[2];
    assert_eq!((big.w, big.h), (CHAT_IMAGE_MAX_SIDE, 784));
    assert_eq!(&big.jpeg[..2], &[0xFF, 0xD8], "JPEG");
    assert!(big.content_hash.starts_with("sha256:"));
    // The hash is the file's, as REF's prepare reads it (the job row has the same).
    let id = g.submit(&d).unwrap();
    assert_eq!(
        json!(ctx.images[0].content_hash),
        g.job_json(id)["references"][0]["content_hash"]
    );
}

#[test]
fn fr_gen_17_fr_prm_25_the_turn_limit_drops_the_tail_so_the_sketch_goes_last() {
    let g = G::new();
    g.sketch("rooms.png", &layout());
    let files: Vec<String> = (0..16).map(|i| format!("r{i:02}.png")).collect();
    for (i, f) in files.iter().enumerate() {
        g.reference(f, &png_bytes(32, 24, i as u8 * 9 + 1));
    }
    let mut d = g.draft("crypt");
    d["structure"] = json!({"file": "rooms.png"});
    g.warm(&d);
    // A model that takes 16 references besides the sketch (as GPT Image 2.5 Flare).
    {
        let mut st = g.lab.core.state();
        let caps = st.generate.caps.get_mut(WH).unwrap();
        caps.slots[0].max = 17;
    }
    let mut ctx = g.context(&d);
    assert_eq!(ctx.images.len(), 17);
    let dropped = limit_images(&mut ctx.images);
    assert_eq!(ctx.images.len(), CHAT_MAX_IMAGES);
    assert_eq!(
        ctx.images[0].role,
        ChatImageRole::Structure,
        "the sketch is kept"
    );
    let kept: Vec<&str> = ctx.images[1..].iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        kept,
        files[..5]
            .iter()
            .map(|f| format!("{f} (project)"))
            .collect::<Vec<_>>()
    );
    assert_eq!(dropped.len(), 11);
    assert_eq!(dropped[0].label, "r05.png (project)");
    let fact = dropped_fact(&dropped).unwrap();
    assert!(fact.starts_with("not sent (turn limit): r05.png (project), r06.png"));
    assert!(fact.ends_with("r15.png (project)"));
}

#[test]
fn fr_prm_25_the_byte_limit_drops_from_the_tail_too() {
    let img = |n: usize, mib: usize| ChatImage {
        id: format!("ref:{n}"),
        role: ChatImageRole::Reference,
        label: format!("{n}"),
        w: 1,
        h: 1,
        content_hash: String::new(),
        jpeg: vec![0; mib * 1024 * 1024],
    };
    let mut images = vec![img(0, 1), img(1, 2), img(2, 1), img(3, 1)];
    let dropped = limit_images(&mut images);
    assert_eq!(images.len(), 3, "1 + 2 + 1 MiB = 4 MiB fits");
    assert_eq!(dropped.len(), 1);
    assert!(images.iter().map(|i| i.jpeg.len()).sum::<usize>() <= CHAT_MAX_BYTES);
    assert_eq!(dropped_fact(&[]), None);
}

#[test]
fn fr_gen_17_fr_prm_41_an_image_that_cannot_be_sent_is_dropped_and_named() {
    let g = G::new();
    g.reference("a.png", &png_bytes(32, 24, 10));
    g.reference("b.png", &png_bytes(32, 24, 20));
    let mut d = g.draft("crypt");
    d["structure"] = json!({"file": "gone.png"});
    g.warm(&g.draft("crypt"));
    let ctx = g.context(&d);
    let ids: Vec<&str> = ctx.images.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["ref:project:a.png", "ref:project:b.png"],
        "the turn still has the rest"
    );
    let f = facts_text(&ctx);
    assert!(
        f.contains("The sketch gone.png can't be sent: it can't be found (not sent)"),
        "{f}"
    );
    // A reference that went corrupt after it was ticked: off, and named.
    g.reference("b.png", b"not an image");
    let mut ticked = g.draft("crypt");
    ticked["references"] = json!({"packs": [], "items": [
        {"key": "project:a.png", "on": true, "role": null},
        {"key": "project:b.png", "on": true, "role": null},
    ]});
    let ctx = g.context(&ticked);
    let ids: Vec<&str> = ctx.images.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, vec!["ref:project:a.png"]);
    assert!(
        facts_text(&ctx).contains("reference b.png not sent (corrupt)"),
        "{}",
        facts_text(&ctx)
    );
}

#[test]
fn fr_gen_17_aspect_models_without_a_sketch_slot_and_no_cached_capabilities() {
    let g = G::new();
    g.sketch("rooms.png", &layout());
    let mut d = g.draft("crypt");
    d["model_id"] = json!(ASPECT);
    d["size"] = json!({"mode": "grid", "cols": 25, "rows": 40, "pps": 70});
    d["structure"] = json!({"file": "rooms.png"});
    // Nothing fetched yet this session: text only, no provider call, the id as the name.
    let ctx = g.context(&d);
    assert!(ctx.images.is_empty());
    assert_eq!(ctx.model.name, ASPECT);
    assert!(facts_text(&ctx).contains("aren't loaded yet"));
    g.warm(&d);
    let ctx = g.context(&d);
    assert!(
        ctx.images.is_empty(),
        "the sketch is never sent to a model that takes none"
    );
    let f = facts_text(&ctx);
    assert!(f.contains("this model takes no structural sketch"), "{f}");
    assert!(
        f.contains(
            "size: grid 25 × 40 at 70 px per square; sent ratio 2:3 (cells won't be square)"
        ),
        "{f}"
    );
    assert!(f.contains("1 image per job"), "{f}");
    let mut none = g.draft("crypt");
    none["model_id"] = json!(WH);
    g.warm(&none);
    assert!(facts_text(&g.context(&none)).contains("no sketch"));
}

#[test]
fn fr_prm_41_build_refuses_another_project_and_a_bad_input() {
    let g = G::new();
    let d = g.draft("crypt");
    let e = super::chat::GenContextSource
        .build(&g.lab.core, Ulid::from_parts(9, 9), &d)
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NoProject);
    let e = super::chat::GenContextSource
        .build(&g.lab.core, g.project_id(), &json!({"model_id": WH}))
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidInput);
}

#[test]
fn fr_gen_32_fr_prm_10_retry_badges_for_changed_and_missing_helpers() {
    let g = G::new();
    let mut d = g.draft("crypt");
    d["prompt_helpers"] = json!([NO_PEOPLE]);
    let id = g.run(&d);
    let r = g.retry(id).unwrap();
    assert_eq!(r["prompt_helpers"], json!([NO_PEOPLE]));
    assert!(r.get("prompt_retry").is_none(), "nothing to badge");
    let mut list = g.dispatch("prm_helpers_list", json!({})).unwrap();
    for h in list.as_array_mut().unwrap() {
        if h["id"] == NO_PEOPLE {
            h["text"] = json!("Empty rooms.");
        }
    }
    g.dispatch("prm_helpers_save", json!({ "helpers": list }))
        .unwrap();
    let r = g.retry(id).unwrap();
    assert_eq!(
        r["prompt_retry"],
        json!({"since": "001", "changed": [NO_PEOPLE], "missing": []})
    );
    assert_eq!(r["prompt_helpers_changed"], json!([NO_PEOPLE]));
}

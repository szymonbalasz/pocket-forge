//! `generate` on Scenario (GEN §10, OQ-GEN-01): the generate overlay rows against the owner's
//! recorded `get-model` files (2026-11-08), `list_models{generate}`, and generate bodies over
//! replay (synthetic exchanges until the owner's `dry-run-generate` recordings are in).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;

use super::normalise::{is_ratio, normalise};
use super::overlay::{OverlayRow, SCENARIO_ROWS, row_for, unique_ids};
use super::record::content_hash;
use super::tests::{BASE, Rig, creds, ex, get_model};
use super::*;
use crate::credentials::MemoryStore;
use crate::fixtures::noise_png;
use crate::replay::{Exchange, ReplayTransport};
use crate::transport::{Method, RecordingSleeper};
use crate::{
    ControlRole, CostAmount, JobRequest, ModelFilter, Operation, OutputRequest, OutputSize,
    Provider, ProviderError, ReferenceImage, ReferenceRole, block_on,
};

const GPT2: &str = "model_openai-gpt-image-2";
const FLARE: &str = "model_openai-gpt-image-2-5-flare";
const SUNBURST: &str = "model_openai-gpt-image-2-5-sunburst";
const I45: &str = "model_ideogram-v4-5";
const PRECISE: &str = "model_ideogram-v4-5-precise-edit";
const V3B: &str = "model_ideogram-v3-balanced";
const V3Q: &str = "model_ideogram-v3-quality";
const V3T: &str = "model_ideogram-v3-turbo";
const CHAR: &str = "model_ideogram-character";

/// A rig with every overlay row (edit and generate) of these models.
fn rig_all(exchanges: Vec<Exchange>, models: &[&str]) -> Rig {
    let t = Arc::new(ReplayTransport::from_exchanges(BASE, exchanges));
    let dir = tempfile::tempdir().unwrap();
    let sleeps = Arc::new(RecordingSleeper::default());
    let rows = SCENARIO_ROWS
        .iter()
        .filter(|r| models.contains(&r.model_id))
        .copied()
        .collect();
    let p = ScenarioProvider::with_overlay(
        ScenarioConfig {
            base_url: BASE.into(),
            transport: t.clone(),
            sleeper: sleeps.clone(),
            credentials: Arc::new(MemoryStore::with(&creds())),
            cache_dir: Some(dir.path().join("cache/provider")),
        },
        rows,
    );
    Rig { p, t, sleeps, dir }
}

fn generate_caps(model: &str) -> crate::Capabilities {
    let row = row_for(&SCENARIO_ROWS, model, Operation::Generate).unwrap();
    let rec = fixture_recording(model, "get-model").unwrap();
    normalise(model, &rec.exchanges[0].response.body, Some(row))
        .unwrap()
        .capabilities
}

fn request(model: &str, output: OutputRequest) -> JobRequest {
    JobRequest {
        operation: Operation::Generate,
        model_id: model.into(),
        final_prompt: "flooded crypt, torchlight".into(),
        controls: BTreeMap::new(),
        references: vec![],
        region_png: vec![],
        mask_png: vec![],
        candidate_count: 1,
        seed: None,
        output: Some(output),
    }
}

fn reference(slot: &str, seed: u64) -> ReferenceImage {
    let bytes = noise_png(32, seed);
    ReferenceImage {
        slot: slot.into(),
        role: None,
        hash: format!("sha256:{}", content_hash(&bytes)),
        bytes,
    }
}

fn ratios(c: &crate::Capabilities) -> Vec<String> {
    match &c.output_size {
        Some(OutputSize::Aspect { allowed }) => allowed.clone(),
        other => panic!("{}: not aspect: {other:?}", c.model_id),
    }
}

#[test]
fn fr_gen_05_generate_rows_exist_only_for_the_seven_txt2img_models() {
    let mut with_row: Vec<&str> = SCENARIO_ROWS
        .iter()
        .filter(|r| r.operation == Operation::Generate)
        .map(|r| r.model_id)
        .collect();
    with_row.sort_unstable();
    let mut txt2img: Vec<&str> = unique_ids(&SCENARIO_ROWS)
        .into_iter()
        .filter(|m| {
            let rec = fixture_recording(m, "get-model").unwrap();
            rec.exchanges[0].response.body["model"]["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("txt2img"))
        })
        .collect();
    txt2img.sort_unstable();
    assert_eq!(with_row, txt2img);
    assert_eq!(with_row.len(), 7);
    // A generate row on an img2img-only model is still not offered (Character, Precise Edit).
    for model in [CHAR, PRECISE] {
        let row = OverlayRow {
            model_id: model,
            ..*row_for(&SCENARIO_ROWS, V3T, Operation::Generate).unwrap()
        };
        let rec = fixture_recording(model, "get-model").unwrap();
        let c = normalise(model, &rec.exchanges[0].response.body, Some(&row))
            .unwrap()
            .capabilities;
        assert!(c.operations.is_empty(), "{model}");
        assert!(c.warnings.iter().any(|w| w.contains("txt2img")), "{model}");
    }
}

/// One table over the generate rows and the recordings (GEN §10): output size, count,
/// transparency, reference slots and sketch slot.
#[test]
fn fr_gen_06_generate_capabilities_per_recorded_row() {
    type Slots = &'static [(&'static str, Option<ReferenceRole>, u32, bool)];
    let gpt_wh = Some(OutputSize::Wh {
        min: 16,
        max: 3840,
        step: 16,
    });
    let table: [(&str, Option<u32>, Option<bool>, Slots); 7] = [
        // model, count max, transparent offered (None = no background control), slots
        (
            GPT2,
            Some(10),
            Some(false),
            &[("referenceImages", None, 10, true)],
        ),
        (
            FLARE,
            Some(10),
            Some(true),
            &[("referenceImages", None, 16, true)],
        ),
        (
            SUNBURST,
            Some(10),
            Some(true),
            &[("referenceImages", None, 16, true)],
        ),
        (I45, Some(8), None, &[("referenceImages", None, 5, false)]),
        (
            V3B,
            None,
            None,
            &[("styleReferenceImages", Some(ReferenceRole::Style), 4, false)],
        ),
        (
            V3Q,
            None,
            None,
            &[("styleReferenceImages", Some(ReferenceRole::Style), 4, false)],
        ),
        (
            V3T,
            None,
            None,
            &[("styleReferenceImages", Some(ReferenceRole::Style), 4, false)],
        ),
    ];
    for (model, count, transparent, slots) in table {
        let c = generate_caps(model);
        assert_eq!(
            c.operations,
            [Operation::Generate],
            "{model}: {:?}",
            c.warnings
        );
        assert!(c.warnings.is_empty(), "{model}: {:?}", c.warnings);
        assert!(c.mask.is_none(), "{model}");
        // FR-GEN-06: no source, no mask; FR-GEN-11: the size inputs are never controls.
        for x in &c.controls {
            assert!(
                !matches!(x.role, ControlRole::Source | ControlRole::Mask),
                "{model}: {}",
                x.name
            );
            assert!(
                !["width", "height", "aspectRatio", "mask", "image"].contains(&x.name.as_str()),
                "{model}: {}",
                x.name
            );
        }
        let count_max = c
            .controls
            .iter()
            .find(|x| x.role == ControlRole::CandidateCount)
            .and_then(|x| x.max)
            .map(|m| m as u32);
        assert_eq!(count_max, count, "{model}: count (none → 1 per job)");
        let background = c
            .controls
            .iter()
            .find(|x| x.role == ControlRole::Background);
        assert_eq!(
            background.map(|b| b.allowed.contains(&json!("transparent"))),
            transparent,
            "{model}: background"
        );
        let got: Vec<_> = c
            .slots
            .iter()
            .map(|s| (s.name.as_str(), s.role, s.max, s.cost_affecting))
            .collect();
        assert_eq!(got, slots, "{model}: slots");
        assert!(
            c.structure.is_none(),
            "{model}: no sketch slot until a paid check (OQ-GEN-05)"
        );
        if model.contains("gpt") {
            assert_eq!(c.output_size, gpt_wh, "{model}");
        }
    }
    // Aspect models list only real ratios: 4.5 drops `auto` and `source` (23 left), V3 has 15.
    let r45 = ratios(&generate_caps(I45));
    assert_eq!(r45.len(), 23);
    assert!(!r45.iter().any(|r| r == "auto" || r == "source"));
    for model in [V3B, V3Q, V3T] {
        assert_eq!(ratios(&generate_caps(model)).len(), 15, "{model}");
    }
}

#[test]
fn fr_gen_09_only_w_colon_h_values_are_ratios() {
    for ok in ["1:1", "16:9", "23:9", "10:16"] {
        assert!(is_ratio(ok), "{ok}");
    }
    for bad in [
        "auto", "source", "None", "1:", ":1", "0:1", "1:0", "01:2", "1:2:3", "a:b",
    ] {
        assert!(!is_ratio(bad), "{bad}");
    }
}

#[test]
fn fr_gen_05_list_models_generate_gives_the_seven_txt2img_models() {
    let models = unique_ids(&SCENARIO_ROWS);
    let gets = || models.iter().map(|m| get_model(m)).collect::<Vec<_>>();
    let r = rig_all([gets(), gets()].concat(), &models);
    let list = |op| {
        block_on(r.p.list_models_cached(&ModelFilter {
            operation: Some(op),
        }))
        .unwrap()
        .models
    };
    let generate = list(Operation::Generate);
    let ids: Vec<_> = generate.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, [GPT2, FLARE, SUNBURST, I45, V3B, V3Q, V3T]);
    assert_eq!(
        generate[0].operations,
        [Operation::Generate, Operation::EditMasked],
        "one entry per model, every operation it offers"
    );
    assert_eq!(list(Operation::EditMasked).len(), 9, "edit is unchanged");
    assert_eq!(r.t.remaining(), 0);
    let cached = std::fs::read(r.dir.path().join("cache/provider/models.json")).unwrap();
    let file: serde_json::Value = serde_json::from_slice(&cached).unwrap();
    assert!(
        file["capabilities"][format!("{GPT2}@generate")]["output_size"].is_object(),
        "generate capabilities cached under their own key"
    );
}

#[test]
fn fr_gen_22_gpt_generate_body_sends_size_count_background_and_references() {
    let generate_path = format!("/generate/custom/{FLARE}?dryRun=true");
    let mut dry = ex(
        Method::Post,
        &generate_path,
        269,
        json!({"creativeUnitsCost": 12}),
    );
    dry.request.body = Some(json!({
        "background": "", "height": 0, "numOutputs": 0, "prompt": "",
        "referenceImages": [""], "width": 0,
    }));
    let r = rig_all(
        vec![
            get_model(FLARE),
            ex(Method::Post, "/assets", 200, json!({"asset": {"id": "r1"}})),
            dry,
        ],
        &[FLARE],
    );
    let mut req = request(FLARE, OutputRequest::Wh { w: 1520, h: 2032 });
    req.candidate_count = 2;
    req.controls
        .insert("background".into(), json!("transparent"));
    req.references = vec![reference("referenceImages", 1)];
    let est = block_on(r.p.estimate(&req)).unwrap();
    assert_eq!(est.cost.amount, CostAmount::Known(12.0));
    assert_eq!(
        est.provider.0["request"],
        json!({
            "background": "transparent", "height": 2032, "numOutputs": 2,
            "prompt": "flooded crypt, torchlight", "referenceImages": ["r1"], "width": 1520,
        }),
        "no source, no mask; the size from `output`"
    );
    assert!(est.provider.0["asset_refs"].get("image").is_none());
    assert_eq!(r.t.remaining(), 0);
}

#[test]
fn fr_gen_22_ideogram_generate_body_sends_the_ratio_and_style_references() {
    let generate_path = format!("/generate/custom/{V3T}");
    let mut submit = ex(
        Method::Post,
        &generate_path,
        200,
        json!({"job": {"jobId": "job_1", "status": "queued"}}),
    );
    submit.request.body = Some(json!({
        "aspectRatio": "", "prompt": "", "seed": 0, "styleReferenceImages": [""],
    }));
    let r = rig_all(
        vec![
            get_model(V3T),
            ex(Method::Post, "/assets", 200, json!({"asset": {"id": "s1"}})),
            submit,
        ],
        &[V3T],
    );
    let mut req = request(
        V3T,
        OutputRequest::Aspect {
            aspect: "2:3".into(),
        },
    );
    req.seed = Some(7);
    req.references = vec![reference("styleReferenceImages", 2)];
    let rec = block_on(r.p.submit(&req)).unwrap();
    assert_eq!(rec.0["job_handle"], "job_1");
    assert_eq!(
        rec.0["request"],
        json!({
            "aspectRatio": "2:3", "prompt": "flooded crypt, torchlight", "seed": 7,
            "styleReferenceImages": ["s1"],
        })
    );
    assert_eq!(
        rec.0["asset_refs"]["references"][0]["slot"],
        "styleReferenceImages"
    );
    assert!(rec.0["asset_refs"].get("mask").is_none());
    assert_eq!(r.t.remaining(), 0);
}

#[test]
fn fr_gen_08_output_backstop_with_zero_network_calls() {
    let cases = [
        (GPT2, Some(OutputRequest::Wh { w: 1000, h: 1024 })), // not a multiple of 16
        (GPT2, Some(OutputRequest::Wh { w: 4096, h: 1024 })), // over 3840
        (
            GPT2,
            Some(OutputRequest::Aspect {
                aspect: "1:1".into(),
            }),
        ),
        (
            V3B,
            Some(OutputRequest::Aspect {
                aspect: "auto".into(),
            }),
        ),
        (V3B, Some(OutputRequest::Wh { w: 1024, h: 1024 })),
        (V3B, None),
    ];
    for (model, output) in cases {
        // Only the capabilities call is recorded; any other request would panic the replay.
        let r = rig_all(vec![get_model(model)], &[model]);
        let mut req = request(model, OutputRequest::Wh { w: 0, h: 0 });
        req.output = output.clone();
        assert_eq!(
            block_on(r.p.estimate(&req)).unwrap_err(),
            ProviderError::InvalidRequest {
                field: "output".into()
            },
            "{model}: {output:?}"
        );
        assert_eq!(
            block_on(r.p.submit(&req)).unwrap_err(),
            ProviderError::InvalidRequest {
                field: "output".into()
            }
        );
        assert_eq!(r.t.seen().len(), 1);
    }
    // A model with no generate row: `generate` is refused as an operation.
    let r = rig_all(vec![get_model(CHAR), get_model(CHAR)], &[CHAR]);
    let req = request(
        CHAR,
        OutputRequest::Aspect {
            aspect: "1:1".into(),
        },
    );
    assert_eq!(
        block_on(r.p.get_capabilities(CHAR, Operation::Generate)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "operation".into()
        }
    );
    assert_eq!(
        block_on(r.p.estimate(&req)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "operation".into()
        }
    );
    assert_eq!(r.t.remaining(), 0);
}

#[test]
fn fr_gen_06_edit_rows_are_unchanged_by_the_generate_rows() {
    // The same model keeps its masked-edit capabilities (source, mask, fixed size inputs).
    let r = rig_all(vec![get_model(GPT2), get_model(GPT2)], &[GPT2]);
    let edit = block_on(r.p.get_capabilities(GPT2, Operation::EditMasked)).unwrap();
    assert_eq!(edit.operations, [Operation::EditMasked]);
    assert!(edit.output_size.is_none());
    assert!(
        edit.controls
            .iter()
            .any(|c| c.role == ControlRole::Source && c.name == "referenceImages")
    );
    let generate = block_on(r.p.get_capabilities(GPT2, Operation::Generate)).unwrap();
    assert!(
        generate
            .controls
            .iter()
            .any(|c| c.role == ControlRole::Reference && c.name == "referenceImages")
    );
    assert_eq!(r.t.seen().len(), 2, "one fetch per call");
}

//! Scenario provider tests over [`ReplayTransport`]: no network (AGENTS §3.7).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::overlay::{SCENARIO_ROWS, row_for, row_in};
use super::*;
use crate::credentials::MemoryStore;
use crate::replay::{Exchange, RecordedRequest, RecordedResponse, ReplayTransport};
use crate::transport::RecordingSleeper;
use crate::{Connectivity, ModelFilter, Operation, ProviderError, block_on};

pub(crate) const BASE: &str = api::DEFAULT_BASE_URL;

pub(crate) fn creds() -> Credentials {
    Credentials::new("test-key-0000wxyz", "test-secret-value")
}

pub(crate) struct Rig {
    pub p: ScenarioProvider,
    pub t: Arc<ReplayTransport>,
    pub sleeps: Arc<RecordingSleeper>,
    pub dir: tempfile::TempDir,
}

/// A provider over `exchanges`, with credentials, a temp cache dir and the given overlay rows.
pub(crate) fn rig_with(exchanges: Vec<Exchange>, models: &[&str]) -> Rig {
    let t = Arc::new(ReplayTransport::from_exchanges(BASE, exchanges));
    rig_over(t, models)
}

pub(crate) fn rig_over(t: Arc<ReplayTransport>, models: &[&str]) -> Rig {
    let rows = models
        .iter()
        .map(|m| *row_in(&SCENARIO_ROWS, m).unwrap())
        .collect();
    rig_rows(t, rows)
}

/// Every Scenario row (edit and generate) of these models.
pub(crate) fn all_rows_of(models: &[&str]) -> Vec<OverlayRow> {
    SCENARIO_ROWS
        .iter()
        .filter(|r| models.contains(&r.model_id))
        .copied()
        .collect()
}

/// A provider over `t` with exactly these overlay rows.
pub(crate) fn rig_rows(t: Arc<ReplayTransport>, rows: Vec<OverlayRow>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let sleeps = Arc::new(RecordingSleeper::default());
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

pub(crate) fn ex(method: Method, path: &str, status: u16, body: Value) -> Exchange {
    let (path, query) = crate::replay::split_query(path);
    Exchange {
        request: RecordedRequest {
            method,
            path,
            query,
            body: None,
        },
        response: RecordedResponse {
            status,
            headers: BTreeMap::new(),
            body,
        },
    }
}

/// The committed `get-model` exchange for a model.
pub(crate) fn get_model(model: &str) -> Exchange {
    fixture_recording(model, "get-model").unwrap().exchanges[0].clone()
}

const V3: &str = "model_ideogram-v3-balanced";
const V3Q: &str = "model_ideogram-v3-quality";
const GPT: &str = "model_openai-gpt-image-2-5-flare";
const CHAR: &str = "model_ideogram-character";

#[test]
fn fr_prv_25_list_models_returns_masked_edit_models_only() {
    let r = rig_with(
        vec![
            get_model(V3),
            ex(
                Method::Get,
                &format!("/models/{V3Q}"),
                404,
                json!({"reason": "Model not found"}),
            ),
            get_model(CHAR),
        ],
        &[V3, V3Q, CHAR],
    );
    let list = block_on(r.p.list_models_cached(&ModelFilter {
        operation: Some(Operation::EditMasked),
    }))
    .unwrap();
    assert!(!list.stale);
    let ids: Vec<_> = list.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        [V3, CHAR],
        "404 skipped; Character's required reference slot no longer blocks it (FR-PRV-32)"
    );
    assert_eq!(list.models[0].name, "Ideogram 3 (Balanced)");
    assert_eq!(r.t.remaining(), 0);
    assert!(
        r.t.seen().iter().all(|s| s.authorized),
        "API calls carry auth"
    );
    assert_eq!(r.p.connectivity(), Connectivity::Ok);

    let other = block_on(r.p.list_models_cached(&ModelFilter {
        operation: Some(Operation::Upscale),
    }))
    .unwrap();
    assert!(other.models.is_empty());
}

#[test]
fn fr_prv_25_offline_returns_the_cached_list_as_stale() {
    let offline = || ex(Method::Get, &format!("/models/{V3}"), 0, json!("offline"));
    let r = rig_with(vec![get_model(V3), offline(), offline(), offline()], &[V3]);
    block_on(r.p.list_models(&ModelFilter::default())).unwrap();
    let list = block_on(r.p.list_models_cached(&ModelFilter::default())).unwrap();
    assert!(list.stale);
    assert_eq!(list.models.len(), 1);
    assert_eq!(r.p.connectivity(), Connectivity::Offline);
    assert_eq!(r.sleeps.delays().len(), 2, "offline is retried (FR-PRV-24)");
}

/// GPT Image 2 left the overlay (2026-11-11): a cache written while it had a row never offers
/// it again, offline included, and its cached capabilities are never served (FR-PRV-25).
#[test]
fn fr_prv_25_a_model_removed_from_the_overlay_never_comes_back_from_the_cache() {
    const GONE: &str = "model_openai-gpt-image-2";
    let offline = || ex(Method::Get, &format!("/models/{V3}"), 0, json!("offline"));
    let r = rig_with(vec![get_model(V3), offline(), offline(), offline()], &[V3]);
    block_on(r.p.list_models(&ModelFilter::default())).unwrap();
    // The cache as an older build wrote it: the gone model listed, with its capabilities.
    let path = r.dir.path().join("cache/provider/models.json");
    let mut file: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut gone = file["models"][0].clone();
    gone["id"] = json!(GONE);
    file["models"].as_array_mut().unwrap().insert(0, gone);
    file["capabilities"][GONE] = file["capabilities"][V3].clone();
    std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();

    let list = block_on(r.p.list_models_cached(&ModelFilter::default())).unwrap();
    assert!(list.stale);
    let ids: Vec<_> = list.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, [V3]);
    assert_eq!(
        block_on(r.p.get_capabilities(GONE, Operation::EditMasked)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "model_id".into()
        }
    );
    assert_eq!(r.t.remaining(), 0, "no call for the gone model");
}

/// GPT Image 2.5 lost its masked-edit row (2026-11-11) but keeps its generate row: a cache
/// written while it had both never offers it for `edit_masked` again, offline included, and its
/// cached edit capabilities are never served; it is still offered for `generate` (FR-PRV-25).
#[test]
fn fr_prv_25_an_operation_removed_from_the_overlay_never_comes_back_from_the_cache() {
    // Table order: GPT Image 2.5 Flare's generate row comes first.
    let offline = || ex(Method::Get, &format!("/models/{GPT}"), 0, json!("offline"));
    let exchanges = vec![get_model(GPT), get_model(V3)];
    // Two offline lists, three attempts each (FR-PRV-24).
    let exchanges = [exchanges, (0..6).map(|_| offline()).collect()].concat();
    let r = rig_rows(
        Arc::new(ReplayTransport::from_exchanges(BASE, exchanges)),
        all_rows_of(&[V3, GPT]),
    );
    block_on(r.p.list_models(&ModelFilter::default())).unwrap();
    // The cache as an older build wrote it: GPT offering masked edit, with edit capabilities.
    let path = r.dir.path().join("cache/provider/models.json");
    let mut file: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let gpt = file["models"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|m| m["id"] == GPT)
        .unwrap();
    assert_eq!(
        gpt["operations"],
        json!(["generate"]),
        "fresh: generate only"
    );
    gpt["operations"] = json!(["generate", "edit_masked"]);
    let edit_key = super::cache::key(GPT, Operation::EditMasked);
    file["capabilities"][&edit_key] = file["capabilities"][V3].clone();
    std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();

    let list = |op| {
        block_on(r.p.list_models_cached(&ModelFilter {
            operation: Some(op),
        }))
        .unwrap()
    };
    let edit = list(Operation::EditMasked);
    assert!(edit.stale);
    let ids: Vec<_> = edit.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, [V3]);
    let generate = list(Operation::Generate);
    let gpt = generate.models.iter().find(|m| m.id == GPT).unwrap();
    assert_eq!(gpt.operations, [Operation::Generate]);
    assert_eq!(
        block_on(r.p.get_capabilities(GPT, Operation::EditMasked)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "operation".into()
        }
    );
    assert_eq!(r.t.remaining(), 0, "no call for the removed operation");
}

/// GPT Image's masked edit is refused before any network call, so neither a list, a
/// capabilities fetch nor a job can use it (FR-PRV-11, paid checks of 2026-11-11).
#[test]
fn fr_prv_11_gpt_image_offers_no_masked_edit() {
    for model in [
        GPT,
        "model_openai-gpt-image-2-5-sunburst",
        "model_openai-gpt-image-2-6-flare",
        "model_openai-gpt-image-2-6-sunburst",
    ] {
        assert!(
            row_for(&SCENARIO_ROWS, model, Operation::EditMasked).is_none(),
            "{model}: no edit row"
        );
        assert!(
            row_for(&SCENARIO_ROWS, model, Operation::Generate).is_some(),
            "{model}: generate row kept"
        );
    }
    let r = rig_rows(
        Arc::new(ReplayTransport::from_exchanges(BASE, vec![])),
        all_rows_of(&[GPT]),
    );
    assert_eq!(
        block_on(r.p.get_capabilities(GPT, Operation::EditMasked)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "operation".into()
        }
    );
    assert!(r.t.seen().is_empty());
}

#[test]
fn fr_prv_25_offline_without_a_cache_is_offline() {
    let offline = || ex(Method::Get, &format!("/models/{V3}"), 0, json!("offline"));
    let r = rig_with(vec![offline(), offline(), offline()], &[V3]);
    assert_eq!(
        block_on(r.p.list_models(&ModelFilter::default())).unwrap_err(),
        ProviderError::Offline
    );
}

#[test]
fn fr_prv_25_a_corrupt_cache_is_deleted_and_refetched() {
    let offline = || ex(Method::Get, &format!("/models/{V3}"), 0, json!("offline"));
    let r = rig_with(vec![offline(), offline(), offline(), get_model(V3)], &[V3]);
    let path = r.dir.path().join("cache/provider/models.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{ corrupt").unwrap();
    assert_eq!(
        block_on(r.p.list_models(&ModelFilter::default())).unwrap_err(),
        ProviderError::Offline
    );
    assert!(!path.exists(), "corrupt cache deleted");
    let list = block_on(r.p.list_models_cached(&ModelFilter::default())).unwrap();
    assert!(!list.stale);
    let cached: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(cached["models"][0]["id"], V3);
    assert!(cached["capabilities"][V3]["controls"].is_array());
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(leftovers.len(), 1, "no temp files left: {leftovers:?}");
}

#[test]
fn fr_prv_10_get_capabilities_normalises_and_falls_back_to_the_cache_offline() {
    const FILL: &str = "model_bfl-flux-1-pro-fill";
    let offline = || ex(Method::Get, &format!("/models/{FILL}"), 0, json!("offline"));
    let r = rig_with(
        vec![get_model(FILL), offline(), offline(), offline()],
        &[FILL],
    );
    let caps = block_on(r.p.get_capabilities(FILL, Operation::EditMasked)).unwrap();
    assert_eq!(
        caps.mask.unwrap().convention,
        crate::MaskConvention::WhiteEdit
    );
    assert_eq!(caps.input_px.max, 2048);
    let again = block_on(r.p.get_capabilities(FILL, Operation::EditMasked)).unwrap();
    assert_eq!(again, caps);
}

#[test]
fn get_capabilities_of_an_unknown_model_is_invalid_model_id() {
    let r = rig_with(vec![ex(Method::Get, "/models/nope", 404, json!({}))], &[]);
    assert_eq!(
        block_on(r.p.get_capabilities("nope", Operation::EditMasked)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "model_id".into()
        }
    );
}

#[test]
fn fr_prv_07_connectivity_follows_auth_and_offline_and_clears() {
    let r = rig_with(
        vec![
            ex(
                Method::Get,
                "/models?pageSize=1",
                401,
                json!({"reason": "Unauthorized"}),
            ),
            ex(
                Method::Get,
                "/models?pageSize=1",
                200,
                json!({"models": []}),
            ),
        ],
        &[],
    );
    assert_eq!(
        block_on(r.p.test_connection()).unwrap_err(),
        ProviderError::Auth
    );
    assert_eq!(r.p.connectivity(), Connectivity::AuthError);
    r.p.credentials_changed();
    assert_eq!(r.p.connectivity(), Connectivity::Unknown);
    block_on(r.p.test_connection()).unwrap();
    assert_eq!(r.p.connectivity(), Connectivity::Ok);
}

#[test]
fn fr_prv_29_project_id_goes_on_api_calls() {
    let r = rig_with(
        vec![ex(
            Method::Get,
            "/models?pageSize=1&projectId=proj_1",
            200,
            json!({}),
        )],
        &[],
    );
    r.p.set_account(ProviderAccount {
        team_id: Some("team_1".into()),
        project_id: Some("proj_1".into()),
    });
    block_on(r.p.test_connection()).unwrap();
    assert_eq!(r.t.remaining(), 0);
}

#[test]
fn malformed_model_json_skips_the_model_never_panics() {
    let mut bad = get_model(V3);
    bad.response.body = json!({"model": {"inputs": "nope"}});
    let r = rig_with(vec![bad], &[V3]);
    let list = block_on(r.p.list_models(&ModelFilter::default())).unwrap();
    assert!(list.is_empty());
}

/// A dry run of a masked edit over `model`'s fixture: the body as sent.
fn dry_run_body(model: &str) -> serde_json::Map<String, Value> {
    dry_run_body_over(model, *row_in(&SCENARIO_ROWS, model).unwrap())
}

/// [`dry_run_body`] with an explicit edit row.
fn dry_run_body_over(model: &str, row: OverlayRow) -> serde_json::Map<String, Value> {
    let exchanges = vec![
        get_model(model),
        ex(
            Method::Post,
            "/assets",
            200,
            json!({"asset": {"id": "a_img"}}),
        ),
        ex(
            Method::Post,
            "/assets",
            200,
            json!({"asset": {"id": "a_mask"}}),
        ),
        ex(
            Method::Post,
            &format!("/generate/custom/{model}?dryRun=true"),
            269,
            json!({"creativeUnitsCost": 3}),
        ),
    ];
    let r = rig_rows(
        Arc::new(ReplayTransport::from_exchanges(BASE, exchanges)),
        vec![row],
    );
    let request = crate::JobRequest {
        operation: Operation::EditMasked,
        model_id: model.into(),
        final_prompt: "a mossy flagstone floor".into(),
        controls: BTreeMap::new(),
        references: vec![],
        region_png: crate::fixtures::room_png(256),
        mask_png: crate::fixtures::disc_mask_png(256),
        candidate_count: 1,
        seed: Some(7),
        output: None,
    };
    let est = block_on(r.p.estimate(&request)).unwrap();
    assert_eq!(r.t.remaining(), 0);
    est.provider.0["request"].as_object().unwrap().clone()
}

/// An edit row like GPT Image's (withheld since 2026-11-11, `overlay::gpt`) takes the source as
/// the first of `referenceImages` (the mask applies to it) and gets the sent size as
/// `width`/`height`, which are never controls (FR-PRV-17, R28).
#[test]
fn fr_prv_17_gpt_source_is_the_first_reference_image_at_the_sent_size() {
    let row = super::overlay::gpt(GPT);
    let caps = super::normalise::normalise(GPT, &get_model(GPT).response.body, Some(&row)).unwrap();
    assert!(caps.capabilities.controls.iter().all(|c| c.name != "width"));
    assert_eq!(
        caps.capabilities.input_px.max, 3840,
        "a list's max is a count"
    );
    let body = dry_run_body_over(GPT, row);
    assert_eq!(body["referenceImages"], json!(["a_img"]));
    assert_eq!(body["mask"], json!("a_mask"));
    assert_eq!(
        (&body["width"], &body["height"]),
        (&json!(256), &json!(256))
    );
    assert!(!body.contains_key("image"));
}

/// Ideogram 4.5: the source goes first in `referenceImages`, and a mask needs
/// `aspectRatio: "auto"`, which the overlay fixes (no control).
#[test]
fn fr_prv_17_ideogram_4_5_fixes_aspect_ratio_auto() {
    let body = dry_run_body("model_ideogram-v4-5");
    assert_eq!(body["referenceImages"], json!(["a_img"]));
    assert_eq!(body["aspectRatio"], json!("auto"));
    // V3 takes a plain `image` and nothing fixed.
    let body = dry_run_body(V3);
    assert_eq!(body["image"], json!("a_img"));
    assert!(!body.contains_key("aspectRatio") && !body.contains_key("width"));
}

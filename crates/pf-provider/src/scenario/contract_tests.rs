//! Contract tests (spec §13): the **same suite** runs against the fake provider and against
//! `ScenarioProvider` over replay recordings, plus one test per slice-4 FR over the recordings
//! in `fixtures/provider/model_ideogram-v3-balanced/`.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::overlay::{SCENARIO_ROWS, row_in};
use super::tests::{BASE, creds, ex, get_model, rig_over, rig_with};
use super::*;
use crate::credentials::MemoryStore;
use crate::fake::FakeProvider;
use crate::fixtures::{disc_mask_png, noise_png, room_png};
use crate::pixels::{decode_rgb8, png_size};
use crate::replay::ReplayTransport;
use crate::transport::{PanicTransport, RecordingSleeper};
use crate::{
    CancelAck, ControlRole, CostAmount, JobRequest, ModelFilter, Operation, Provider,
    ProviderError, ProviderJobState, block_on,
};

const V3: &str = "model_ideogram-v3-balanced";

fn request(model: &str) -> JobRequest {
    JobRequest {
        operation: Operation::EditMasked,
        model_id: model.into(),
        final_prompt: "a mossy flagstone floor".into(),
        controls: BTreeMap::new(),
        references: vec![],
        region_png: room_png(256),
        mask_png: disc_mask_png(256),
        candidate_count: 2,
        seed: Some(7),
        output: None,
    }
}

/// The generic outputs every provider must give for one masked edit (spec §13 contract).
fn contract_suite<P: Provider>(p: &P, model: &str) -> crate::Candidates {
    let models = block_on(p.list_models(&ModelFilter {
        operation: Some(Operation::EditMasked),
    }))
    .unwrap();
    assert!(models.iter().any(|m| m.id == model), "{model} listed");
    assert!(
        models
            .iter()
            .all(|m| m.operations.contains(&Operation::EditMasked))
    );

    let caps = block_on(p.get_capabilities(model, Operation::EditMasked)).unwrap();
    assert_eq!(caps.operations, [Operation::EditMasked]);
    assert!(caps.mask.is_some());
    let roles: Vec<_> = caps.controls.iter().map(|c| c.role).collect();
    for r in [
        ControlRole::Source,
        ControlRole::Mask,
        ControlRole::Prompt,
        ControlRole::CandidateCount,
    ] {
        assert!(roles.contains(&r), "{r:?}");
    }

    let req = request(model);
    let est = block_on(p.estimate(&req)).unwrap();
    assert!(matches!(est.cost.amount, CostAmount::Known(v) if v > 0.0));
    assert!(est.provider.0.get("job_handle").is_none());

    let mut rec = block_on(p.submit(&req)).unwrap();
    assert!(rec.0["job_handle"].is_string());
    assert_eq!(rec.0["v"], 1);
    let mut seen = vec![];
    for _ in 0..10 {
        let st = block_on(p.status(&rec)).unwrap();
        assert!((0.0..=1.0).contains(&st.progress));
        seen.push(st.state);
        rec = st.provider;
        if st.state == ProviderJobState::Succeeded {
            assert!(st.cost.is_some(), "cost on success");
            break;
        }
    }
    assert_eq!(
        seen,
        [
            ProviderJobState::Queued,
            ProviderJobState::Running,
            ProviderJobState::Succeeded
        ]
    );

    let c = block_on(p.fetch_candidates(&rec)).unwrap();
    assert_eq!(c.images.len(), 2);
    for img in &c.images {
        assert_eq!(
            png_size(&img.bytes),
            Some((256, 256)),
            "candidates at the sent size"
        );
        assert!(decode_rgb8(&img.bytes).is_some());
        assert!(img.provider_meta["asset_id"].is_string());
        assert!(img.seed.is_some());
    }
    assert!(
        matches!(block_on(p.balance()).unwrap(), crate::Balance::Known { remaining, .. } if remaining > 0.0)
    );
    c
}

fn recording(case: &str) -> Arc<ReplayTransport> {
    Arc::new(ReplayTransport::new(
        BASE,
        &fixture_recording(V3, case).unwrap(),
    ))
}

fn scenario(case: &str) -> super::tests::Rig {
    rig_over(recording(case), &[V3])
}

#[test]
fn contract_suite_fake() {
    let c = contract_suite(&FakeProvider::default(), "fake-black-edit");
    assert!(c.provider.0["asset_refs"]["image"].is_string());
}

#[test]
fn contract_suite_scenario_replay() {
    let r = scenario("contract");
    r.p.set_account(ProviderAccount {
        team_id: Some("<team-id>".into()),
        project_id: None,
    });
    let c = contract_suite(&r.p, V3);
    assert_eq!(r.t.remaining(), 0, "every recorded exchange was used");
    // FR-PRV-09: auth on API calls only, never on signed URLs.
    for s in r.t.seen() {
        assert_eq!(s.authorized, !s.path.starts_with("fake://"), "{s:?}");
    }
    assert_eq!(c.images[0].provider_meta["asset_id"], "<asset-3>");
    assert_eq!(c.images[1].seed, Some(8));
    assert_eq!(c.images[0].provider_meta["inputs"]["seed"], 7);
}

#[test]
fn fr_prv_22_balance_is_plan_plus_one_shot_with_the_period_end() {
    let r = rig_with(
        vec![ex(
            Method::Get,
            "/teams/team_1/limits",
            200,
            json!({
                "limits": {"creativeUnits": 1500}, "values": {"creativeUnits": 300, "periodEnd": "2026-12-01T00:00:00Z"},
                "oneShotCreativeUnits": [{"remaining": 30}, {"remaining": 4}],
            }),
        )],
        &[],
    );
    r.p.set_account(ProviderAccount {
        team_id: Some("team_1".into()),
        project_id: None,
    });
    assert_eq!(
        block_on(r.p.balance()).unwrap(),
        crate::Balance::Known {
            remaining: 1234.0,
            unit: "CU".into(),
            period_end: Some("2026-12-01T00:00:00Z".into())
        }
    );
}

#[test]
fn fr_prv_22_no_team_id_or_401_403_is_unknown() {
    let r = rig_with(vec![], &[]);
    assert_eq!(
        block_on(r.p.balance()).unwrap(),
        crate::Balance::Unknown,
        "no call without a team id"
    );
    for status in [401, 403] {
        let r = rig_with(
            vec![ex(Method::Get, "/teams/t/limits", status, json!({}))],
            &[],
        );
        r.p.set_account(ProviderAccount {
            team_id: Some("t".into()),
            project_id: None,
        });
        assert_eq!(block_on(r.p.balance()).unwrap(), crate::Balance::Unknown);
    }
}

#[test]
fn fr_prv_14_large_files_use_multipart_and_upload_once_per_hash() {
    let r = scenario("upload-multipart");
    let big = noise_png(1200, 7);
    assert!(big.len() >= jobs::SINGLE_UPLOAD_LIMIT);
    let a = block_on(r.p.upload(&big, UploadPurpose::Source)).unwrap();
    assert_eq!(a.0, "<asset-1>");
    assert_eq!(
        block_on(r.p.upload(&big, UploadPurpose::Source)).unwrap(),
        a,
        "no second upload"
    );
    assert_eq!(r.t.remaining(), 0);
    let put =
        r.t.seen()
            .into_iter()
            .find(|s| s.method == Method::Put)
            .unwrap();
    assert!(!put.authorized, "presigned part: no auth (FR-PRV-09)");
    assert_eq!(
        r.sleeps.delays().len(),
        1,
        "polled the validating upload once"
    );
}

/// A multipart upload by hand: create, one part, complete, then `GET /uploads/{id}` per poll.
fn multipart(polls: Vec<Value>) -> super::tests::Rig {
    let upload = |status: &str| json!({"upload": {"id": "up1", "status": status}});
    let mut x = vec![
        ex(
            Method::Post,
            "/uploads",
            200,
            json!({"upload": {"id": "up1", "status": "pending",
                "parts": [{"number": 1, "url": "fake://upload/1/part/1", "expires": "later"}]}}),
        ),
        ex(Method::Put, "fake://upload/1/part/1", 200, Value::Null),
        ex(
            Method::Post,
            "/uploads/up1/action",
            200,
            upload("validating"),
        ),
    ];
    x.extend(
        polls
            .into_iter()
            .map(|b| ex(Method::Get, "/uploads/up1", 200, b)),
    );
    rig_with(x, &[])
}

#[test]
fn fr_prv_14_a_slow_validating_upload_is_polled_with_backoff_until_it_names_its_asset() {
    let mut polls = vec![json!({"upload": {"id": "up1", "status": "validating"}}); 5];
    polls.push(json!({"upload": {"id": "up1", "status": "validated"}}));
    polls.push(json!({"upload": {"id": "up1", "status": "imported", "entityId": "asset_Big"}}));
    let r = multipart(polls);
    let a = block_on(r.p.upload(&noise_png(1200, 7), UploadPurpose::Source)).unwrap();
    assert_eq!(a.0, "asset_Big", "the asset id is the upload's entityId");
    assert_eq!(r.t.remaining(), 0);
    let secs: Vec<u64> = r.sleeps.delays().iter().map(|d| d.as_secs()).collect();
    assert_eq!(secs, [1, 2, 4, 8, 8, 8, 8], "1 s, then doubling to 8 s");
}

#[test]
fn fr_prv_14_an_upload_that_never_validates_gives_up_after_about_a_minute() {
    let polls = vec![json!({"upload": {"id": "up1", "status": "validating"}}); 10];
    let r = multipart(polls);
    let e = block_on(r.p.upload(&noise_png(1200, 7), UploadPurpose::Source)).unwrap_err();
    let ProviderError::ProviderFailure { hint } = e else {
        panic!("{e:?}")
    };
    assert_eq!(
        hint,
        "the upload did not complete within 60 s (last status: validating)"
    );
    let total: u64 = r.sleeps.delays().iter().map(|d| d.as_secs()).sum();
    assert!((60..=70).contains(&total), "waited {total} s");
}

#[test]
fn fr_prv_14_a_failed_upload_surfaces_the_provider_error_message() {
    let r = multipart(vec![json!({"upload": {"id": "up1", "status": "failed",
        "errorMessage": "Image dimensions exceed the limit"}})]);
    let e = block_on(r.p.upload(&noise_png(1200, 7), UploadPurpose::Source)).unwrap_err();
    let ProviderError::ProviderFailure { hint } = e else {
        panic!("{e:?}")
    };
    assert_eq!(
        hint,
        "the provider rejected the upload: Image dimensions exceed the limit"
    );
    assert_eq!(r.t.remaining(), 0, "stopped polling at failed");
}

#[test]
fn fr_prv_14_a_finished_upload_without_an_asset_id_is_a_readable_error() {
    let r = multipart(vec![json!({"upload": {"id": "up1", "status": "complete"}})]);
    let e = block_on(r.p.upload(&noise_png(1200, 7), UploadPurpose::Source)).unwrap_err();
    assert_eq!(
        e,
        ProviderError::ProviderFailure {
            hint: "the upload finished (complete) without an asset id".into()
        }
    );
}

#[test]
fn fr_prv_14_small_files_use_post_assets_and_the_ref_cache_clears_on_credential_change() {
    let up = |n: u32| {
        ex(
            Method::Post,
            "/assets",
            200,
            json!({"asset": {"id": format!("a{n}")}}),
        )
    };
    let r = rig_with(vec![up(1), up(2)], &[]);
    let small = room_png(64);
    assert_eq!(
        block_on(r.p.upload(&small, UploadPurpose::Source))
            .unwrap()
            .0,
        "a1"
    );
    assert_eq!(
        block_on(r.p.upload(&small, UploadPurpose::Source))
            .unwrap()
            .0,
        "a1"
    );
    r.p.credentials_changed();
    assert_eq!(
        block_on(r.p.upload(&small, UploadPurpose::Source))
            .unwrap()
            .0,
        "a2"
    );
    assert_eq!(r.t.remaining(), 0);
}

#[test]
fn fr_prv_15_a_missing_asset_is_reuploaded_once_and_retried_once() {
    let r = scenario("reupload");
    let rec = block_on(r.p.submit(&request(V3))).unwrap();
    assert_eq!(rec.0["job_handle"], "<job-1>");
    assert_eq!(
        rec.0["asset_refs"]["image"], "<asset-3>",
        "new refs on the record"
    );
    assert_eq!(rec.0["asset_refs"]["mask"], "<asset-4>");
    assert_eq!(r.t.remaining(), 0);
}

#[test]
fn fr_prv_15_a_second_missing_asset_is_invalid_request() {
    let mut exchanges = fixture_recording(V3, "reupload").unwrap().exchanges;
    let last = exchanges.len() - 1;
    exchanges[last].response = exchanges[3].response.clone();
    let r = rig_over(
        Arc::new(ReplayTransport::from_exchanges(BASE, exchanges)),
        &[V3],
    );
    assert_eq!(
        block_on(r.p.submit(&request(V3))).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "image".into()
        }
    );
}

#[test]
fn fr_prv_16_dry_run_with_assets_uploads_first_and_never_sets_ip_detection() {
    // The contract recording: two uploads, then `?dryRun=true` (query checked exactly).
    let r = scenario("contract");
    block_on(r.p.get_capabilities(V3, Operation::EditMasked)).unwrap();
    block_on(r.p.get_capabilities(V3, Operation::EditMasked)).unwrap();
    let est = block_on(r.p.estimate(&request(V3))).unwrap();
    assert_eq!(est.cost.amount, CostAmount::Known(6.0));
    assert_eq!(est.cost.unit, "CU");
    assert_eq!(est.provider.0["cost_raw"]["perOutput"], 3);
    let seen = r.t.seen();
    assert_eq!(seen.iter().filter(|s| s.path == "/assets").count(), 2);
    assert!(est.provider.0["request"].get("ipDetection").is_none());
}

#[test]
fn fr_prv_16_ip_detection_is_stripped_even_if_the_model_lists_it() {
    let mut model = get_model(V3);
    model.response.body["model"]["inputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "ipDetection", "type": "boolean", "default": false}));
    let up = |n: u32| {
        ex(
            Method::Post,
            "/assets",
            200,
            json!({"asset": {"id": format!("a{n}")}}),
        )
    };
    let mut dry = ex(
        Method::Post,
        &format!("/generate/custom/{V3}?dryRun=true"),
        269,
        json!({"creativeUnitsCost": 3}),
    );
    dry.request.body = Some(json!({"image": "", "mask": "", "prompt": "", "seed": 0})); // real V3: no numOutputs
    let r = rig_with(vec![model, up(1), up(2), dry], &[V3]);
    let mut req = request(V3);
    req.controls.insert("ipDetection".into(), json!(true));
    let est = block_on(r.p.estimate(&req)).unwrap();
    assert!(est.provider.0["request"].get("ipDetection").is_none());
}

#[test]
fn fr_prv_16_without_dry_run_assets_estimate_uploads_nothing_and_unknown_cost() {
    let mut row = *row_in(&SCENARIO_ROWS, V3).unwrap();
    row.dry_run_assets = false;
    let t = recording("dry-run-no-assets");
    let p = ScenarioProvider::with_overlay(
        ScenarioConfig {
            base_url: BASE.into(),
            transport: t.clone(),
            sleeper: Arc::new(RecordingSleeper::default()),
            credentials: Arc::new(MemoryStore::with(&creds())),
            cache_dir: None,
        },
        vec![row],
    );
    let est = block_on(p.estimate(&request(V3))).unwrap();
    assert_eq!(est.cost.amount, CostAmount::Unknown);
    assert_eq!(
        t.seen().iter().filter(|s| s.path == "/assets").count(),
        0,
        "uploads wait for submit"
    );
    block_on(p.submit(&request(V3))).unwrap();
    assert_eq!(t.remaining(), 0);
}

#[test]
fn fr_prv_17_submit_is_never_retried_and_insufficient_balance_is_distinct() {
    let up = |n: u32| {
        ex(
            Method::Post,
            "/assets",
            200,
            json!({"asset": {"id": format!("a{n}")}}),
        )
    };
    let gen_path = format!("/generate/custom/{V3}");
    let r = rig_with(
        vec![
            get_model(V3),
            up(1),
            up(2),
            ex(Method::Post, &gen_path, 503, json!({"message": "busy"})),
        ],
        &[V3],
    );
    assert_eq!(
        block_on(r.p.submit(&request(V3))).unwrap_err().code(),
        "timeout",
        "a 5xx after sending may have created the job (R102)"
    );
    assert_eq!(r.t.remaining(), 0);
    assert!(
        r.sleeps.delays().is_empty(),
        "no retry once the request was sent"
    );

    let r = rig_with(
        vec![
            get_model(V3),
            up(1),
            up(2),
            ex(
                Method::Post,
                &gen_path,
                402,
                json!({"reason": "Insufficient credits"}),
            ),
        ],
        &[V3],
    );
    assert_eq!(
        block_on(r.p.submit(&request(V3))).unwrap_err(),
        ProviderError::InsufficientBalance
    );
}

/// Review item 1 / R102: once the submit was sent, anything but a clear refusal may have
/// created (and billed) the job, so it is `timeout` (RGN: `lost` + possible charge), never an
/// error RGN turns into `failed` + Retry.
#[test]
fn fr_prv_17_a_submit_the_provider_may_have_accepted_is_a_timeout() {
    let up = |n: u32| {
        ex(
            Method::Post,
            "/assets",
            200,
            json!({"asset": {"id": format!("a{n}")}}),
        )
    };
    let gen_path = format!("/generate/custom/{V3}");
    let cases: [(u16, Value); 7] = [
        (0, json!("interrupted")),
        (500, json!({"reason": "boom"})),
        (502, json!({"message": "bad gateway"})),
        (503, json!({"message": "busy"})),
        (504, json!({"message": "gateway timeout"})),
        (200, json!({"job": {"status": "queued"}})),
        (200, json!("not json {")),
    ];
    for (status, body) in cases {
        let r = rig_with(
            vec![
                get_model(V3),
                up(1),
                up(2),
                ex(Method::Post, &gen_path, status, body.clone()),
            ],
            &[V3],
        );
        assert_eq!(
            block_on(r.p.submit(&request(V3))).unwrap_err(),
            ProviderError::Timeout,
            "status {status} {body}"
        );
        assert_eq!(r.t.remaining(), 0);
        assert!(r.sleeps.delays().is_empty(), "never retried");
    }
    // A connection that never opened sent nothing: still `offline` (→ `failed`).
    let r = rig_with(
        vec![
            get_model(V3),
            up(1),
            up(2),
            ex(Method::Post, &gen_path, 0, json!("offline")),
        ],
        &[V3],
    );
    assert_eq!(
        block_on(r.p.submit(&request(V3))).unwrap_err(),
        ProviderError::Offline
    );
    // An interrupted idempotent call is retried, then `offline`.
    let r = rig_with(
        vec![
            get_model(V3),
            up(1),
            up(2),
            ex(
                Method::Post,
                &format!("{gen_path}?dryRun=true"),
                0,
                json!("interrupted"),
            ),
            ex(
                Method::Post,
                &format!("{gen_path}?dryRun=true"),
                0,
                json!("interrupted"),
            ),
            ex(
                Method::Post,
                &format!("{gen_path}?dryRun=true"),
                0,
                json!("interrupted"),
            ),
        ],
        &[V3],
    );
    assert_eq!(
        block_on(r.p.estimate(&request(V3))).unwrap_err(),
        ProviderError::Offline
    );
}

#[test]
fn fr_prv_17_the_body_maps_controls_by_role_and_name() {
    let r = scenario("contract");
    block_on(r.p.get_capabilities(V3, Operation::EditMasked)).unwrap();
    block_on(r.p.get_capabilities(V3, Operation::EditMasked)).unwrap();
    let mut req = request(V3);
    req.controls.insert("styleType".into(), json!("DESIGN"));
    // styleType = DESIGN makes colorPalette required (the fixture's conditional rule).
    assert_eq!(
        block_on(r.p.estimate(&req)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "colorPalette".into()
        }
    );
    req.controls.insert("nope".into(), json!(1));
    assert_eq!(
        block_on(r.p.estimate(&req)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "nope".into()
        }
    );
}

/// `estimate` and `submit` can return every normalised error (FR-PRV-23, R20).
#[test]
fn fr_prv_23_estimate_and_submit_return_every_normalised_error() {
    let up = |n: u32| {
        ex(
            Method::Post,
            "/assets",
            200,
            json!({"asset": {"id": format!("a{n}")}}),
        )
    };
    let cases: [(u16, Value, ProviderError); 8] = [
        (401, json!({}), ProviderError::Auth),
        (402, json!({}), ProviderError::InsufficientBalance),
        (
            429,
            json!({}),
            ProviderError::RateLimited {
                retry_after_ms: 1000,
            },
        ),
        (
            422,
            json!({"details": {"field": "prompt"}}),
            ProviderError::InvalidRequest {
                field: "prompt".into(),
            },
        ),
        (
            409,
            json!({"reason": "boom"}),
            ProviderError::ProviderFailure {
                hint: "boom".into(),
            },
        ),
        // A submit's 5xx may have created the job: `timeout` (R102, review item 1).
        (
            500,
            json!({"reason": "boom"}),
            ProviderError::ProviderFailure {
                hint: "boom".into(),
            },
        ),
        (0, json!("offline"), ProviderError::Offline),
        (0, json!("timeout"), ProviderError::Timeout),
    ];
    for (status, body, want) in cases {
        for dry in [true, false] {
            let path = format!(
                "/generate/custom/{V3}{}",
                if dry { "?dryRun=true" } else { "" }
            );
            let retried = dry && (status == 429 || status >= 500 || status == 0);
            let attempts = if retried { 3 } else { 1 };
            let mut exchanges = vec![get_model(V3), up(1), up(2)];
            exchanges.extend((0..attempts).map(|_| ex(Method::Post, &path, status, body.clone())));
            let r = rig_with(exchanges, &[V3]);
            let got = if dry {
                block_on(r.p.estimate(&request(V3)))
                    .map(|_| ())
                    .unwrap_err()
            } else {
                block_on(r.p.submit(&request(V3))).map(|_| ()).unwrap_err()
            };
            let want = if !dry && status >= 500 {
                ProviderError::Timeout
            } else {
                want.clone()
            };
            assert_eq!(got, want, "status {status}, dry run {dry}");
            assert_eq!(r.t.remaining(), 0);
        }
    }
}

#[test]
fn fr_prv_13_limits_are_checked_before_any_network_call() {
    // Only the capabilities fetch is recorded: one more request would panic the replay.
    let cases: [(Vec<u8>, Vec<u8>, &str); 4] = [
        (room_png(250), disc_mask_png(250), "image"), // not a multiple of 8
        (room_png(128), disc_mask_png(128), "image"), // below min 256
        (room_png(256), disc_mask_png(128), "mask"),  // mask size ≠ sent size
        (b"not a png".to_vec(), disc_mask_png(256), "image"),
    ];
    for (img, mask, field) in cases {
        let r = rig_with(vec![get_model(V3)], &[V3]);
        let mut req = request(V3);
        (req.region_png, req.mask_png) = (img, mask);
        assert_eq!(
            block_on(r.p.estimate(&req)).unwrap_err(),
            ProviderError::InvalidRequest {
                field: field.into()
            }
        );
        assert_eq!(
            block_on(r.p.submit(&req)).unwrap_err(),
            ProviderError::InvalidRequest {
                field: field.into()
            }
        );
    }
    // A file over max_bytes: GPT Image's max is 50 MB; shrink it through the source's maxSize.
    let mut model = get_model(V3);
    let inputs = model.response.body["model"]["inputs"]
        .as_array_mut()
        .unwrap();
    let image = inputs.iter_mut().find(|i| i["name"] == "image").unwrap();
    image["maxSize"] = json!(1000);
    let r = rig_with(vec![model], &[V3]);
    assert_eq!(
        block_on(r.p.estimate(&request(V3))).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "image".into()
        }
    );
}

#[test]
fn fr_prv_13_the_fake_backstops_too_with_no_transport() {
    let _no_network = PanicTransport;
    let p = FakeProvider::default();
    let mut req = request("fake-white-edit");
    req.region_png = room_png(70);
    req.mask_png = disc_mask_png(70);
    assert_eq!(
        block_on(p.estimate(&req)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "image".into()
        }
    );
}

#[test]
fn fr_prv_18_status_map_progress_clamp_and_cost() {
    let states = [
        ("queued", ProviderJobState::Queued),
        ("warming-up", ProviderJobState::Queued),
        ("pending", ProviderJobState::Queued),
        ("in-progress", ProviderJobState::Running),
        ("finalizing", ProviderJobState::Running),
        ("success", ProviderJobState::Succeeded),
        ("failure", ProviderJobState::Failed),
        ("canceled", ProviderJobState::Canceled),
        ("something-new", ProviderJobState::Unknown),
    ];
    let job = crate::ProviderRecord(json!({"v": 1, "name": "scenario", "job_handle": "j1"}));
    for (raw, want) in states {
        let r = rig_with(
            vec![ex(
                Method::Get,
                "/jobs/j1",
                200,
                json!({"job": {
                    "status": raw, "progress": 1.7, "billing": {"cuCost": 4},
                    "metadata": {"hint": "out of memory"},
                }}),
            )],
            &[],
        );
        let st = block_on(r.p.status(&job)).unwrap();
        assert_eq!(st.state, want, "{raw}");
        assert_eq!(st.progress, 1.0, "clamped");
        assert_eq!(st.cost.unwrap().amount, CostAmount::Known(4.0));
        assert_eq!(st.provider.0["raw_status"], raw);
        assert_eq!(st.error.is_some(), want == ProviderJobState::Failed);
    }
    let offline = || ex(Method::Get, "/jobs/j1", 0, json!("offline"));
    let r = rig_with(vec![offline(), offline(), offline()], &[]);
    let st = block_on(r.p.status(&job)).unwrap();
    assert_eq!(
        (st.state, st.error.clone()),
        (ProviderJobState::Unknown, Some(ProviderError::Offline))
    );
    assert_eq!(
        crate::poll_delay(std::time::Duration::ZERO, st.error.as_ref()).as_secs(),
        10
    );
}

#[test]
fn fr_prv_20_cancel_and_not_cancellable() {
    let r = scenario("cancel");
    let job = crate::ProviderRecord(json!({"v": 1, "job_handle": "<job-1>"}));
    assert_eq!(block_on(r.p.cancel(&job)).unwrap(), CancelAck::Canceled);
    assert_eq!(
        block_on(r.p.cancel(&job)).unwrap(),
        CancelAck::NotCancellable
    );
}

#[test]
fn fr_prv_21_an_expired_signed_url_is_re_resolved_once() {
    let r = scenario("fetch-expired-url");
    let job = crate::ProviderRecord(json!({"v": 1, "job_handle": "<job-1>"}));
    let c = block_on(r.p.fetch_candidates(&job)).unwrap();
    assert_eq!(c.images.len(), 1);
    assert_eq!(
        c.images[0].bytes,
        crate::fixtures::by_name("candidate-1-256.png").unwrap(),
        "bytes untouched"
    );
    assert_eq!(c.images[0].seed, Some(7));
    assert_eq!(r.t.remaining(), 0);
}

#[test]
fn fr_prv_26_the_record_has_the_appendix_b_shape() {
    let r = scenario("contract");
    r.p.set_account(ProviderAccount {
        team_id: Some("<team-id>".into()),
        project_id: None,
    });
    block_on(r.p.list_models(&ModelFilter::default())).unwrap();
    block_on(r.p.get_capabilities(V3, Operation::EditMasked)).unwrap();
    block_on(r.p.estimate(&request(V3))).unwrap();
    let rec = block_on(r.p.submit(&request(V3))).unwrap();
    let keys: Vec<_> = rec.0.as_object().unwrap().keys().cloned().collect();
    for k in [
        "v",
        "name",
        "model",
        "job_handle",
        "asset_refs",
        "request",
        "raw_status",
        "cost_raw",
    ] {
        assert!(keys.contains(&k.to_owned()), "{k} in {keys:?}");
    }
    let refs = &rec.0["asset_refs"];
    assert!(
        refs["image"].is_string()
            && refs["mask"].is_string()
            && refs["hash_image"].is_string()
            && refs["hash_mask"].is_string()
    );
    assert_eq!(rec.0["request"]["prompt"], "a mossy flagstone floor");
    // The capability snapshot is the serialised generic capabilities.
    let caps = block_on(r.p.job_capabilities(V3, Operation::EditMasked)).unwrap();
    let back: Capabilities = serde_json::from_value(serde_json::to_value(&caps).unwrap()).unwrap();
    assert_eq!(back, caps);
}

#[test]
fn fr_prv_01_the_suite_runs_through_the_any_provider_enum() {
    contract_suite(&crate::AnyProvider::fake(), "fake-white-edit");
    let r = scenario("contract");
    r.p.set_account(ProviderAccount {
        team_id: Some("<team-id>".into()),
        project_id: None,
    });
    contract_suite(&crate::AnyProvider::Scenario(r.p), V3);
}

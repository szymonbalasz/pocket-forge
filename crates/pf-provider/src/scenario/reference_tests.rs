//! Reference inputs over replay (FR-PRV-33/34, #51): references go into their slots' inputs,
//! are uploaded once per hash (also across estimate and submit), are re-uploaded with the rest,
//! and a bad reference is refused before any network call.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::overlay::{SCENARIO_ROWS, row_in};
use super::record::content_hash;
use super::tests::{BASE, creds, ex, get_model, rig_over, rig_with};
use super::*;
use crate::credentials::MemoryStore;
use crate::fixtures::{disc_mask_png, room_png};
use crate::replay::ReplayTransport;
use crate::transport::{Method, RecordingSleeper};
use crate::{
    CostAmount, JobRequest, Operation, Provider, ProviderError, ReferenceImage, ReferenceRole,
    block_on,
};

const CHAR: &str = "model_ideogram-character";
const V3: &str = "model_ideogram-v3-balanced";
const STYLE: &str = "styleReferenceImages";
const IDENTITY: &str = "characterReferenceImages";

fn style_a() -> Vec<u8> {
    room_png(64)
}

fn style_b() -> Vec<u8> {
    b"\xFF\xD8\xFF\xE0 a style reference, JPEG".to_vec()
}

fn identity() -> Vec<u8> {
    b"RIFF\x10\0\0\0WEBPVP8 an identity reference".to_vec()
}

fn reference(slot: &str, role: Option<ReferenceRole>, bytes: Vec<u8>) -> ReferenceImage {
    ReferenceImage {
        slot: slot.into(),
        role,
        hash: format!("sha256:{}", content_hash(&bytes)),
        bytes,
    }
}

fn request(model: &str, references: Vec<ReferenceImage>) -> JobRequest {
    JobRequest {
        operation: Operation::EditMasked,
        model_id: model.into(),
        final_prompt: "a mossy flagstone floor".into(),
        controls: BTreeMap::new(),
        references,
        region_png: room_png(256),
        mask_png: disc_mask_png(256),
        candidate_count: 2,
        seed: Some(7),
    }
}

/// Style A + B (PNG, JPEG) and one identity (WebP), in that order.
fn character_refs() -> Vec<ReferenceImage> {
    use ReferenceRole::{Identity, Style};
    vec![
        reference(STYLE, Some(Style), style_a()),
        reference(STYLE, None, style_b()),
        reference(IDENTITY, Some(Identity), identity()),
    ]
}

fn asset(id: &str) -> Value {
    json!({"asset": {"id": id}})
}

fn job(id: &str) -> Value {
    json!({"job": {"jobId": id, "status": "queued"}})
}

/// The data-URL media types of the `POST /assets` uploads seen, in order.
fn uploaded_types(t: &ReplayTransport) -> Vec<String> {
    t.seen()
        .iter()
        .filter(|s| s.path == "/assets")
        .map(|s| {
            let data = s.body.as_ref().unwrap()["image"].as_str().unwrap();
            data.trim_start_matches("data:")
                .split(';')
                .next()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn fr_prv_33_34_references_fill_their_slots_and_upload_once_per_hash() {
    let t = Arc::new(ReplayTransport::new(
        BASE,
        &fixture_recording(CHAR, "references").unwrap(),
    ));
    let r = rig_over(t.clone(), &[CHAR]);
    let req = request(CHAR, character_refs());

    // Estimate (dry_run_assets: true) uploads all five files, references too (FR-PRV-34).
    let est = block_on(r.p.estimate(&req)).unwrap();
    assert_eq!(est.cost.amount, CostAmount::Known(12.0));
    assert_eq!(
        uploaded_types(&t),
        [
            "image/png",
            "image/png",
            "image/png",
            "image/jpeg",
            "image/webp"
        ],
        "each reference keeps its format (FR-PRV-33)"
    );
    // Submit uploads nothing again (FR-PRV-14).
    let rec = block_on(r.p.submit(&req)).unwrap();
    assert_eq!(t.remaining(), 0, "every recorded exchange was used");
    assert_eq!(uploaded_types(&t).len(), 5);

    // Request order per slot; an array for `file_array` (FR-PRV-33).
    for body in [&est.provider.0["request"], &rec.0["request"]] {
        assert_eq!(body[STYLE], json!(["<asset-3>", "<asset-4>"]));
        assert_eq!(body[IDENTITY], json!(["<asset-5>"]));
    }
    // Appendix B: `asset_refs.references` = [{slot, hash, ref}], the caller's hash.
    let hashes: Vec<String> = req.references.iter().map(|x| x.hash.clone()).collect();
    assert_eq!(
        rec.0["asset_refs"]["references"],
        json!([
            {"slot": STYLE, "hash": hashes[0], "ref": "<asset-3>"},
            {"slot": STYLE, "hash": hashes[1], "ref": "<asset-4>"},
            {"slot": IDENTITY, "hash": hashes[2], "ref": "<asset-5>"},
        ])
    );
    assert_eq!(
        est.provider.0["asset_refs"]["references"],
        rec.0["asset_refs"]["references"]
    );
    assert_eq!(rec.0["job_handle"], "<job-1>");
}

#[test]
fn fr_prv_33_bad_references_are_refused_with_zero_network_calls() {
    use ReferenceRole::Style;
    let one_identity = || reference(IDENTITY, None, identity());
    let mut wrong_hash = reference(STYLE, Some(Style), style_a());
    wrong_hash.hash = format!("sha256:{}", content_hash(b"something else"));
    let cases: Vec<(&str, Vec<ReferenceImage>)> = vec![
        (
            "unknown slot",
            vec![one_identity(), reference("moodBoard", None, style_a())],
        ),
        ("required slot under min", vec![]),
        (
            "slot over max",
            vec![one_identity(), reference(IDENTITY, None, style_b())],
        ),
        (
            "unsniffable format",
            vec![
                one_identity(),
                reference(STYLE, None, b"GIF89a....".to_vec()),
            ],
        ),
        ("hash mismatch", vec![one_identity(), wrong_hash]),
    ];
    // Only the capabilities call is recorded; any other request would panic the replay.
    let r = rig_with(vec![get_model(CHAR)], &[CHAR]);
    block_on(r.p.get_capabilities(CHAR)).unwrap();
    let reference_error = ProviderError::InvalidRequest {
        field: "reference".into(),
    };
    for (why, refs) in cases {
        let req = request(CHAR, refs);
        assert_eq!(
            block_on(r.p.estimate(&req)).unwrap_err(),
            reference_error,
            "estimate: {why}"
        );
        assert_eq!(
            block_on(r.p.submit(&req)).unwrap_err(),
            reference_error,
            "submit: {why}"
        );
    }
    assert_eq!(r.t.seen().len(), 1, "no call after the capabilities");
}

#[test]
fn fr_prv_33_a_file_slot_gets_one_ref_and_empty_slots_are_omitted() {
    let mut model = get_model(V3);
    model.response.body["model"]["inputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "poseRef", "type": "file", "kind": "image"}));
    let r = rig_with(
        vec![
            model,
            ex(Method::Post, "/assets", 200, asset("a-image")),
            ex(Method::Post, "/assets", 200, asset("a-mask")),
            ex(Method::Post, "/assets", 200, asset("a-pose")),
            ex(
                Method::Post,
                &format!("/generate/custom/{V3}"),
                200,
                job("j1"),
            ),
        ],
        &[V3],
    );
    let req = request(V3, vec![reference("poseRef", None, style_b())]);
    let rec = block_on(r.p.submit(&req)).unwrap();
    let body = rec.0["request"].as_object().unwrap();
    assert_eq!(
        body["poseRef"],
        json!("a-pose"),
        "a `file` slot gets one ref"
    );
    assert!(
        !body.contains_key(STYLE),
        "a slot with no references is omitted"
    );
    assert_eq!(r.t.remaining(), 0);
}

#[test]
fn fr_prv_34_a_missing_reference_is_reuploaded_with_the_rest_once() {
    // Style A is also the identity: identical bytes go up once, even on the forced re-upload.
    let refs = vec![
        reference(STYLE, None, style_a()),
        reference(STYLE, None, style_b()),
        reference(IDENTITY, None, style_a()),
    ];
    let generate = format!("/generate/custom/{CHAR}");
    let r = rig_with(
        vec![
            get_model(CHAR),
            ex(Method::Post, "/assets", 200, asset("image-1")),
            ex(Method::Post, "/assets", 200, asset("mask-1")),
            ex(Method::Post, "/assets", 200, asset("a-1")),
            ex(Method::Post, "/assets", 200, asset("b-1")),
            ex(
                Method::Post,
                &generate,
                404,
                json!({"reason": "Asset b-1 not found", "name": "NotFoundError"}),
            ),
            ex(Method::Post, "/assets", 200, asset("image-2")),
            ex(Method::Post, "/assets", 200, asset("mask-2")),
            ex(Method::Post, "/assets", 200, asset("a-2")),
            ex(Method::Post, "/assets", 200, asset("b-2")),
            ex(Method::Post, &generate, 200, job("j1")),
        ],
        &[CHAR],
    );
    let rec = block_on(r.p.submit(&request(CHAR, refs))).unwrap();
    assert_eq!(r.t.remaining(), 0);
    assert_eq!(rec.0["request"][STYLE], json!(["a-2", "b-2"]));
    assert_eq!(rec.0["request"][IDENTITY], json!(["a-2"]));
    let sent: Vec<_> = rec.0["asset_refs"]["references"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["ref"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(sent, ["a-2", "b-2", "a-2"], "the new refs are recorded");
}

#[test]
fn fr_prv_34_a_reference_missing_twice_is_invalid_request_reference() {
    let generate = format!("/generate/custom/{CHAR}");
    let missing = || {
        ex(
            Method::Post,
            &generate,
            404,
            json!({"reason": "Asset in characterReferenceImages not found"}),
        )
    };
    let mut exchanges = vec![get_model(CHAR)];
    for round in 0..2 {
        for name in ["image", "mask", "identity"] {
            exchanges.push(ex(
                Method::Post,
                "/assets",
                200,
                asset(&format!("{name}-{round}")),
            ));
        }
        exchanges.push(missing());
    }
    let r = rig_with(exchanges, &[CHAR]);
    let req = request(CHAR, vec![reference(IDENTITY, None, identity())]);
    assert_eq!(
        block_on(r.p.submit(&req)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "reference".into()
        }
    );
    assert_eq!(r.t.remaining(), 0);
}

#[test]
fn fr_prv_34_without_dry_run_assets_estimate_uploads_no_references() {
    let row = OverlayRow {
        dry_run_assets: false,
        ..*row_in(&SCENARIO_ROWS, CHAR).unwrap()
    };
    let t = Arc::new(ReplayTransport::from_exchanges(
        BASE,
        vec![
            get_model(CHAR),
            ex(
                Method::Post,
                &format!("/generate/custom/{CHAR}?dryRun=true"),
                269,
                json!({"creativeUnitsCost": 9}),
            ),
        ],
    ));
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
    let est = block_on(p.estimate(&request(CHAR, character_refs()))).unwrap();
    assert_eq!(est.cost.amount, CostAmount::Known(9.0));
    assert_eq!(t.remaining(), 0, "no uploads before the dry run");
    let body = est.provider.0["request"].as_object().unwrap();
    assert!(!body.contains_key(STYLE) && !body.contains_key(IDENTITY));
    assert!(est.provider.0["asset_refs"].get("references").is_none());
}

//! The fake `generate` models (GEN §10): listing, capabilities, the output backstop,
//! deterministic candidates at the asked size, transparency, cost and the Appendix D cases.

use super::*;
use crate::block_on as run;
use crate::fixtures::noise_png;
use crate::pixels::png_size;
use crate::{OutputSize, ReferenceImage, StructureSlot};

const WH: &str = "fake-gen-wh";
const ASPECT: &str = "fake-gen-aspect";

fn generate(model: &str, output: OutputRequest, count: u32) -> JobRequest {
    JobRequest {
        operation: Operation::Generate,
        model_id: model.into(),
        final_prompt: "flooded crypt, torchlight".into(),
        controls: no_controls(),
        references: vec![],
        region_png: vec![],
        mask_png: vec![],
        candidate_count: count,
        seed: Some(11),
        output: Some(output),
    }
}

fn wh(w: u32, h: u32) -> OutputRequest {
    OutputRequest::Wh { w, h }
}

fn reference(slot: &str, seed: u64) -> ReferenceImage {
    let bytes = noise_png(32, seed);
    ReferenceImage {
        slot: slot.into(),
        role: None,
        hash: format!("sha256:{}", sha256_hex(&bytes)),
        bytes,
    }
}

/// `(width, height, channels, pixels)` of a PNG.
fn decode(bytes: &[u8]) -> (u32, u32, usize, Vec<u8>) {
    let mut r = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, info.color_type.samples(), buf)
}

fn run_job(p: &FakeProvider, req: &JobRequest) -> Candidates {
    let mut rec = run(p.submit(req)).unwrap();
    for _ in 0..3 {
        rec = run(p.status(&rec)).unwrap().provider;
    }
    run(p.fetch_candidates(&rec)).unwrap()
}

#[test]
fn fr_gen_05_generate_lists_the_two_fake_generate_models_and_edit_is_unchanged() {
    let p = FakeProvider::default();
    let list = |op| {
        run(p.list_models(&ModelFilter {
            operation: Some(op),
        }))
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect::<Vec<_>>()
    };
    assert_eq!(list(Operation::Generate), [WH, ASPECT]);
    assert_eq!(list(Operation::EditMasked), FAKE_MODEL_IDS);
    let all = run(p.list_models(&ModelFilter::default())).unwrap();
    assert_eq!(all.len(), 5);
    assert_eq!(all[3].operations, [Operation::Generate]);
}

#[test]
fn fr_gen_06_fake_generate_capabilities() {
    let p = FakeProvider::default();
    let c = run(p.get_capabilities(WH, Operation::Generate)).unwrap();
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    assert_eq!(c.operations, [Operation::Generate]);
    assert!(c.mask.is_none());
    assert_eq!(
        c.output_size,
        Some(OutputSize::Wh {
            min: 64,
            max: 2048,
            step: 16
        })
    );
    assert_eq!(
        c.structure,
        Some(StructureSlot {
            slot: "refs".into()
        })
    );
    let names: Vec<_> = c
        .controls
        .iter()
        .map(|x| (x.name.as_str(), x.role))
        .collect();
    assert_eq!(
        names,
        [
            ("prompt", ControlRole::Prompt),
            ("num_samples", ControlRole::CandidateCount),
            ("seed", ControlRole::Seed),
            ("background", ControlRole::Background),
            ("refs", ControlRole::Reference),
        ],
        "width and height are output_size, never controls (FR-GEN-11)"
    );
    let a = run(p.get_capabilities(ASPECT, Operation::Generate)).unwrap();
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);
    assert_eq!(
        a.output_size,
        Some(OutputSize::Aspect {
            allowed: vec!["1:1".into(), "3:4".into(), "4:3".into(), "2:3".into()]
        }),
        "`auto` is not a ratio"
    );
    assert!(a.structure.is_none());
    assert!(
        a.controls
            .iter()
            .all(|x| x.role != ControlRole::CandidateCount),
        "no count: one result per job"
    );
    // The generate models are not edit models, and the reverse.
    assert!(run(p.get_capabilities(WH, Operation::EditMasked)).is_err());
    assert!(run(p.get_capabilities("fake-white-edit", Operation::Generate)).is_err());
}

#[test]
fn fr_gen_08_the_output_backstop_refuses_sizes_the_model_doesnt_take() {
    let p = FakeProvider::default();
    let output_error = ProviderError::InvalidRequest {
        field: "output".into(),
    };
    let mut no_output = generate(WH, wh(512, 512), 1);
    no_output.output = None;
    for req in [
        generate(WH, wh(520, 512), 1),  // not a multiple of 16
        generate(WH, wh(2064, 512), 1), // over max
        generate(WH, wh(48, 512), 1),   // under min
        generate(
            WH,
            OutputRequest::Aspect {
                aspect: "1:1".into(),
            },
            1,
        ),
        generate(
            ASPECT,
            OutputRequest::Aspect {
                aspect: "auto".into(),
            },
            1,
        ),
        generate(ASPECT, wh(512, 512), 1),
        no_output,
    ] {
        assert_eq!(run(p.estimate(&req)).unwrap_err(), output_error, "{req:?}");
        assert_eq!(run(p.submit(&req)).unwrap_err(), output_error);
    }
    // No count input → exactly one result per job (R72).
    let two = generate(
        ASPECT,
        OutputRequest::Aspect {
            aspect: "1:1".into(),
        },
        2,
    );
    assert_eq!(
        run(p.estimate(&two)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "candidate_count".into()
        }
    );
}

#[test]
fn fr_gen_24_candidates_come_at_the_asked_size_and_are_deterministic() {
    let p = FakeProvider::default();
    let req = generate(WH, wh(320, 480), 2);
    let a = run_job(&p, &req);
    assert_eq!(a.images.len(), 2);
    for img in &a.images {
        let (w, h, ch, _) = decode(&img.bytes);
        assert_eq!((w, h, ch), (320, 480, 3), "opaque: RGB");
    }
    assert_ne!(a.images[0].bytes, a.images[1].bytes);
    let again = run_job(&FakeProvider::default(), &req);
    assert_eq!(
        a.images[0].bytes, again.images[0].bytes,
        "same request + seed"
    );

    let mut other_prompt = req.clone();
    other_prompt.final_prompt = "sunlit crypt".into();
    assert_ne!(
        run_job(&p, &other_prompt).images[0].bytes,
        a.images[0].bytes
    );
    let mut with_sketch = req.clone();
    with_sketch.references = vec![reference("refs", 1)];
    let s1 = run_job(&p, &with_sketch);
    assert_ne!(
        s1.images[0].bytes, a.images[0].bytes,
        "references change it"
    );
    assert_eq!(s1.images[0].seed, a.images[0].seed, "but not the seed");

    // Aspect: the long side is 1024; 2:3 → 683 × 1024.
    let tall = generate(
        ASPECT,
        OutputRequest::Aspect {
            aspect: "2:3".into(),
        },
        1,
    );
    let c = run_job(&p, &tall);
    let (w, h, _, _) = decode(&c.images[0].bytes);
    assert_eq!((w, h), (683, 1024));
}

#[test]
fn fr_gen_12_transparent_background_gives_an_alpha_disc() {
    let p = FakeProvider::default();
    let mut req = generate(WH, wh(128, 128), 1);
    req.controls
        .insert("background".into(), json!("transparent"));
    let c = run_job(&p, &req);
    let (w, h, ch, px) = decode(&c.images[0].bytes);
    assert_eq!((w, h, ch), (128, 128, 4));
    let alpha = |x: u32, y: u32| px[((y * w + x) * 4 + 3) as usize];
    assert_eq!(alpha(0, 0), 0, "corner transparent");
    assert_eq!(alpha(64, 64), 255, "centre opaque");
    req.controls.insert("background".into(), json!("opaque"));
    assert_eq!(decode(&run_job(&p, &req).images[0].bytes).2, 3);
}

#[test]
fn fr_gen_20_the_estimate_adds_one_unit_per_image_in_the_cost_affecting_slot() {
    let p = FakeProvider::default();
    let mut req = generate(WH, wh(256, 256), 2);
    let cost = |r: &JobRequest| run(p.estimate(r)).unwrap().cost.amount;
    assert_eq!(cost(&req), CostAmount::Known(2.0));
    req.references = vec![reference("refs", 1), reference("refs", 2)];
    assert_eq!(cost(&req), CostAmount::Known(4.0), "sketch + reference");
    assert_eq!(
        p.last_reference_hashes()
            .iter()
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>(),
        ["refs", "refs"]
    );
    req.references = (0..5).map(|i| reference("refs", i)).collect();
    assert_eq!(
        run(p.estimate(&req)).unwrap_err(),
        ProviderError::InvalidRequest {
            field: "reference".into()
        },
        "over the slot's max"
    );
}

#[test]
fn fr_gen_23_appendix_d_cases_apply_to_generate() {
    let req = generate(WH, wh(160, 96), 2);
    let bad = run_job(&FakeProvider::with_case(FakeCase::BadCandidate), &req);
    assert!(png_size(&bad.images[0].bytes).is_none());
    assert_eq!(png_size(&bad.images[1].bytes), Some((160, 96)));
    let wide = run_job(&FakeProvider::with_case(FakeCase::AspectMismatch(2)), &req);
    assert_eq!(png_size(&wide.images[0].bytes), Some((164, 96)));
    assert_eq!(
        run(FakeProvider::with_case(FakeCase::SubmitTimeout).submit(&req)).unwrap_err(),
        ProviderError::Timeout
    );
    let p = FakeProvider::with_case(FakeCase::Failure);
    let mut rec = run(p.submit(&req)).unwrap();
    let mut last = ProviderJobState::Queued;
    for _ in 0..3 {
        let st = run(p.status(&rec)).unwrap();
        (last, rec) = (st.state, st.provider);
    }
    assert_eq!(last, ProviderJobState::Failed);
    assert_eq!(
        run(FakeProvider::with_case(FakeCase::InsufficientBalance).estimate(&req)).unwrap_err(),
        ProviderError::InsufficientBalance
    );
}

#[test]
fn fr_gen_23_a_generate_job_survives_an_app_restart() {
    let dir = tempfile::tempdir().unwrap();
    let jobs = dir.path().join("fake-jobs");
    let mut req = generate(WH, wh(96, 64), 1);
    req.controls
        .insert("background".into(), json!("transparent"));
    req.references = vec![reference("refs", 3)];
    let expected = run_job(&FakeProvider::default(), &req);

    let before = FakeProvider::with_jobs_dir(jobs.clone());
    let rec = run(before.submit(&req)).unwrap();
    assert!(
        rec.0["asset_refs"].get("image").is_none(),
        "no source upload"
    );
    assert_eq!(
        rec.0["request"]["output"],
        json!({"mode": "wh", "w": 96, "h": 64})
    );
    drop(before);
    let after = FakeProvider::with_jobs_dir(jobs);
    let mut rec = rec;
    for _ in 0..3 {
        rec = run(after.status(&rec)).unwrap().provider;
    }
    let got = run(after.fetch_candidates(&rec)).unwrap();
    assert_eq!(got.images[0].bytes, expected.images[0].bytes);
}

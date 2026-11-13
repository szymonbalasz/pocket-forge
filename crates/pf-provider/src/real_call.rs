//! Real-call checks the owner runs by hand (FR-PRV-28, PRV slice 5, spec §13 "Manual
//! acceptance"). The flows are generic over [`Provider`], so CI runs them against the fake;
//! only the recorder (`pf-record`) and the gated tests in `tests/real_call.rs` point them at
//! Scenario, and both need cargo feature `real-call`.
//!
//! - [`ci_guard`]: CI fails if the feature or `PF_REAL_CALL` is set (FR-PRV-28).
//! - [`check_model`]: free calls for one model (`GET /models/{id}`, a dry run).
//! - [`masked_edit`]: one paid masked edit under a spend cap: estimate, refuse over the cap or
//!   on an unknown cost, submit, poll with [`poll_delay`], fetch, and check where the
//!   candidates changed ([`polarity`]).

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use crate::fixtures::{disc_mask_png, noise_png, room_png};
use crate::pixels::{decode_grey8, decode_rgb8, sniff};
use crate::scenario::record::content_hash;
use crate::transport::Sleeper;
use crate::{
    Capabilities, ControlRole, Cost, CostAmount, JobRequest, Operation, OutputRequest, OutputSize,
    Provider, ProviderError, ProviderJobState, ProviderRecord, ReferenceImage, poll_delay,
};

/// The environment variable that turns the gated real-call tests on (`PF_REAL_CALL=1`).
pub const ENV_GATE: &str = "PF_REAL_CALL";

/// FR-PRV-28: under CI (`CI` set and not empty), the `real-call` feature and `PF_REAL_CALL`
/// are both refused. Outside CI anything goes; the callers add their own gates.
pub fn ci_guard(ci: Option<&str>, real_call: Option<&str>, feature: bool) -> Result<(), String> {
    let in_ci = ci.is_some_and(|v| !v.trim().is_empty());
    if !in_ci {
        return Ok(());
    }
    if feature {
        return Err("cargo feature `real-call` is set in CI (FR-PRV-28, AGENTS §3.7)".into());
    }
    if real_call.is_some_and(|v| !v.trim().is_empty()) {
        return Err(format!("{ENV_GATE} is set in CI (FR-PRV-28, AGENTS §3.7)"));
    }
    Ok(())
}

/// [`ci_guard`] over this process's environment and build.
pub fn ci_guard_env() -> Result<(), String> {
    let var = |k: &str| std::env::var(k).ok();
    ci_guard(
        var("CI").as_deref(),
        var(ENV_GATE).as_deref(),
        cfg!(feature = "real-call"),
    )
}

/// True if the gated real-call tests should run: not CI, and `PF_REAL_CALL=1`.
pub fn real_calls_enabled() -> Result<bool, String> {
    ci_guard_env()?;
    Ok(std::env::var(ENV_GATE).is_ok_and(|v| v.trim() == "1"))
}

// ------------------------------------------------------------------ the test request

/// The prompt every real-call check sends.
pub const PROMPT: &str = "a mossy flagstone floor";

/// The side of the generated test image: 512, clamped to the model's limits.
fn side_for(caps: &Capabilities) -> u32 {
    let px = caps.input_px;
    let step = px.multiple_of.max(1);
    let side = 512u32.clamp(px.min, px.max.max(px.min));
    (side / step).max(1) * step
}

/// A masked-edit request over the generated room image and disc mask (public-safe fixtures,
/// KQ-22). Required controls without a default get their first allowed value; a required
/// reference slot gets generated noise images. Errors name what can't be filled.
pub fn test_request(caps: &Capabilities) -> Result<JobRequest, String> {
    if !caps.operations.contains(&Operation::EditMasked) {
        return Err(format!(
            "{} does not offer masked edit: {}",
            caps.model_id,
            caps.warnings.join("; ")
        ));
    }
    let side = side_for(caps);
    let (controls, references) = fill(caps)?;
    Ok(JobRequest {
        operation: Operation::EditMasked,
        model_id: caps.model_id.clone(),
        final_prompt: PROMPT.into(),
        controls,
        references,
        region_png: room_png(side),
        mask_png: disc_mask_png(side),
        candidate_count: 1,
        seed: caps.seed.then_some(1),
        output: None,
    })
}

/// The output a `generate` check asks for: 1024 × 1024 within the model's limits (snapped to
/// its step; raised to the smallest square of at least `min_pixels`), or ratio `1:1` if
/// allowed, else the first allowed ratio.
pub fn test_output(caps: &Capabilities) -> Option<OutputRequest> {
    match caps.output_size.as_ref()? {
        OutputSize::Wh {
            min,
            max,
            step,
            min_pixels,
        } => {
            let step = (*step).max(1);
            let top = ((*max).max(*min) / step).max(1) * step;
            let mut side = (1024u32.clamp(*min, (*max).max(*min)) / step).max(1) * step;
            while min_pixels.is_some_and(|m| u64::from(side) * u64::from(side) < m) && side < top {
                side = (side + step).min(top);
            }
            Some(OutputRequest::Wh { w: side, h: side })
        }
        OutputSize::Aspect { allowed } => {
            let pick = allowed
                .iter()
                .find(|a| *a == "1:1")
                .or_else(|| allowed.first())?;
            Some(OutputRequest::Aspect {
                aspect: pick.clone(),
            })
        }
    }
}

/// A `generate` request (GEN §10): the prompt, the [`test_output`] size, one result, no
/// sketch; required controls and slots filled as for [`test_request`].
pub fn test_generate_request(caps: &Capabilities) -> Result<JobRequest, String> {
    if !caps.operations.contains(&Operation::Generate) {
        return Err(format!(
            "{} does not offer generate: {}",
            caps.model_id,
            caps.warnings.join("; ")
        ));
    }
    let output = test_output(caps).ok_or_else(|| format!("{}: no output size", caps.model_id))?;
    let (controls, references) = fill(caps)?;
    Ok(JobRequest {
        operation: Operation::Generate,
        model_id: caps.model_id.clone(),
        final_prompt: PROMPT.into(),
        controls,
        references,
        region_png: vec![],
        mask_png: vec![],
        candidate_count: 1,
        seed: caps.seed.then_some(1),
        output: Some(output),
    })
}

/// Required controls without a default get their first allowed value; a required reference
/// slot gets generated noise images.
fn fill(caps: &Capabilities) -> Result<(BTreeMap<String, Value>, Vec<ReferenceImage>), String> {
    let mut controls = BTreeMap::new();
    for c in &caps.controls {
        if c.role != ControlRole::Other || !c.required || c.default.is_some() {
            continue;
        }
        match c.allowed.first() {
            Some(v) => {
                controls.insert(c.name.clone(), v.clone());
            }
            None => {
                return Err(format!(
                    "required input {:?} has no default and no allowed values",
                    c.name
                ));
            }
        }
    }
    let mut references = vec![];
    for (i, slot) in caps.slots.iter().enumerate() {
        for n in 0..slot.min {
            let bytes = noise_png(256, 1000 + (i as u64) * 10 + u64::from(n));
            references.push(ReferenceImage {
                slot: slot.name.clone(),
                role: slot.role,
                hash: format!("sha256:{}", content_hash(&bytes)),
                bytes,
            });
        }
    }
    Ok((controls, references))
}

// ------------------------------------------------------------------ free checks

/// What the free calls found for one model.
#[derive(Debug)]
pub struct ModelCheck {
    pub model: String,
    /// The operation checked (`edit_masked` or `generate`).
    pub operation: Operation,
    /// `GET /models/{id}` succeeded (a 404 → false).
    pub found: bool,
    pub capabilities: Option<Capabilities>,
    /// The dry run's cost, if one was run.
    pub estimate: Option<Result<Cost, String>>,
    /// The first error that stopped the check.
    pub error: Option<String>,
}

impl ModelCheck {
    pub fn offered(&self) -> bool {
        self.capabilities
            .as_ref()
            .is_some_and(|c| c.operations.contains(&self.operation))
    }

    /// One readable line for the console.
    pub fn summary(&self) -> String {
        let mut s = format!("{:<40}", self.model);
        if !self.found {
            s.push_str(" NOT FOUND");
            if let Some(e) = &self.error {
                s.push_str(&format!(" ({e})"));
            }
            return s;
        }
        s.push_str(if self.offered() {
            " offered"
        } else {
            " NOT OFFERED"
        });
        match &self.estimate {
            Some(Ok(c)) => s.push_str(&format!(" · dry run {}", cost_text(c))),
            Some(Err(e)) => s.push_str(&format!(" · dry run FAILED: {e}")),
            None => {}
        }
        if let Some(c) = &self.capabilities
            && !c.warnings.is_empty()
        {
            s.push_str(&format!(" · warnings: {}", c.warnings.join("; ")));
        }
        s
    }
}

/// `1.5 CU` or `unknown CU`.
pub fn cost_text(c: &Cost) -> String {
    match c.amount {
        CostAmount::Known(a) => format!("{a} {}", c.unit),
        CostAmount::Unknown => format!("unknown {}", c.unit),
    }
}

/// Free calls for one model and operation: its capabilities, then (if `dry_run` and the
/// model offers it) one dry run, which spends nothing (`?dryRun=true`, never `ipDetection`,
/// FR-PRV-16).
pub async fn check_model<P: Provider>(
    p: &P,
    model: &str,
    operation: Operation,
    dry_run: bool,
) -> ModelCheck {
    let mut out = ModelCheck {
        model: model.into(),
        operation,
        found: false,
        capabilities: None,
        estimate: None,
        error: None,
    };
    match p.get_capabilities(model, operation).await {
        Ok(c) => {
            out.found = true;
            out.capabilities = Some(c);
        }
        Err(e) => {
            out.error = Some(error_text(&e));
            return out;
        }
    }
    if dry_run && out.offered() {
        let caps = out.capabilities.as_ref().expect("set above");
        let request = match operation {
            Operation::Generate => test_generate_request(caps),
            _ => test_request(caps),
        };
        out.estimate = Some(match request {
            Ok(req) => p
                .estimate(&req)
                .await
                .map(|e| e.cost)
                .map_err(|e| error_text(&e)),
            Err(e) => Err(e),
        });
    }
    out
}

/// A normalised error as text with its code (`auth`, `invalid_request(model_id)`, …).
pub fn error_text(e: &ProviderError) -> String {
    format!("{}: {e}", e.code())
}

// ------------------------------------------------------------------ the paid edit

/// Where the candidate changed, against the sent image (white mask = edit).
#[derive(Debug, Clone, PartialEq)]
pub enum Polarity {
    /// Changed inside the mask much more than outside: the mask convention is right.
    Inside { inside: f64, outside: f64 },
    /// Changed outside much more than inside: the overlay's convention looks inverted.
    Outside { inside: f64, outside: f64 },
    /// No clear difference: look at the files.
    Unclear { inside: f64, outside: f64 },
    /// Not measurable (format, size); look at the files.
    NotChecked(String),
}

impl Polarity {
    pub fn text(&self) -> String {
        match self {
            Polarity::Inside { inside, outside } => format!(
                "edited inside the mask (mean change {inside:.1} inside vs {outside:.1} outside): mask convention OK"
            ),
            Polarity::Outside { inside, outside } => format!(
                "EDITED OUTSIDE THE MASK (mean change {inside:.1} inside vs {outside:.1} outside): the overlay's mask convention looks inverted"
            ),
            Polarity::Unclear { inside, outside } => format!(
                "unclear (mean change {inside:.1} inside vs {outside:.1} outside): open the files and look"
            ),
            Polarity::NotChecked(why) => format!("not checked ({why}): open the files and look"),
        }
    }
}

/// Mean absolute RGB change of `candidate` vs `sent`, inside (mask ≥ 128) and outside the
/// canonical mask. PNG candidates at the sent size only.
pub fn polarity(sent_png: &[u8], mask_png: &[u8], candidate: &[u8]) -> Polarity {
    let format = sniff(candidate).map_or("an unknown format", |f| f.ext());
    let (Some(sent), Some(mask)) = (decode_rgb8(sent_png), decode_grey8(mask_png)) else {
        return Polarity::NotChecked("the sent image or mask did not decode".into());
    };
    let Some(cand) = decode_rgb8(candidate) else {
        return Polarity::NotChecked(format!("the candidate is {format}, not a PNG"));
    };
    if (cand.width, cand.height) != (sent.width, sent.height) {
        return Polarity::NotChecked(format!(
            "the candidate is {}×{}, the sent image {}×{}",
            cand.width, cand.height, sent.width, sent.height
        ));
    }
    let (mut sum, mut n) = ([0.0f64; 2], [0u64; 2]);
    for (i, m) in mask.data.iter().enumerate() {
        let k = usize::from(*m < 128);
        for c in 0..3 {
            let a = f64::from(sent.data[i * 3 + c]);
            let b = f64::from(cand.data[i * 3 + c]);
            sum[k] += (a - b).abs();
        }
        n[k] += 3;
    }
    let mean = |k: usize| if n[k] == 0 { 0.0 } else { sum[k] / n[k] as f64 };
    let (inside, outside) = (mean(0), mean(1));
    if inside >= 8.0 && inside >= 2.0 * outside {
        Polarity::Inside { inside, outside }
    } else if outside >= 8.0 && outside >= 2.0 * inside {
        Polarity::Outside { inside, outside }
    } else {
        Polarity::Unclear { inside, outside }
    }
}

/// One fetched candidate.
#[derive(Debug)]
pub struct CandidateCheck {
    pub bytes: Vec<u8>,
    /// `png`, `jpg`, `webp` or `bin`.
    pub ext: &'static str,
    pub polarity: Polarity,
}

/// What one paid masked edit did.
#[derive(Debug)]
pub struct EditOutcome {
    pub request: JobRequest,
    pub estimate: Cost,
    pub record: ProviderRecord,
    pub state: ProviderJobState,
    pub actual: Option<Cost>,
    pub error: Option<String>,
    pub waited: Duration,
    pub candidates: Vec<CandidateCheck>,
}

/// Why a paid edit was not sent.
pub fn refuse_reason(estimate: &Cost, allow_spend: f64) -> Option<String> {
    match estimate.amount {
        CostAmount::Unknown => Some(format!(
            "the dry run returned no cost; not spending without a known estimate (cap {allow_spend} {})",
            estimate.unit
        )),
        CostAmount::Known(a) if a > allow_spend => Some(format!(
            "the estimate is {a} {u}, over --allow-spend {allow_spend} {u}",
            u = estimate.unit
        )),
        CostAmount::Known(_) => None,
    }
}

/// One masked edit under a spend cap: estimate (free) → refuse if the cost is unknown or over
/// `allow_spend` → submit (spends) → poll until terminal or `max_wait` → fetch → polarity per
/// candidate. `log` gets one line per step. Errors are readable text, never a panic.
pub async fn masked_edit<P: Provider>(
    p: &P,
    model: &str,
    allow_spend: f64,
    sleeper: &dyn Sleeper,
    max_wait: Duration,
    mut log: impl FnMut(&str),
) -> Result<EditOutcome, String> {
    let caps = p
        .get_capabilities(model, Operation::EditMasked)
        .await
        .map_err(|e| format!("capabilities: {}", error_text(&e)))?;
    let request = test_request(&caps)?;
    let estimate = p
        .estimate(&request)
        .await
        .map_err(|e| format!("dry run: {}", error_text(&e)))?
        .cost;
    log(&format!("estimate: {}", cost_text(&estimate)));
    if let Some(why) = refuse_reason(&estimate, allow_spend) {
        return Err(format!("not submitted: {why}"));
    }
    let mut record = p
        .submit(&request)
        .await
        .map_err(|e| format!("submit: {}", error_text(&e)))?;
    log("submitted; polling");
    let mut waited = Duration::ZERO;
    let mut last_err: Option<ProviderError> = None;
    let status = loop {
        let delay = poll_delay(waited, last_err.as_ref());
        sleeper.sleep(delay).await;
        waited += delay;
        match p.status(&record).await {
            Ok(s) => {
                record = s.provider.clone();
                last_err = s.error.clone();
                log(&format!(
                    "  {:>4}s {:?} {:.0}%",
                    waited.as_secs(),
                    s.state,
                    s.progress * 100.0
                ));
                if matches!(
                    s.state,
                    ProviderJobState::Succeeded
                        | ProviderJobState::Failed
                        | ProviderJobState::Canceled
                ) {
                    break s;
                }
            }
            Err(e @ (ProviderError::RateLimited { .. } | ProviderError::Offline)) => {
                last_err = Some(e);
            }
            Err(e) => return Err(format!("status: {}", error_text(&e))),
        }
        if waited >= max_wait {
            return Err(format!(
                "still not finished after {} s; the job may still spend. Job record: {}",
                waited.as_secs(),
                record.0
            ));
        }
    };
    let mut out = EditOutcome {
        request,
        estimate,
        record,
        state: status.state,
        actual: status.cost.clone(),
        error: status.error.as_ref().map(error_text),
        waited,
        candidates: vec![],
    };
    if status.state != ProviderJobState::Succeeded {
        return Ok(out);
    }
    let fetched = p
        .fetch_candidates(&out.record)
        .await
        .map_err(|e| format!("fetch: {}", error_text(&e)))?;
    for img in fetched.images {
        let ext = sniff(&img.bytes).map_or("bin", |f| f.ext());
        let polarity = polarity(&out.request.region_png, &out.request.mask_png, &img.bytes);
        out.candidates.push(CandidateCheck {
            bytes: img.bytes,
            ext,
            polarity,
        });
    }
    Ok(out)
}

/// `true` if `v` names a provider record field worth printing (job handle, raw status).
pub fn record_line(record: &ProviderRecord) -> String {
    let get = |k: &str| record.0.get(k).and_then(Value::as_str).unwrap_or("-");
    format!(
        "job {} · provider status {}",
        get("job_handle"),
        get("raw_status")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeCase;
    use crate::transport::RecordingSleeper;
    use crate::{AnyProvider, block_on};

    /// FR-PRV-28: this test runs in CI's `cargo test -p pf-provider` and fails there if the
    /// `real-call` feature or `PF_REAL_CALL` is set.
    #[test]
    fn fr_prv_28_ci_never_enables_real_calls() {
        ci_guard_env().unwrap();
    }

    /// The recorder's free `dry-run-generate` (GEN §10): 1024 × 1024 on `wh` models, `1:1` on
    /// aspect models; the dry run goes through (here over the fake, which needs no network).
    #[test]
    fn fr_gen_05_dry_run_generate_request_and_check_over_the_fake() {
        let p = crate::fake::FakeProvider::default();
        for (model, output) in [
            ("fake-gen-wh", OutputRequest::Wh { w: 1024, h: 1024 }),
            (
                "fake-gen-aspect",
                OutputRequest::Aspect {
                    aspect: "1:1".into(),
                },
            ),
        ] {
            let caps = block_on(p.get_capabilities(model, Operation::Generate)).unwrap();
            let req = test_generate_request(&caps).unwrap();
            assert_eq!(req.output, Some(output), "{model}");
            assert!(req.region_png.is_empty() && req.mask_png.is_empty());
            let c = block_on(check_model(&p, model, Operation::Generate, true));
            assert!(c.offered(), "{}", c.summary());
            assert!(matches!(c.estimate, Some(Ok(_))), "{}", c.summary());
        }
        let edit = block_on(check_model(
            &p,
            "fake-white-edit",
            Operation::Generate,
            true,
        ));
        assert!(!edit.offered() && edit.estimate.is_none());
        let wide = OutputSize::Wh {
            min: 16,
            max: 512,
            step: 16,
            min_pixels: None,
        };
        let caps = crate::Capabilities {
            output_size: Some(wide),
            ..block_on(p.get_capabilities("fake-gen-wh", Operation::Generate)).unwrap()
        };
        assert_eq!(
            test_output(&caps),
            Some(OutputRequest::Wh { w: 512, h: 512 }),
            "clamped to the model's max"
        );
        // Seedream's minimum area (3 981 312 px): the smallest step-16 square above it.
        let seedream = crate::Capabilities {
            output_size: Some(OutputSize::Wh {
                min: 1024,
                max: 6240,
                step: 16,
                min_pixels: Some(3_981_312),
            }),
            ..caps
        };
        assert_eq!(
            test_output(&seedream),
            Some(OutputRequest::Wh { w: 2000, h: 2000 })
        );
    }

    #[test]
    fn fr_prv_28_ci_guard_table() {
        assert!(ci_guard(None, Some("1"), true).is_ok(), "outside CI");
        assert!(ci_guard(Some(""), Some("1"), true).is_ok(), "empty CI");
        assert!(ci_guard(Some("true"), None, false).is_ok());
        assert!(ci_guard(Some("true"), None, true).is_err());
        assert!(ci_guard(Some("true"), Some("1"), false).is_err());
        assert!(
            ci_guard(Some("true"), Some("0"), false).is_err(),
            "set at all"
        );
    }

    #[test]
    fn fr_prv_28_spend_is_refused_without_a_known_estimate_under_the_cap() {
        let cu = |a| Cost {
            amount: a,
            unit: "CU".into(),
        };
        assert!(refuse_reason(&cu(CostAmount::Unknown), 100.0).is_some());
        assert!(refuse_reason(&cu(CostAmount::Known(12.0)), 10.0).is_some());
        assert_eq!(refuse_reason(&cu(CostAmount::Known(10.0)), 10.0), None);
    }

    fn fake(case: FakeCase) -> AnyProvider {
        let p = AnyProvider::fake();
        p.as_fake().unwrap().set_case(case);
        p
    }

    /// The whole paid flow over the fake: estimate, submit, poll, fetch; the fake edits
    /// inside the mask only, so polarity reads `Inside` for every convention.
    #[test]
    fn fr_prv_28_masked_edit_flow_over_the_fake() {
        for model in ["fake-white-edit", "fake-black-edit", "fake-alpha-edit"] {
            let p = fake(FakeCase::Success);
            let sleeper = RecordingSleeper::default();
            let mut lines = vec![];
            let out = block_on(masked_edit(
                &p,
                model,
                1000.0,
                &sleeper,
                Duration::from_secs(600),
                |l| lines.push(l.to_owned()),
            ))
            .unwrap();
            assert_eq!(out.state, ProviderJobState::Succeeded, "{model}");
            assert!(!out.candidates.is_empty());
            for c in &out.candidates {
                assert!(
                    matches!(c.polarity, Polarity::Inside { .. }),
                    "{model}: {}",
                    c.polarity.text()
                );
            }
            assert!(lines[0].starts_with("estimate: "), "{lines:?}");
        }
    }

    #[test]
    fn fr_prv_28_unknown_estimate_is_never_submitted() {
        let p = fake(FakeCase::EstimateUnknown);
        let err = block_on(masked_edit(
            &p,
            "fake-white-edit",
            1000.0,
            &RecordingSleeper::default(),
            Duration::from_secs(60),
            |_| {},
        ))
        .unwrap_err();
        assert!(err.starts_with("not submitted"), "{err}");
    }

    #[test]
    fn fr_prv_28_errors_are_readable_text() {
        let p = fake(FakeCase::Auth);
        let c = block_on(check_model(
            &p,
            "fake-white-edit",
            Operation::EditMasked,
            true,
        ));
        assert!(!c.found);
        assert!(c.summary().contains("NOT FOUND (auth"), "{}", c.summary());
        let p = AnyProvider::fake();
        let c = block_on(check_model(
            &p,
            "fake-black-edit",
            Operation::EditMasked,
            true,
        ));
        assert!(
            c.offered() && matches!(c.estimate, Some(Ok(_))),
            "{}",
            c.summary()
        );
    }

    #[test]
    fn fr_prv_28_polarity_reads_an_inverted_edit() {
        let sent = room_png(64);
        let mask = disc_mask_png(64);
        // Everything outside the disc replaced, inside untouched: inverted.
        let s = decode_rgb8(&sent).unwrap();
        let m = decode_grey8(&mask).unwrap();
        let mut data = s.data.clone();
        for (i, v) in m.data.iter().enumerate() {
            if *v < 128 {
                for c in 0..3 {
                    data[i * 3 + c] = 255 - data[i * 3 + c];
                }
            }
        }
        let cand = crate::fixtures::encode_png(64, 64, png::ColorType::Rgb, &data);
        assert!(matches!(
            polarity(&sent, &mask, &cand),
            Polarity::Outside { .. }
        ));
        assert!(matches!(
            polarity(&sent, &mask, &sent),
            Polarity::Unclear { .. }
        ));
        assert!(matches!(
            polarity(&sent, &mask, &room_png(32)),
            Polarity::NotChecked(_)
        ));
        assert!(matches!(
            polarity(&sent, &mask, b"\xff\xd8\xff\xe0jpeg"),
            Polarity::NotChecked(_)
        ));
    }
}

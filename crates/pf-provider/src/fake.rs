//! The fake provider (FR-PRV-03/04/05): deterministic, offline, free.
//!
//! - Three models, one per mask convention, whose capabilities are committed Scenario-shaped
//!   fixtures (`fixtures/provider/fake-*/get-model.json`) run through the real normaliser.
//! - Every Appendix D case, set by `PF_FAKE_CASE` or `provider_fake_set_case`.
//! - It has no transport at all, so it can't open a network connection (FR-PRV-05).
//!
//! Like a provider's server, it keeps jobs and uploads in memory; job progress lives in the
//! opaque record (`fake.polls`), so RGN's persisted record drives the state machine.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::sync::Mutex;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::pixels::{Grey8, Rgb8, decode_grey8, decode_rgb8, encode_png};
use crate::scenario::fixture_recording;
use crate::scenario::normalise::normalise;
use crate::scenario::overlay::{FAKE_ROWS, row_in};
use crate::{
    AssetRef, Balance, CancelAck, CandidateImage, Candidates, Capabilities, ConnectivityCell, Cost,
    CostAmount, Estimate, JobRequest, JobStatus, ModelFilter, ModelSummary, Operation, Provider,
    ProviderError, ProviderJobState, ProviderRecord, ProviderResult, UploadPurpose,
};

/// The unit the fake reports costs in.
pub const FAKE_UNIT: &str = "fake";

/// The fake models (FR-PRV-03).
pub const FAKE_MODEL_IDS: [&str; 3] = ["fake-white-edit", "fake-black-edit", "fake-alpha-edit"];

/// `retry_after` of the `rate_limited_once` case.
pub const FAKE_RETRY_AFTER_MS: u64 = 2000;

/// Fake behaviours (spec Appendix D).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FakeCase {
    /// queued → running → succeeded.
    #[default]
    Success,
    /// `running` for n polls, then succeeds.
    Slow(u32),
    /// `failed` with a hint.
    Failure,
    /// Provider-side `canceled` after one `running` poll.
    Canceled,
    /// These three: that error on every call.
    Auth,
    InsufficientBalance,
    Offline,
    /// One 429 with `retry_after`, then normal.
    RateLimitedOnce,
    /// The first submit reports a missing asset (exercises FR-PRV-15).
    AssetMissingOnce,
    /// Estimate returns `unknown`.
    EstimateUnknown,
    /// Cancel acks `not_cancellable`; the job continues.
    NotCancellable,
    /// Candidate 0 is undecodable; the rest are normal.
    BadCandidate,
    /// Candidates are pct % wider than sent.
    AspectMismatch(u32),
    /// Each candidate = the sent image + levels (clamped), with seed noise in the mask.
    Offset(i32),
}

fn param<T: std::str::FromStr>(name: &str, prefix: &str, default: T) -> Option<T> {
    let rest = name.strip_prefix(prefix)?;
    if rest.is_empty() {
        return Some(default);
    }
    rest.strip_prefix('(')?
        .strip_suffix(')')?
        .trim()
        .parse()
        .ok()
}

impl FakeCase {
    /// Parses a case name: `success`, `slow` / `slow(n)`, `aspect_mismatch(pct)`,
    /// `offset(levels)`, … (Appendix D).
    pub fn parse(name: &str) -> Option<Self> {
        let n = name.trim();
        Some(match n {
            "success" => FakeCase::Success,
            "failure" => FakeCase::Failure,
            "canceled" => FakeCase::Canceled,
            "auth" => FakeCase::Auth,
            "insufficient_balance" => FakeCase::InsufficientBalance,
            "offline" => FakeCase::Offline,
            "rate_limited_once" => FakeCase::RateLimitedOnce,
            "asset_missing_once" => FakeCase::AssetMissingOnce,
            "estimate_unknown" => FakeCase::EstimateUnknown,
            "not_cancellable" => FakeCase::NotCancellable,
            "bad_candidate" => FakeCase::BadCandidate,
            _ if n.starts_with("slow") => FakeCase::Slow(param(n, "slow", 5)?),
            _ if n.starts_with("aspect_mismatch") => {
                FakeCase::AspectMismatch(param(n, "aspect_mismatch", 10)?.min(400))
            }
            _ if n.starts_with("offset") => {
                FakeCase::Offset(param::<i32>(n, "offset", 32)?.clamp(-255, 255))
            }
            _ => return None,
        })
    }

    pub fn name(self) -> String {
        match self {
            FakeCase::Success => "success".into(),
            FakeCase::Slow(n) => format!("slow({n})"),
            FakeCase::Failure => "failure".into(),
            FakeCase::Canceled => "canceled".into(),
            FakeCase::Auth => "auth".into(),
            FakeCase::InsufficientBalance => "insufficient_balance".into(),
            FakeCase::Offline => "offline".into(),
            FakeCase::RateLimitedOnce => "rate_limited_once".into(),
            FakeCase::AssetMissingOnce => "asset_missing_once".into(),
            FakeCase::EstimateUnknown => "estimate_unknown".into(),
            FakeCase::NotCancellable => "not_cancellable".into(),
            FakeCase::BadCandidate => "bad_candidate".into(),
            FakeCase::AspectMismatch(p) => format!("aspect_mismatch({p})"),
            FakeCase::Offset(l) => format!("offset({l})"),
        }
    }

    /// The error every call returns in this case, if any.
    fn standing_error(self) -> Option<ProviderError> {
        match self {
            FakeCase::Auth => Some(ProviderError::Auth),
            FakeCase::InsufficientBalance => Some(ProviderError::InsufficientBalance),
            FakeCase::Offline => Some(ProviderError::Offline),
            _ => None,
        }
    }
}

/// What the fake remembers about a submitted job.
#[derive(Debug, Clone)]
struct FakeJob {
    region_png: Vec<u8>,
    mask_png: Vec<u8>,
    candidate_count: u32,
    seed: u64,
    case: FakeCase,
}

#[derive(Debug, Default)]
struct FakeState {
    case: FakeCase,
    next_job: u64,
    rate_limit_fired: bool,
    asset_missing_fired: bool,
    uploads: HashMap<String, Vec<u8>>,
    /// The fake's upload ref cache (content hash → ref), like Scenario's (R15).
    refs: HashMap<String, AssetRef>,
    upload_count: u64,
    jobs: HashMap<String, FakeJob>,
    canceled: HashSet<String>,
}

/// The deterministic stand-in provider.
#[derive(Debug, Default)]
pub struct FakeProvider {
    state: Mutex<FakeState>,
    /// Last known connectivity (FR-PRV-07).
    pub connectivity: ConnectivityCell,
}

fn invalid(field: &str) -> ProviderError {
    ProviderError::InvalidRequest {
        field: field.into(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn handle_of(job: &ProviderRecord) -> ProviderResult<String> {
    job.0
        .get("job_handle")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| invalid("job_handle"))
}

impl FakeProvider {
    pub fn with_case(case: FakeCase) -> Self {
        let p = Self::default();
        p.set_case(case);
        p
    }

    /// Sets the case and re-arms the one-shot cases.
    pub fn set_case(&self, case: FakeCase) {
        let mut s = self.lock();
        s.case = case;
        s.rate_limit_fired = false;
        s.asset_missing_fired = false;
    }

    pub fn case(&self) -> FakeCase {
        self.lock().case
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        // A panic while holding the lock can't leave the fake's maps inconsistent; keep going.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Notes connectivity and returns a ready future.
    fn done<T>(&self, out: ProviderResult<T>) -> std::future::Ready<ProviderResult<T>> {
        self.connectivity.note(&out);
        std::future::ready(out)
    }

    /// The case's standing error, or the one-shot 429.
    fn gate(&self) -> ProviderResult<()> {
        let mut s = self.lock();
        if let Some(e) = s.case.standing_error() {
            return Err(e);
        }
        if s.case == FakeCase::RateLimitedOnce && !s.rate_limit_fired {
            s.rate_limit_fired = true;
            return Err(ProviderError::RateLimited {
                retry_after_ms: FAKE_RETRY_AFTER_MS,
            });
        }
        Ok(())
    }

    /// Capabilities: the committed fixture through the real normaliser (FR-PRV-03).
    fn capabilities_for(model_id: &str) -> Option<(String, Capabilities)> {
        let row = row_in(&FAKE_ROWS, model_id)?;
        let rec = fixture_recording(model_id, "get-model")?;
        let n = normalise(model_id, &rec.exchanges.first()?.response.body, Some(row)).ok()?;
        Some((n.name, n.capabilities))
    }

    /// Checks a request against the model (incl. the FR-PRV-13 backstop) and decodes it.
    fn validate(request: &JobRequest) -> ProviderResult<(Capabilities, Rgb8, Grey8)> {
        let (_, caps) =
            Self::capabilities_for(&request.model_id).ok_or_else(|| invalid("model_id"))?;
        if !caps.operations.contains(&request.operation) {
            return Err(invalid("operation"));
        }
        if request.final_prompt.trim().is_empty() {
            return Err(invalid("prompt"));
        }
        if !(1..=4).contains(&request.candidate_count) {
            return Err(invalid("num_samples"));
        }
        crate::limits::check(&caps, request)?;
        let region = decode_rgb8(&request.region_png).ok_or_else(|| invalid("image"))?;
        let mask = decode_grey8(&request.mask_png).ok_or_else(|| invalid("mask"))?;
        Ok((caps, region, mask))
    }

    fn cost(candidate_count: u32) -> Cost {
        Cost {
            amount: CostAmount::Known(f64::from(candidate_count)),
            unit: FAKE_UNIT.into(),
        }
    }

    /// Uploads by content hash, at most once per session (R15).
    fn upload_once(&self, bytes: &[u8], force: bool) -> (AssetRef, String) {
        let hash = sha256_hex(bytes);
        let mut s = self.lock();
        if !force && let Some(r) = s.refs.get(&hash) {
            return (r.clone(), hash);
        }
        s.upload_count += 1;
        let id = AssetRef(format!("fake-asset-{}-{}", &hash[..12], s.upload_count));
        s.uploads.insert(id.0.clone(), bytes.to_vec());
        s.refs.insert(hash.clone(), id.clone());
        (id, hash)
    }

    fn record(
        &self,
        request: &JobRequest,
        job_handle: Option<&str>,
        case: FakeCase,
    ) -> ProviderRecord {
        let mut rec = json!({
            "v": 1,
            "name": "fake",
            "model": request.model_id,
            "asset_refs": {},
            "request": {
                "operation": request.operation,
                "prompt": request.final_prompt,
                "num_samples": request.candidate_count,
                "seed": request.seed,
                "controls": request.controls,
            },
            "fake": {"case": case.name(), "polls": 0},
        });
        if let Some(handle) = job_handle {
            rec["job_handle"] = json!(handle);
        }
        ProviderRecord(rec)
    }

    fn do_submit(&self, request: &JobRequest) -> ProviderResult<ProviderRecord> {
        self.gate()?;
        Self::validate(request)?;
        let case = self.case();
        let (mut image, mut hash_image) = self.upload_once(&request.region_png, false);
        let (mut mask, mut hash_mask) = self.upload_once(&request.mask_png, false);
        let mut reuploads = 0;
        {
            let mut s = self.lock();
            if s.case == FakeCase::AssetMissingOnce && !s.asset_missing_fired {
                // The provider says the asset is gone: re-upload once and retry (FR-PRV-15).
                s.asset_missing_fired = true;
                drop(s);
                (image, hash_image) = self.upload_once(&request.region_png, true);
                (mask, hash_mask) = self.upload_once(&request.mask_png, true);
                reuploads = 1;
            }
        }
        // The seed defaults to one derived from the request, so a repeat is identical.
        let seed = request.seed.unwrap_or_else(|| {
            let mut h = Sha256::new();
            h.update(&request.region_png);
            h.update(&request.mask_png);
            h.update(request.final_prompt.as_bytes());
            let d = h.finalize();
            u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]])
        });
        let mut s = self.lock();
        s.next_job += 1;
        let handle = format!("fake-job-{}-{}", s.next_job, &hash_image[..8]);
        s.jobs.insert(
            handle.clone(),
            FakeJob {
                region_png: request.region_png.clone(),
                mask_png: request.mask_png.clone(),
                candidate_count: request.candidate_count,
                seed,
                case,
            },
        );
        drop(s);
        let mut rec = self.record(request, Some(&handle), case);
        rec.0["request"]["seed"] = json!(seed);
        rec.0["asset_refs"] = json!({
            "image": image, "mask": mask, "hash_image": hash_image, "hash_mask": hash_mask,
        });
        rec.0["fake"]["reuploads"] = json!(reuploads);
        Ok(rec)
    }

    fn do_status(&self, job: &ProviderRecord) -> ProviderResult<JobStatus> {
        if self.case() == FakeCase::Offline {
            // Offline → state `unknown` (Appendix C status map); the poll delay is 10 s.
            self.connectivity.note_error(&ProviderError::Offline);
            return Ok(JobStatus {
                state: ProviderJobState::Unknown,
                progress: 0.0,
                cost: None,
                error: Some(ProviderError::Offline),
                provider: job.clone(),
            });
        }
        self.gate()?;
        let handle = handle_of(job)?;
        let polls = job.0["fake"]["polls"].as_u64().unwrap_or(0);
        let count = job.0["request"]["num_samples"].as_u64().unwrap_or(1) as u32;
        let (canceled, case) = {
            let s = self.lock();
            let case = s.jobs.get(&handle).map_or(s.case, |j| j.case);
            (s.canceled.contains(&handle), case)
        };
        let running_polls = match case {
            FakeCase::Slow(n) => u64::from(n.max(1)),
            _ => 1,
        };
        let (state, progress, error) = if canceled {
            (ProviderJobState::Canceled, 0.0, None)
        } else if polls == 0 {
            (ProviderJobState::Queued, 0.0, None)
        } else if polls <= running_polls {
            (
                ProviderJobState::Running,
                polls as f64 / (running_polls + 1) as f64,
                None,
            )
        } else {
            match case {
                FakeCase::Failure => (
                    ProviderJobState::Failed,
                    1.0,
                    Some(ProviderError::ProviderFailure {
                        hint: "the fake provider failed this job (case failure)".into(),
                    }),
                ),
                FakeCase::Canceled => (ProviderJobState::Canceled, 0.5, None),
                _ => (ProviderJobState::Succeeded, 1.0, None),
            }
        };
        let mut rec = job.clone();
        rec.0["fake"]["polls"] = json!(polls + 1);
        rec.0["raw_status"] = json!(state);
        Ok(JobStatus {
            state,
            progress: progress.clamp(0.0, 1.0),
            cost: (state == ProviderJobState::Succeeded).then(|| Self::cost(count)),
            error,
            provider: rec,
        })
    }

    fn do_fetch(&self, job: &ProviderRecord) -> ProviderResult<Candidates> {
        self.gate()?;
        let handle = handle_of(job)?;
        let fake_job = self.lock().jobs.get(&handle).cloned().ok_or_else(|| {
            ProviderError::ProviderFailure {
                hint: "the fake provider forgot this job (it keeps jobs in memory only)".into(),
            }
        })?;
        let region = decode_rgb8(&fake_job.region_png).ok_or_else(|| invalid("image"))?;
        let mask = decode_grey8(&fake_job.mask_png).ok_or_else(|| invalid("mask"))?;
        let images = (0..fake_job.candidate_count)
            .map(|i| {
                let seed = fake_job.seed.wrapping_add(u64::from(i));
                let bytes = match fake_job.case {
                    FakeCase::BadCandidate if i == 0 => b"this is not an image".to_vec(),
                    FakeCase::AspectMismatch(pct) => {
                        widen(&render_candidate(&region, &mask, seed, 0), pct)
                    }
                    FakeCase::Offset(levels) => {
                        encode(&render_candidate(&region, &mask, seed, levels))
                    }
                    _ => encode(&render_candidate(&region, &mask, seed, 0)),
                };
                CandidateImage {
                    bytes,
                    seed: Some(seed),
                    provider_meta: json!({"asset_id": format!("{handle}-c{i}"), "seed": seed}),
                }
            })
            .collect();
        Ok(Candidates {
            images,
            provider: job.clone(),
        })
    }
}

impl Provider for FakeProvider {
    fn list_models(
        &self,
        filter: &ModelFilter,
    ) -> impl Future<Output = ProviderResult<Vec<ModelSummary>>> + Send {
        let out = self.gate().map(|()| {
            FAKE_MODEL_IDS
                .iter()
                .filter(|_| matches!(filter.operation, None | Some(Operation::EditMasked)))
                .filter_map(|id| Self::capabilities_for(id))
                .filter(|(_, c)| c.operations.contains(&Operation::EditMasked))
                .map(|(name, c)| ModelSummary {
                    id: c.model_id,
                    name,
                    operations: c.operations,
                })
                .collect()
        });
        self.done(out)
    }

    fn get_capabilities(
        &self,
        model_id: &str,
    ) -> impl Future<Output = ProviderResult<Capabilities>> + Send {
        let out = self.gate().and_then(|()| {
            Self::capabilities_for(model_id)
                .map(|(_, c)| c)
                .ok_or_else(|| invalid("model_id"))
        });
        self.done(out)
    }

    fn upload(
        &self,
        bytes: &[u8],
        _purpose: UploadPurpose,
    ) -> impl Future<Output = ProviderResult<AssetRef>> + Send {
        let out = self.gate().map(|()| self.upload_once(bytes, false).0);
        self.done(out)
    }

    fn estimate(
        &self,
        request: &JobRequest,
    ) -> impl Future<Output = ProviderResult<Estimate>> + Send {
        let out = self.gate().and_then(|()| Self::validate(request)).map(|_| {
            let case = self.case();
            Estimate {
                cost: if case == FakeCase::EstimateUnknown {
                    Cost {
                        amount: CostAmount::Unknown,
                        unit: FAKE_UNIT.into(),
                    }
                } else {
                    Self::cost(request.candidate_count)
                },
                provider: self.record(request, None, case),
            }
        });
        self.done(out)
    }

    fn submit(
        &self,
        request: &JobRequest,
    ) -> impl Future<Output = ProviderResult<ProviderRecord>> + Send {
        let out = self.do_submit(request);
        self.done(out)
    }

    fn status(
        &self,
        job: &ProviderRecord,
    ) -> impl Future<Output = ProviderResult<JobStatus>> + Send {
        let out = self.do_status(job);
        if self.case() != FakeCase::Offline {
            self.connectivity.note(&out);
        }
        std::future::ready(out)
    }

    fn cancel(
        &self,
        job: &ProviderRecord,
    ) -> impl Future<Output = ProviderResult<CancelAck>> + Send {
        let out = self.gate().and_then(|()| handle_of(job)).map(|handle| {
            let mut s = self.lock();
            if s.case == FakeCase::NotCancellable {
                CancelAck::NotCancellable
            } else {
                s.canceled.insert(handle);
                CancelAck::Canceled
            }
        });
        self.done(out)
    }

    fn fetch_candidates(
        &self,
        job: &ProviderRecord,
    ) -> impl Future<Output = ProviderResult<Candidates>> + Send {
        let out = self.do_fetch(job);
        self.done(out)
    }

    fn balance(&self) -> impl Future<Output = ProviderResult<Balance>> + Send {
        let out = self.gate().map(|()| Balance::Known {
            remaining: 1000.0,
            unit: FAKE_UNIT.into(),
            period_end: None,
        });
        self.done(out)
    }
}

/// SplitMix64: a tiny deterministic generator for the seed-derived pattern.
fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Masked pixels get a seed-derived pattern, blended by the mask value; unmasked pixels
/// (mask 0) are copied unchanged (FR-PRV-03). With `levels ≠ 0` (case `offset`), every pixel
/// is the sent pixel + levels (clamped) and masked pixels add seed noise instead.
fn render_candidate(region: &Rgb8, mask: &Grey8, seed: u64, levels: i32) -> Rgb8 {
    let base = splitmix(seed);
    let tint = [
        (base & 0xFF) as u32,
        ((base >> 8) & 0xFF) as u32,
        ((base >> 16) & 0xFF) as u32,
    ];
    let mut out = region.data.clone();
    for (i, &m) in mask.data.iter().enumerate() {
        let noise = (splitmix(base ^ (i as u64)) & 0x3F) as u32;
        if levels != 0 {
            for c in 0..3 {
                let shifted = i32::from(out[i * 3 + c]) + levels;
                let n = if m == 0 { 0 } else { noise as i32 / 4 };
                out[i * 3 + c] = (shifted + n).clamp(0, 255) as u8;
            }
            continue;
        }
        if m == 0 {
            continue;
        }
        let (x, y) = (
            i as u64 % u64::from(region.width),
            i as u64 / u64::from(region.width),
        );
        // 16 px checker of the tint plus seeded per-pixel noise.
        let checker = ((x / 16 + y / 16) % 2) as u32 * 64;
        let m = u32::from(m);
        for c in 0..3 {
            let pattern = (tint[c] / 2 + checker + noise).min(255);
            let orig = u32::from(out[i * 3 + c]);
            out[i * 3 + c] = ((orig * (255 - m) + pattern * m + 127) / 255) as u8;
        }
    }
    Rgb8 {
        width: region.width,
        height: region.height,
        data: out,
    }
}

fn encode(img: &Rgb8) -> Vec<u8> {
    encode_png(img.width, img.height, png::ColorType::Rgb, &img.data)
}

/// Nearest-neighbour horizontal stretch by `pct` % (case `aspect_mismatch`).
fn widen(img: &Rgb8, pct: u32) -> Vec<u8> {
    let w = (u64::from(img.width) * u64::from(100 + pct)).div_ceil(100) as u32;
    let data: Vec<u8> = (0..img.height)
        .flat_map(|y| {
            (0..w).flat_map(move |x| {
                let sx = (u64::from(x) * u64::from(img.width) / u64::from(w)) as u32;
                let i = ((y * img.width + sx) * 3) as usize;
                [img.data[i], img.data[i + 1], img.data[i + 2]]
            })
        })
        .collect();
    encode_png(w, img.height, png::ColorType::Rgb, &data)
}

/// Canonical controls map helper for tests and tools.
pub fn no_controls() -> BTreeMap<String, Value> {
    BTreeMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on as run;
    use crate::pixels::png_size;

    /// A 64×64 gradient region and a mask covering the left half.
    pub(crate) fn request(seed: Option<u64>) -> JobRequest {
        let (w, h) = (64u32, 64u32);
        let region: Vec<u8> = (0..w * h)
            .flat_map(|i| [(i % w) as u8 * 4, (i / w) as u8 * 4, 90])
            .collect();
        let mask: Vec<u8> = (0..w * h)
            .map(|i| if i % w < w / 2 { 255 } else { 0 })
            .collect();
        JobRequest {
            operation: Operation::EditMasked,
            model_id: "fake-white-edit".into(),
            final_prompt: "a mossy flagstone".into(),
            controls: no_controls(),
            references: vec![],
            region_png: encode_png(w, h, png::ColorType::Rgb, &region),
            mask_png: encode_png(w, h, png::ColorType::Grayscale, &mask),
            candidate_count: 3,
            seed,
        }
    }

    fn states(
        p: &FakeProvider,
        req: &JobRequest,
        polls: usize,
    ) -> (Vec<ProviderJobState>, ProviderRecord) {
        let mut rec = run(p.submit(req)).unwrap();
        let mut out = vec![];
        for _ in 0..polls {
            let st = run(p.status(&rec)).unwrap();
            out.push(st.state);
            rec = st.provider;
        }
        (out, rec)
    }

    fn run_job(p: &FakeProvider, req: &JobRequest) -> Candidates {
        let (s, rec) = states(p, req, 3);
        assert_eq!(s.last(), Some(&ProviderJobState::Succeeded));
        run(p.fetch_candidates(&rec)).unwrap()
    }

    fn case(name: &str) -> FakeProvider {
        FakeProvider::with_case(FakeCase::parse(name).unwrap())
    }

    use ProviderJobState::*;

    #[test]
    fn fr_prv_03_three_models_one_per_mask_convention_from_fixtures() {
        let p = FakeProvider::default();
        let models = run(p.list_models(&ModelFilter::default())).unwrap();
        assert_eq!(models.len(), 3);
        let conventions: HashSet<_> = models
            .iter()
            .map(|m| {
                let caps = run(p.get_capabilities(&m.id)).unwrap();
                assert!(caps.warnings.is_empty(), "{:?}", caps.warnings);
                format!("{:?}", caps.mask.unwrap().convention)
            })
            .collect();
        assert_eq!(conventions.len(), 3);
        assert_eq!(models[0].name, "Fake (white = edit)");
    }

    #[test]
    fn fr_prv_03_same_request_and_seed_gives_byte_identical_candidates() {
        let a = run_job(&FakeProvider::default(), &request(Some(7)));
        let b = run_job(&FakeProvider::default(), &request(Some(7)));
        assert_eq!(a.images.len(), 3);
        assert_eq!(a.images, b.images);
        let c = run_job(&FakeProvider::default(), &request(Some(8)));
        assert_ne!(a.images[0].bytes, c.images[0].bytes);
    }

    #[test]
    fn fr_prv_03_unmasked_pixels_unchanged_masked_pixels_patterned() {
        let req = request(Some(1));
        let original = decode_rgb8(&req.region_png).unwrap();
        let cands = run_job(&FakeProvider::default(), &req);
        let cand = decode_rgb8(&cands.images[0].bytes).unwrap();
        let w = original.width as usize;
        let mut changed = 0;
        for i in 0..w * original.height as usize {
            let px = |img: &Rgb8| [img.data[i * 3], img.data[i * 3 + 1], img.data[i * 3 + 2]];
            if i % w >= w / 2 {
                assert_eq!(px(&cand), px(&original), "unmasked pixel {i} changed");
            } else if px(&cand) != px(&original) {
                changed += 1;
            }
        }
        assert!(changed > w * 64 / 4, "masked pixels mostly changed");
    }

    #[test]
    fn fr_prv_04_case_names_parse_and_round_trip() {
        for n in [
            "success",
            "slow",
            "slow(3)",
            "failure",
            "canceled",
            "auth",
            "insufficient_balance",
            "offline",
            "rate_limited_once",
            "asset_missing_once",
            "estimate_unknown",
            "not_cancellable",
            "bad_candidate",
            "aspect_mismatch(25)",
            "offset(-40)",
        ] {
            let c = FakeCase::parse(n).unwrap_or_else(|| panic!("{n}"));
            assert_eq!(FakeCase::parse(&c.name()), Some(c));
        }
        assert_eq!(FakeCase::parse("slow"), Some(FakeCase::Slow(5)));
        for bad in ["nope", "slow(x)", "slow(3", "offset()"] {
            assert!(FakeCase::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn fr_prv_04_success_goes_queued_running_succeeded() {
        assert_eq!(
            states(&case("success"), &request(None), 3).0,
            [Queued, Running, Succeeded]
        );
    }

    #[test]
    fn fr_prv_04_slow_runs_for_n_polls() {
        assert_eq!(
            states(&case("slow(3)"), &request(None), 5).0,
            [Queued, Running, Running, Running, Succeeded]
        );
    }

    #[test]
    fn fr_prv_04_failure_fails_with_a_hint() {
        let p = case("failure");
        let mut rec = run(p.submit(&request(None))).unwrap();
        for _ in 0..2 {
            rec = run(p.status(&rec)).unwrap().provider;
        }
        let st = run(p.status(&rec)).unwrap();
        assert_eq!(st.state, Failed);
        assert_eq!(st.error.unwrap().code(), "provider_failure");
    }

    #[test]
    fn fr_prv_04_canceled_after_one_running_poll() {
        assert_eq!(
            states(&case("canceled"), &request(None), 3).0,
            [Queued, Running, Canceled]
        );
    }

    #[test]
    fn fr_prv_04_auth_balance_and_offline_fail_every_call() {
        for (name, code) in [
            ("auth", "auth"),
            ("insufficient_balance", "insufficient_balance"),
            ("offline", "offline"),
        ] {
            let p = case(name);
            assert_eq!(
                run(p.list_models(&ModelFilter::default()))
                    .unwrap_err()
                    .code(),
                code
            );
            assert_eq!(
                run(p.get_capabilities("fake-white-edit"))
                    .unwrap_err()
                    .code(),
                code
            );
            assert_eq!(run(p.estimate(&request(None))).unwrap_err().code(), code);
            assert_eq!(run(p.submit(&request(None))).unwrap_err().code(), code);
            assert_eq!(run(p.balance()).unwrap_err().code(), code);
        }
        // Offline status is state `unknown` (Appendix C), not an error; the job resumes.
        let p = FakeProvider::default();
        let rec = run(p.submit(&request(None))).unwrap();
        p.set_case(FakeCase::Offline);
        let st = run(p.status(&rec)).unwrap();
        assert_eq!(
            (st.state, st.error),
            (Unknown, Some(ProviderError::Offline))
        );
        assert_eq!(p.connectivity.get(), crate::Connectivity::Offline);
        p.set_case(FakeCase::Success);
        assert_eq!(run(p.status(&st.provider)).unwrap().state, Queued);
        assert_eq!(p.connectivity.get(), crate::Connectivity::Ok);
        assert_eq!(
            case("auth").connectivity.get(),
            crate::Connectivity::Unknown
        );
    }

    #[test]
    fn fr_prv_04_rate_limited_once_then_normal() {
        let p = case("rate_limited_once");
        assert_eq!(
            run(p.estimate(&request(None))).unwrap_err(),
            ProviderError::RateLimited {
                retry_after_ms: FAKE_RETRY_AFTER_MS
            }
        );
        run(p.estimate(&request(None))).unwrap();
        run(p.estimate(&request(None))).unwrap();
    }

    #[test]
    fn fr_prv_04_asset_missing_once_reuploads_and_succeeds() {
        let p = case("asset_missing_once");
        let req = request(Some(1));
        run(p.upload(&req.region_png, UploadPurpose::Source)).unwrap();
        let first = run(p.submit(&req)).unwrap();
        assert_eq!(first.0["fake"]["reuploads"], 1);
        let second = run(p.submit(&req)).unwrap();
        assert_eq!(second.0["fake"]["reuploads"], 0);
        assert_eq!(
            first.0["asset_refs"]["image"], second.0["asset_refs"]["image"],
            "new refs are cached"
        );
    }

    #[test]
    fn fr_prv_04_estimate_unknown() {
        let est = run(case("estimate_unknown").estimate(&request(None))).unwrap();
        assert_eq!(est.cost.amount, CostAmount::Unknown);
        let est = run(case("success").estimate(&request(None))).unwrap();
        assert_eq!(est.cost.amount, CostAmount::Known(3.0));
    }

    #[test]
    fn fr_prv_04_not_cancellable_acks_and_the_job_continues() {
        let p = case("not_cancellable");
        let rec = run(p.submit(&request(None))).unwrap();
        assert_eq!(run(p.cancel(&rec)).unwrap(), CancelAck::NotCancellable);
        let (s, _) = states(&p, &request(None), 3);
        assert_eq!(s, [Queued, Running, Succeeded]);
        let p = case("success");
        let rec = run(p.submit(&request(None))).unwrap();
        assert_eq!(run(p.cancel(&rec)).unwrap(), CancelAck::Canceled);
        assert_eq!(run(p.status(&rec)).unwrap().state, Canceled);
    }

    #[test]
    fn fr_prv_04_bad_candidate_is_undecodable_and_the_rest_are_normal() {
        let c = run_job(&case("bad_candidate"), &request(Some(1)));
        assert!(decode_rgb8(&c.images[0].bytes).is_none());
        assert!(
            c.images[1..]
                .iter()
                .all(|i| decode_rgb8(&i.bytes).is_some())
        );
    }

    #[test]
    fn fr_prv_04_aspect_mismatch_is_pct_wider() {
        let c = run_job(&case("aspect_mismatch(25)"), &request(Some(1)));
        assert_eq!(png_size(&c.images[0].bytes), Some((80, 64)));
    }

    #[test]
    fn fr_prv_04_offset_adds_levels_with_noise_in_the_mask() {
        let req = request(Some(1));
        let original = decode_rgb8(&req.region_png).unwrap();
        let c = run_job(&case("offset(40)"), &req);
        let cand = decode_rgb8(&c.images[0].bytes).unwrap();
        let w = original.width as usize;
        let i = w - 1; // top-right: unmasked
        assert_eq!(
            i32::from(cand.data[i * 3]),
            (i32::from(original.data[i * 3]) + 40).min(255)
        );
        let masked_diff: i32 = (0..w / 2)
            .map(|x| i32::from(cand.data[x * 3]) - i32::from(original.data[x * 3]))
            .sum();
        assert!(
            masked_diff > 40 * (w as i32 / 2),
            "seed noise on top of the offset in the mask"
        );
    }

    #[test]
    fn fr_prv_05_the_fake_opens_no_network_connection() {
        // The fake has no transport field at all: a whole job, every case, runs without one.
        // A real transport would also be unusable here: the panic transport is the only one
        // in scope for this test.
        let _guard = crate::transport::PanicTransport;
        for name in [
            "success",
            "slow(2)",
            "failure",
            "canceled",
            "bad_candidate",
            "offset(10)",
        ] {
            let p = case(name);
            run(p.list_models(&ModelFilter::default())).unwrap();
            run(p.estimate(&request(Some(2)))).unwrap();
            let (_, rec) = states(&p, &request(Some(2)), 4);
            let _ = run(p.fetch_candidates(&rec));
            run(p.balance()).unwrap();
        }
    }

    #[test]
    fn fr_prv_26_record_is_opaque_json_with_the_handle() {
        let p = FakeProvider::default();
        let rec = run(p.submit(&request(Some(3)))).unwrap();
        assert_eq!(rec.0["v"], 1);
        assert_eq!(rec.0["name"], "fake");
        assert!(
            rec.0["job_handle"]
                .as_str()
                .unwrap()
                .starts_with("fake-job-")
        );
        assert!(rec.0["asset_refs"]["hash_image"].is_string());
        let est = run(p.estimate(&request(Some(3)))).unwrap();
        assert_eq!(est.cost.amount, CostAmount::Known(3.0));
        assert!(est.provider.0.get("job_handle").is_none());
    }

    #[test]
    fn invalid_requests_are_field_errors() {
        let p = FakeProvider::default();
        let mut req = request(None);
        req.model_id = "nope".into();
        assert_eq!(run(p.estimate(&req)).unwrap_err(), invalid("model_id"));
        let mut req = request(None);
        req.mask_png = encode_png(8, 8, png::ColorType::Grayscale, &[0; 64]);
        assert_eq!(run(p.submit(&req)).unwrap_err(), invalid("mask"));
    }
}

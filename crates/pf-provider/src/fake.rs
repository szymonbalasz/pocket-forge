//! The fake provider (FR-PRV-03/04/05): deterministic, offline, free.
//!
//! - Three models, one per mask convention, whose capabilities are committed Scenario-shaped
//!   fixtures (`fixtures/provider/fake-*/get-model.json`) run through the real normaliser.
//! - Every Appendix D case, set by `PF_FAKE_CASE` or `provider_fake_set_case`.
//! - It has no transport at all, so it can't open a network connection (FR-PRV-05).
//! - Reference slots (FR-PRV-35): `fake-white-edit` has `style_refs` (≤ 3, free);
//!   `fake-black-edit` has `style_refs` (≤ 2, +1 unit each in the estimate) and `identity_ref`
//!   (one, optional); `fake-alpha-edit` has none. Candidates mix the ordered reference hashes
//!   into the pattern; with none they are byte-identical to before. Tests read the last
//!   request's references with [`FakeProvider::last_reference_hashes`].
//! - Three `generate` models (GEN §10): `fake-gen-wh` (`wh` 64–2048 step 16, count 1–4, a
//!   `background` with `transparent`, one `refs` slot ≤ 4 that also takes the sketch, +1 unit
//!   per image), `fake-gen-aspect` (ratios 1:1, 3:4, 4:3, 2:3; no count; `style_refs` ≤ 2) and
//!   `fake-gen-refs` (as `fake-gen-wh` with `refs` ≤ 12, so a sketch + 8 references can be on:
//!   PRM §13 step 7).
//!   Candidates are a deterministic pattern from the seed, the final prompt and the ordered
//!   reference hashes (sketch included), at the asked size (`aspect`: long side 1024);
//!   `transparent` → RGBA with an opaque disc and alpha 0 around it.
//!
//! Like a provider's server, it keeps jobs and uploads in memory; job progress lives in the
//! opaque record (`fake.polls`), so RGN's persisted record drives the state machine. With a
//! jobs dir (the app's `<data-dir>/cache/provider/fake-jobs/`), every submitted job also gets a
//! copy on disk, so status, cancel and fetch work after an app restart (R48): the job is
//! rebuilt from its record plus that copy.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::pixels::{Grey8, Rgb8, decode_grey8, decode_rgb8, encode_png};
use crate::scenario::fixture_recording;
use crate::scenario::normalise::normalise;
use crate::scenario::overlay::{FAKE_ROWS, row_for};
use crate::{
    AssetRef, Balance, CancelAck, CandidateImage, Candidates, Capabilities, ConnectivityCell,
    ControlRole, Cost, CostAmount, Estimate, JobRequest, JobStatus, ModelFilter, ModelSummary,
    Operation, OutputRequest, Provider, ProviderError, ProviderJobState, ProviderRecord,
    ProviderResult, UploadPurpose,
};

/// The unit the fake reports costs in.
pub const FAKE_UNIT: &str = "fake";

/// The fake models (FR-PRV-03).
pub const FAKE_MODEL_IDS: [&str; 3] = ["fake-white-edit", "fake-black-edit", "fake-alpha-edit"];

/// The fake `generate` models (GEN §10).
pub const FAKE_GENERATE_MODEL_IDS: [&str; 3] = ["fake-gen-wh", "fake-gen-aspect", "fake-gen-refs"];

/// The long side of a fake `aspect` candidate.
pub const FAKE_ASPECT_LONG_SIDE: u32 = 1024;

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
    /// Submit fails with `provider_failure` before the job is accepted: no job, no spend
    /// (FR-RGN-23 → `failed`). Other calls are normal.
    SubmitFailure,
    /// Submit times out (`timeout`); the fake keeps no job (FR-RGN-23 → `lost`).
    SubmitTimeout,
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
            "submit_failure" => FakeCase::SubmitFailure,
            "submit_timeout" => FakeCase::SubmitTimeout,
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
            FakeCase::SubmitFailure => "submit_failure".into(),
            FakeCase::SubmitTimeout => "submit_timeout".into(),
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
    /// Content hashes of the references, in request order (FR-PRV-35).
    reference_hashes: Vec<String>,
    case: FakeCase,
    /// `generate` jobs: what to draw instead of editing a region.
    generate: Option<GenOutput>,
}

/// A fake `generate` job's output: its size, background and the prompt's hash (the pattern
/// depends on it).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct GenOutput {
    w: u32,
    h: u32,
    transparent: bool,
    prompt_hash: String,
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
    /// `(slot, hash)` of the last estimate or submit that passed validation.
    last_references: Vec<(String, String)>,
}

/// The deterministic stand-in provider.
#[derive(Debug, Default)]
pub struct FakeProvider {
    state: Mutex<FakeState>,
    /// Last known connectivity (FR-PRV-07).
    pub connectivity: ConnectivityCell,
    /// Where submitted jobs are copied so they survive a restart (R48); none in unit tests.
    jobs_dir: Option<PathBuf>,
    /// Every `Provider` call so far (PRM FR-PRM-31: a chat makes none).
    calls: std::sync::atomic::AtomicU64,
}

/// `job.json` of a job copy (R48). The rest comes from the job record.
#[derive(serde::Serialize, serde::Deserialize)]
struct JobCopy {
    v: u32,
    /// Content hashes of the references, in request order.
    reference_hashes: Vec<String>,
    /// `generate` jobs only.
    #[serde(default)]
    generate: Option<GenOutput>,
}

/// A handle is a safe single path segment (it names the copy's folder).
fn safe_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
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
    /// A fake that copies every submitted job under `dir` (R48), normally
    /// `<data-dir>/cache/provider/fake-jobs/`. The folder is a rebuildable cache.
    pub fn with_jobs_dir(dir: PathBuf) -> Self {
        FakeProvider {
            jobs_dir: Some(dir),
            ..Self::default()
        }
    }

    pub fn jobs_dir(&self) -> Option<&Path> {
        self.jobs_dir.as_deref()
    }

    /// Writes the job's copy: `<handle>/` with `region.png`, `mask.png` and `job.json`,
    /// built in a temp folder and renamed into place. Failures only warn (it's a cache).
    fn save_job(&self, handle: &str, job: &FakeJob) {
        let Some(dir) = &self.jobs_dir else { return };
        let tmp = dir.join(format!(".{handle}.pf-tmp"));
        let write = || -> std::io::Result<()> {
            let _ = std::fs::remove_dir_all(&tmp);
            std::fs::create_dir_all(&tmp)?;
            std::fs::write(tmp.join("region.png"), &job.region_png)?;
            std::fs::write(tmp.join("mask.png"), &job.mask_png)?;
            let copy = JobCopy {
                v: 1,
                reference_hashes: job.reference_hashes.clone(),
                generate: job.generate.clone(),
            };
            std::fs::write(
                tmp.join("job.json"),
                serde_json::to_vec(&copy).map_err(std::io::Error::other)?,
            )?;
            std::fs::rename(&tmp, dir.join(handle))
        };
        if let Err(e) = write() {
            tracing::warn!(error = %e, "could not copy the fake job; it won't survive a restart");
            let _ = std::fs::remove_dir_all(&tmp);
        }
    }

    /// The job behind a handle: from memory, else rebuilt from its record plus its copy
    /// (R48). Seed, count and case come from the record; images and references from the copy.
    fn job_for(&self, handle: &str, record: &ProviderRecord) -> Option<FakeJob> {
        if let Some(j) = self.lock().jobs.get(handle) {
            return Some(j.clone());
        }
        let dir = self.jobs_dir.as_ref()?.join(handle);
        if !safe_handle(handle) {
            return None;
        }
        let copy: JobCopy =
            serde_json::from_slice(&std::fs::read(dir.join("job.json")).ok()?).ok()?;
        let request = &record.0["request"];
        let job = FakeJob {
            region_png: std::fs::read(dir.join("region.png")).ok()?,
            mask_png: std::fs::read(dir.join("mask.png")).ok()?,
            candidate_count: request["num_samples"].as_u64()? as u32,
            seed: request["seed"].as_u64()?,
            reference_hashes: copy.reference_hashes,
            case: record.0["fake"]["case"]
                .as_str()
                .and_then(FakeCase::parse)
                .unwrap_or_default(),
            generate: copy.generate,
        };
        let mut s = self.lock();
        if dir.join("canceled").exists() {
            s.canceled.insert(handle.to_owned());
        }
        s.jobs.insert(handle.to_owned(), job.clone());
        Some(job)
    }

    /// A handle no job in memory or on disk has yet: the counter restarts with the app.
    fn new_handle(&self, s: &mut FakeState, hash_image: &str) -> String {
        loop {
            s.next_job += 1;
            let handle = format!("fake-job-{}-{}", s.next_job, &hash_image[..8]);
            let on_disk = self
                .jobs_dir
                .as_ref()
                .is_some_and(|d| d.join(&handle).exists());
            if !s.jobs.contains_key(&handle) && !on_disk {
                return handle;
            }
        }
    }

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

    /// `(slot, hash)` of each reference of the last estimate or submit that passed
    /// validation, in request order, with the caller's hash string (FR-PRV-35). Empty before
    /// any such call.
    pub fn last_reference_hashes(&self) -> Vec<(String, String)> {
        self.lock().last_references.clone()
    }

    fn note_references(&self, request: &JobRequest) {
        self.lock().last_references = request
            .references
            .iter()
            .map(|r| (r.slot.clone(), r.hash.clone()))
            .collect();
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

    fn note_call(&self) {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// How many `Provider` calls this fake has served (FR-PRM-31 tests: a chat makes none).
    pub fn call_count(&self) -> u64 {
        self.calls.load(std::sync::atomic::Ordering::Relaxed)
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

    /// Capabilities for one operation: the committed fixture through the real normaliser
    /// (FR-PRV-03).
    fn capabilities_for(model_id: &str, operation: Operation) -> Option<(String, Capabilities)> {
        let row = row_for(&FAKE_ROWS, model_id, operation)?;
        let rec = fixture_recording(model_id, "get-model")?;
        let n = normalise(model_id, &rec.exchanges.first()?.response.body, Some(row)).ok()?;
        Some((n.name, n.capabilities))
    }

    /// Checks a request against the model (incl. the FR-PRV-13 backstop, or the output size
    /// for `generate`) and decodes it.
    fn validate(request: &JobRequest) -> ProviderResult<Capabilities> {
        let (_, caps) = Self::capabilities_for(&request.model_id, request.operation)
            .ok_or_else(|| invalid("model_id"))?;
        if !caps.operations.contains(&request.operation) {
            return Err(invalid("operation"));
        }
        if request.final_prompt.trim().is_empty() {
            return Err(invalid("prompt"));
        }
        // No count input → one result per job (GEN R72).
        let count = caps
            .controls
            .iter()
            .find(|c| c.role == ControlRole::CandidateCount);
        let max = count.and_then(|c| c.max).map_or(1, |m| m as u32);
        if !(1..=max).contains(&request.candidate_count) {
            return Err(invalid(count.map_or("candidate_count", |c| &c.name)));
        }
        if request.operation == Operation::Generate {
            crate::limits::check_output(&caps, request)?;
            crate::limits::check_references(&caps, request)?;
            return Ok(caps);
        }
        crate::limits::check(&caps, request)?;
        crate::limits::check_references(&caps, request)?;
        decode_rgb8(&request.region_png).ok_or_else(|| invalid("image"))?;
        decode_grey8(&request.mask_png).ok_or_else(|| invalid("mask"))?;
        Ok(caps)
    }

    /// A `generate` request's output: the asked size (`aspect`: long side 1024) and whether
    /// the `background` control asks for transparency.
    fn gen_output(caps: &Capabilities, request: &JobRequest) -> GenOutput {
        let (w, h) = match &request.output {
            Some(OutputRequest::Wh { w, h }) => (*w, *h),
            Some(OutputRequest::Aspect { aspect }) => {
                let (a, b) = aspect
                    .split_once(':')
                    .and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?)))
                    .unwrap_or((1.0, 1.0));
                let long = f64::from(FAKE_ASPECT_LONG_SIDE);
                if a >= b {
                    (
                        FAKE_ASPECT_LONG_SIDE,
                        (long * b / a).round().max(1.0) as u32,
                    )
                } else {
                    (
                        (long * a / b).round().max(1.0) as u32,
                        FAKE_ASPECT_LONG_SIDE,
                    )
                }
            }
            None => (FAKE_ASPECT_LONG_SIDE, FAKE_ASPECT_LONG_SIDE),
        };
        let transparent = caps
            .controls
            .iter()
            .filter(|c| c.role == ControlRole::Background)
            .any(|c| request.controls.get(&c.name) == Some(&json!("transparent")));
        GenOutput {
            w,
            h,
            transparent,
            prompt_hash: sha256_hex(request.final_prompt.as_bytes()),
        }
    }

    /// One unit per candidate, plus one per reference in a cost-affecting slot (FR-PRV-35).
    fn cost(candidate_count: u32, ref_units: u32) -> Cost {
        Cost {
            amount: CostAmount::Known(f64::from(candidate_count + ref_units)),
            unit: FAKE_UNIT.into(),
        }
    }

    /// References in cost-affecting slots: only these change the estimate (FR-REF-24).
    fn ref_units(caps: &Capabilities, request: &JobRequest) -> u32 {
        request
            .references
            .iter()
            .filter(|r| {
                caps.slots
                    .iter()
                    .any(|s| s.name == r.slot && s.cost_affecting)
            })
            .count() as u32
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
        ref_units: u32,
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
                "output": request.output,
            },
            "fake": {"case": case.name(), "polls": 0},
        });
        if let Some(handle) = job_handle {
            rec["job_handle"] = json!(handle);
        }
        if !request.references.is_empty() {
            rec["request"]["references"] = request
                .references
                .iter()
                .map(|r| json!({"slot": r.slot, "hash": r.hash}))
                .collect();
            rec["fake"]["ref_units"] = json!(ref_units);
        }
        ProviderRecord(rec)
    }

    /// Uploads every reference once per hash (FR-PRV-34): `(slot, caller's hash, ref)`.
    fn upload_references(&self, request: &JobRequest, force: bool) -> Vec<Value> {
        let mut forced = HashSet::new();
        request
            .references
            .iter()
            .map(|r| {
                let again = force && forced.insert(sha256_hex(&r.bytes));
                let (asset, _) = self.upload_once(&r.bytes, again);
                json!({"slot": r.slot, "hash": r.hash, "ref": asset})
            })
            .collect()
    }

    fn do_submit(&self, request: &JobRequest) -> ProviderResult<ProviderRecord> {
        self.gate()?;
        let caps = Self::validate(request)?;
        self.note_references(request);
        let case = self.case();
        // FR-RGN-23: refused before the provider accepted it, or no answer at all.
        match case {
            FakeCase::SubmitFailure => {
                return Err(ProviderError::ProviderFailure {
                    hint: "the fake provider refused the submit (case submit_failure)".into(),
                });
            }
            FakeCase::SubmitTimeout => return Err(ProviderError::Timeout),
            _ => {}
        }
        if request.operation == Operation::Generate {
            return self.submit_generate(request, &caps, case);
        }
        let (mut image, mut hash_image) = self.upload_once(&request.region_png, false);
        let (mut mask, mut hash_mask) = self.upload_once(&request.mask_png, false);
        let mut references = self.upload_references(request, false);
        let mut reuploads = 0;
        {
            let mut s = self.lock();
            if s.case == FakeCase::AssetMissingOnce && !s.asset_missing_fired {
                // The provider says the asset is gone: re-upload once and retry (FR-PRV-15).
                s.asset_missing_fired = true;
                drop(s);
                (image, hash_image) = self.upload_once(&request.region_png, true);
                (mask, hash_mask) = self.upload_once(&request.mask_png, true);
                references = self.upload_references(request, true);
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
        let job = FakeJob {
            region_png: request.region_png.clone(),
            mask_png: request.mask_png.clone(),
            candidate_count: request.candidate_count,
            seed,
            reference_hashes: request
                .references
                .iter()
                .map(|r| sha256_hex(&r.bytes))
                .collect(),
            case,
            generate: None,
        };
        let mut s = self.lock();
        let handle = self.new_handle(&mut s, &hash_image);
        s.jobs.insert(handle.clone(), job.clone());
        drop(s);
        self.save_job(&handle, &job);
        let ref_units = Self::ref_units(&caps, request);
        let mut rec = self.record(request, Some(&handle), case, ref_units);
        rec.0["request"]["seed"] = json!(seed);
        rec.0["asset_refs"] = json!({
            "image": image, "mask": mask, "hash_image": hash_image, "hash_mask": hash_mask,
        });
        if !references.is_empty() {
            rec.0["asset_refs"]["references"] = Value::Array(references);
        }
        rec.0["fake"]["reuploads"] = json!(reuploads);
        Ok(rec)
    }

    /// `generate`: no source or mask to upload; references only (the sketch first in its
    /// slot, as sent). The seed defaults to one derived from prompt, output and references.
    fn submit_generate(
        &self,
        request: &JobRequest,
        caps: &Capabilities,
        case: FakeCase,
    ) -> ProviderResult<ProviderRecord> {
        let mut references = self.upload_references(request, false);
        let mut reuploads = 0;
        {
            let mut s = self.lock();
            if s.case == FakeCase::AssetMissingOnce
                && !s.asset_missing_fired
                && !references.is_empty()
            {
                s.asset_missing_fired = true;
                drop(s);
                references = self.upload_references(request, true);
                reuploads = 1;
            }
        }
        let output = Self::gen_output(caps, request);
        let reference_hashes: Vec<String> = request
            .references
            .iter()
            .map(|r| sha256_hex(&r.bytes))
            .collect();
        let seed = request.seed.unwrap_or_else(|| {
            let mut h = Sha256::new();
            h.update(output.prompt_hash.as_bytes());
            h.update(output.w.to_le_bytes());
            h.update(output.h.to_le_bytes());
            for r in &reference_hashes {
                h.update(r.as_bytes());
            }
            let d = h.finalize();
            u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]])
        });
        let job = FakeJob {
            region_png: vec![],
            mask_png: vec![],
            candidate_count: request.candidate_count,
            seed,
            reference_hashes,
            case,
            generate: Some(output.clone()),
        };
        let mut s = self.lock();
        let handle = self.new_handle(&mut s, &output.prompt_hash);
        s.jobs.insert(handle.clone(), job.clone());
        drop(s);
        self.save_job(&handle, &job);
        let ref_units = Self::ref_units(caps, request);
        let mut rec = self.record(request, Some(&handle), case, ref_units);
        rec.0["request"]["seed"] = json!(seed);
        if !references.is_empty() {
            rec.0["asset_refs"]["references"] = Value::Array(references);
        }
        rec.0["fake"]["reuploads"] = json!(reuploads);
        rec.0["fake"]["output"] =
            json!({"w": output.w, "h": output.h, "transparent": output.transparent});
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
        let ref_units = job.0["fake"]["ref_units"].as_u64().unwrap_or(0) as u32;
        let known = self.job_for(&handle, job);
        let (canceled, case) = {
            let s = self.lock();
            let case = known.map_or(s.case, |j| j.case);
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
            cost: (state == ProviderJobState::Succeeded).then(|| Self::cost(count, ref_units)),
            error,
            provider: rec,
        })
    }

    fn do_fetch(&self, job: &ProviderRecord) -> ProviderResult<Candidates> {
        self.gate()?;
        let handle = handle_of(job)?;
        let fake_job =
            self.job_for(&handle, job)
                .ok_or_else(|| ProviderError::ProviderFailure {
                    hint: "the fake provider has no copy of this job".into(),
                })?;
        if let Some(out) = &fake_job.generate {
            return Ok(Candidates {
                images: generated_candidates(&handle, &fake_job, out),
                provider: job.clone(),
            });
        }
        let region = decode_rgb8(&fake_job.region_png).ok_or_else(|| invalid("image"))?;
        let mask = decode_grey8(&fake_job.mask_png).ok_or_else(|| invalid("mask"))?;
        let images = (0..fake_job.candidate_count)
            .map(|i| {
                let seed = fake_job.seed.wrapping_add(u64::from(i));
                // The references change the pattern, not the reported seed (FR-PRV-35).
                let pattern = pattern_seed(seed, &fake_job.reference_hashes);
                let bytes = match fake_job.case {
                    FakeCase::BadCandidate if i == 0 => b"this is not an image".to_vec(),
                    FakeCase::AspectMismatch(pct) => {
                        widen(&render_candidate(&region, &mask, pattern, 0), pct)
                    }
                    FakeCase::Offset(levels) => {
                        encode(&render_candidate(&region, &mask, pattern, levels))
                    }
                    _ => encode(&render_candidate(&region, &mask, pattern, 0)),
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
        self.note_call();
        let out = self.gate().map(|()| {
            FAKE_MODEL_IDS
                .iter()
                .chain(FAKE_GENERATE_MODEL_IDS.iter())
                .filter_map(|id| {
                    let found: Vec<(String, Capabilities)> =
                        [Operation::EditMasked, Operation::Generate]
                            .into_iter()
                            .filter_map(|op| Self::capabilities_for(id, op))
                            .collect();
                    let name = found.first()?.0.clone();
                    let mut operations: Vec<Operation> =
                        found.into_iter().flat_map(|(_, c)| c.operations).collect();
                    operations.sort();
                    operations.dedup();
                    Some(ModelSummary {
                        id: (*id).to_owned(),
                        name,
                        operations,
                    })
                })
                .filter(|m| match filter.operation {
                    Some(op) => m.operations.contains(&op),
                    None => !m.operations.is_empty(),
                })
                .collect()
        });
        self.done(out)
    }

    fn get_capabilities(
        &self,
        model_id: &str,
        operation: Operation,
    ) -> impl Future<Output = ProviderResult<Capabilities>> + Send {
        self.note_call();
        let out = self.gate().and_then(|()| {
            Self::capabilities_for(model_id, operation)
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
        self.note_call();
        let out = self.gate().map(|()| self.upload_once(bytes, false).0);
        self.done(out)
    }

    fn estimate(
        &self,
        request: &JobRequest,
    ) -> impl Future<Output = ProviderResult<Estimate>> + Send {
        self.note_call();
        let out = self
            .gate()
            .and_then(|()| Self::validate(request))
            .map(|caps| {
                self.note_references(request);
                let case = self.case();
                let ref_units = Self::ref_units(&caps, request);
                Estimate {
                    cost: if case == FakeCase::EstimateUnknown {
                        Cost {
                            amount: CostAmount::Unknown,
                            unit: FAKE_UNIT.into(),
                        }
                    } else {
                        Self::cost(request.candidate_count, ref_units)
                    },
                    provider: self.record(request, None, case, ref_units),
                }
            });
        self.done(out)
    }

    fn submit(
        &self,
        request: &JobRequest,
    ) -> impl Future<Output = ProviderResult<ProviderRecord>> + Send {
        self.note_call();
        let out = self.do_submit(request);
        self.done(out)
    }

    fn status(
        &self,
        job: &ProviderRecord,
    ) -> impl Future<Output = ProviderResult<JobStatus>> + Send {
        self.note_call();
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
        self.note_call();
        let out = self.gate().and_then(|()| handle_of(job)).map(|handle| {
            self.job_for(&handle, job);
            let mut s = self.lock();
            if s.case == FakeCase::NotCancellable {
                return CancelAck::NotCancellable;
            }
            s.canceled.insert(handle.clone());
            drop(s);
            // The cancel survives a restart too (R48).
            if let Some(dir) = self.jobs_dir.as_ref().filter(|_| safe_handle(&handle)) {
                let copy = dir.join(&handle);
                if copy.is_dir()
                    && let Err(e) = std::fs::write(copy.join("canceled"), b"")
                {
                    tracing::warn!(error = %e, "could not mark the fake job canceled");
                }
            }
            CancelAck::Canceled
        });
        self.done(out)
    }

    fn fetch_candidates(
        &self,
        job: &ProviderRecord,
    ) -> impl Future<Output = ProviderResult<Candidates>> + Send {
        self.note_call();
        let out = self.do_fetch(job);
        self.done(out)
    }

    fn balance(&self) -> impl Future<Output = ProviderResult<Balance>> + Send {
        self.note_call();
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

/// The pattern seed: the candidate's seed, with the ordered reference hashes mixed in. With
/// no references it is the seed itself, so those candidates don't change (FR-PRV-35).
fn pattern_seed(seed: u64, reference_hashes: &[String]) -> u64 {
    if reference_hashes.is_empty() {
        return seed;
    }
    let mut h = Sha256::new();
    h.update(seed.to_le_bytes());
    for hash in reference_hashes {
        h.update(hash.as_bytes());
        h.update([0]);
    }
    let d = h.finalize();
    u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]])
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

/// A fake `generate` job's candidates: a seeded pattern at the asked size, RGBA with an
/// opaque disc when transparent. Appendix D cases apply as for edits.
fn generated_candidates(handle: &str, job: &FakeJob, out: &GenOutput) -> Vec<CandidateImage> {
    (0..job.candidate_count)
        .map(|i| {
            let seed = job.seed.wrapping_add(u64::from(i));
            let mut hashes = vec![out.prompt_hash.clone()];
            hashes.extend(job.reference_hashes.iter().cloned());
            let pattern = pattern_seed(seed, &hashes);
            let levels = match job.case {
                FakeCase::Offset(l) => l,
                _ => 0,
            };
            let (channels, data) = render_generated(out, pattern, levels);
            let bytes = match job.case {
                FakeCase::BadCandidate if i == 0 => b"this is not an image".to_vec(),
                FakeCase::AspectMismatch(pct) => {
                    let (w, data) = widen_px(out.w, out.h, channels, &data, pct);
                    encode_px(w, out.h, channels, &data)
                }
                _ => encode_px(out.w, out.h, channels, &data),
            };
            CandidateImage {
                bytes,
                seed: Some(seed),
                provider_meta: json!({"asset_id": format!("{handle}-c{i}"), "seed": seed}),
            }
        })
        .collect()
}

/// A seeded 32 px checker of two tints with per-pixel noise (+ `levels`, clamped), and with
/// `transparent` an alpha channel: 255 inside a centred disc (radius 45 % of the short side),
/// 0 outside. Returns `(channels, pixels)`.
fn render_generated(out: &GenOutput, seed: u64, levels: i32) -> (usize, Vec<u8>) {
    let base = splitmix(seed);
    let tint = |k: u32| {
        [
            ((base >> k) & 0xFF) as i32,
            ((base >> (k + 8)) & 0xFF) as i32,
            ((base >> (k + 16)) & 0xFF) as i32,
        ]
    };
    let (a, b) = (tint(0), tint(24));
    let channels = if out.transparent { 4 } else { 3 };
    let (w, h) = (u64::from(out.w), u64::from(out.h));
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let r = w.min(h) as f64 * 0.45;
    let mut data = Vec::with_capacity((w * h) as usize * channels);
    for y in 0..h {
        for x in 0..w {
            let t = if (x / 32 + y / 32) % 2 == 0 { a } else { b };
            let noise = (splitmix(base ^ (y * w + x)) & 0x1F) as i32;
            for c in t {
                data.push((c + noise + levels).clamp(0, 255) as u8);
            }
            if out.transparent {
                let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                data.push(if dx * dx + dy * dy <= r * r { 255 } else { 0 });
            }
        }
    }
    (channels, data)
}

fn encode_px(w: u32, h: u32, channels: usize, data: &[u8]) -> Vec<u8> {
    let color = if channels == 4 {
        png::ColorType::Rgba
    } else {
        png::ColorType::Rgb
    };
    encode_png(w, h, color, data)
}

/// Nearest-neighbour horizontal stretch by `pct` % of RGB or RGBA pixels.
fn widen_px(width: u32, height: u32, channels: usize, data: &[u8], pct: u32) -> (u32, Vec<u8>) {
    let w = (u64::from(width) * u64::from(100 + pct)).div_ceil(100) as u32;
    let mut out = Vec::with_capacity(w as usize * height as usize * channels);
    for y in 0..height {
        for x in 0..w {
            let sx = (u64::from(x) * u64::from(width) / u64::from(w)) as u32;
            let i = (y * width + sx) as usize * channels;
            out.extend_from_slice(&data[i..i + channels]);
        }
    }
    (w, out)
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
mod generate_tests;

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
            output: None,
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
        let models = run(p.list_models(&ModelFilter {
            operation: Some(Operation::EditMasked),
        }))
        .unwrap();
        assert_eq!(models.len(), 3);
        let conventions: HashSet<_> = models
            .iter()
            .map(|m| {
                let caps = run(p.get_capabilities(&m.id, Operation::EditMasked)).unwrap();
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

    /// The candidates of a request with no references, pinned before references existed
    /// (FR-PRV-35: "byte-identical to before").
    #[test]
    fn fr_prv_35_no_references_keeps_the_pre_reference_candidates() {
        let c = run_job(&FakeProvider::default(), &request(Some(7)));
        // Decoded pixels, not PNG bytes (OQ-IMG-07): an encoder update can't break this.
        let digest: Vec<String> = c
            .images
            .iter()
            .map(|i| sha256_hex(&decode_rgb8(&i.bytes).unwrap().data))
            .collect();
        assert_eq!(
            digest,
            [
                "5d022c9b7f8f2f690897ff9b2f9e31b1c62fb63f35b6a5f5beaa17913e3b4144",
                "630bb631f3be46d3c03800121b4d5576ed9b20174e7efd05be97a3f5b79ee520",
                "08e55fe6cb7cd1d3499f068350978ed1e418b5fb6f6f27bf36fdbbfa44389cf6",
            ]
        );
    }

    /// A tiny PNG reference whose content depends on `n`.
    fn ref_png(n: u8) -> Vec<u8> {
        encode_png(2, 2, png::ColorType::Rgb, &[n; 12])
    }

    fn reference(slot: &str, bytes: Vec<u8>) -> crate::ReferenceImage {
        crate::ReferenceImage {
            slot: slot.into(),
            role: None,
            hash: format!("sha256:{}", sha256_hex(&bytes)),
            bytes,
        }
    }

    fn black(references: Vec<crate::ReferenceImage>) -> JobRequest {
        JobRequest {
            model_id: "fake-black-edit".into(),
            references,
            ..request(Some(7))
        }
    }

    #[test]
    fn fr_prv_35_fake_models_have_their_reference_slots() {
        use crate::ReferenceRole::{Identity, Style};
        let p = FakeProvider::default();
        let slots = |id: &str| -> Vec<_> {
            run(p.get_capabilities(id, Operation::EditMasked))
                .unwrap()
                .slots
                .into_iter()
                .map(|s| (s.name, s.role, s.min, s.max, s.cost_affecting, s.array))
                .collect()
        };
        assert_eq!(
            slots("fake-white-edit"),
            [("style_refs".into(), Some(Style), 0, 3, false, true)]
        );
        assert_eq!(
            slots("fake-black-edit"),
            [
                ("style_refs".into(), Some(Style), 0, 2, true, true),
                ("identity_ref".into(), Some(Identity), 0, 1, false, false),
            ]
        );
        assert!(slots("fake-alpha-edit").is_empty());
        assert_eq!(
            run(p.list_models(&ModelFilter {
                operation: Some(Operation::EditMasked)
            }))
            .unwrap()
            .len(),
            3
        );
    }

    #[test]
    fn fr_prv_35_references_change_candidates_deterministically() {
        let refs = |a: u8, b: u8| {
            black(vec![
                reference("style_refs", ref_png(a)),
                reference("style_refs", ref_png(b)),
            ])
        };
        let none = run_job(&FakeProvider::default(), &black(vec![]));
        let ab = run_job(&FakeProvider::default(), &refs(1, 2));
        assert_eq!(
            ab.images,
            run_job(&FakeProvider::default(), &refs(1, 2)).images
        );
        for other in [refs(1, 3), refs(2, 1)] {
            let c = run_job(&FakeProvider::default(), &other);
            assert_ne!(
                c.images[0].bytes, ab.images[0].bytes,
                "other or reordered refs"
            );
        }
        assert_ne!(ab.images[0].bytes, none.images[0].bytes);
        // The reported seeds stay the request's; unmasked pixels stay unchanged.
        let seeds = |c: &Candidates| c.images.iter().map(|i| i.seed).collect::<Vec<_>>();
        assert_eq!(seeds(&ab), seeds(&none));
        let original = decode_rgb8(&black(vec![]).region_png).unwrap();
        let cand = decode_rgb8(&ab.images[0].bytes).unwrap();
        let w = original.width as usize;
        for i in (0..w * original.height as usize).filter(|i| i % w >= w / 2) {
            assert_eq!(cand.data[i * 3..i * 3 + 3], original.data[i * 3..i * 3 + 3]);
        }
    }

    #[test]
    fn fr_prv_35_only_cost_affecting_references_change_the_estimate() {
        let p = FakeProvider::default();
        let cost = |req: &JobRequest| run(p.estimate(req)).unwrap().cost.amount;
        // fake-white-edit: style refs are free.
        let mut white = request(Some(7));
        assert_eq!(cost(&white), CostAmount::Known(3.0));
        white.references = (1..=3)
            .map(|n| reference("style_refs", ref_png(n)))
            .collect();
        assert_eq!(cost(&white), CostAmount::Known(3.0));
        // fake-black-edit: +1 per style ref; the identity ref is free.
        let style = |n| reference("style_refs", ref_png(n));
        assert_eq!(cost(&black(vec![style(1)])), CostAmount::Known(4.0));
        let full = black(vec![
            style(1),
            style(2),
            reference("identity_ref", ref_png(9)),
        ]);
        assert_eq!(cost(&full), CostAmount::Known(5.0));
        // The charge on success matches the estimate.
        let (_, rec) = states(&p, &full, 2);
        assert_eq!(
            run(p.status(&rec)).unwrap().cost.unwrap().amount,
            CostAmount::Known(5.0)
        );
    }

    #[test]
    fn fr_prv_35_the_fake_exposes_the_last_reference_hashes() {
        let p = FakeProvider::default();
        assert!(p.last_reference_hashes().is_empty());
        let req = black(vec![
            reference("identity_ref", ref_png(9)),
            reference("style_refs", ref_png(1)),
        ]);
        let want: Vec<_> = req
            .references
            .iter()
            .map(|r| (r.slot.clone(), r.hash.clone()))
            .collect();
        run(p.estimate(&req)).unwrap();
        assert_eq!(p.last_reference_hashes(), want);
        run(p.submit(&black(vec![]))).unwrap();
        assert!(p.last_reference_hashes().is_empty());
        run(p.submit(&req)).unwrap();
        // A refused request doesn't overwrite it.
        let mut bad = req.clone();
        bad.references[0].slot = "nope".into();
        run(p.submit(&bad)).unwrap_err();
        assert_eq!(p.last_reference_hashes(), want);
    }

    #[test]
    fn fr_prv_33_the_fake_backstops_references() {
        let p = FakeProvider::default();
        let bad = invalid("reference");
        let style = |n| reference("style_refs", ref_png(n));
        let mut wrong_hash = style(1);
        wrong_hash.hash = format!("sha256:{}", sha256_hex(b"other"));
        for refs in [
            vec![reference("moodBoard", ref_png(1))],
            vec![style(1), style(2), style(3)],
            vec![
                reference("identity_ref", ref_png(1)),
                reference("identity_ref", ref_png(2)),
            ],
            vec![reference("style_refs", b"not an image".to_vec())],
            vec![wrong_hash],
        ] {
            assert_eq!(run(p.estimate(&black(refs.clone()))).unwrap_err(), bad);
            assert_eq!(run(p.submit(&black(refs))).unwrap_err(), bad);
        }
    }

    #[test]
    fn fr_prv_34_the_fake_uploads_references_once_per_hash_and_reuploads_them() {
        let p = case("asset_missing_once");
        let req = black(vec![
            reference("style_refs", ref_png(1)),
            reference("identity_ref", ref_png(1)),
        ]);
        let refs_of = |rec: &ProviderRecord| -> Vec<String> {
            rec.0["asset_refs"]["references"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x["ref"].as_str().unwrap().to_owned())
                .collect()
        };
        let before = run(p.upload(&ref_png(1), UploadPurpose::Reference)).unwrap();
        let first = run(p.submit(&req)).unwrap();
        assert_eq!(first.0["fake"]["reuploads"], 1);
        let first_refs = refs_of(&first);
        assert_eq!(first_refs[0], first_refs[1], "identical bytes: one upload");
        assert_ne!(first_refs[0], before.0, "re-uploaded with the rest");
        let second = run(p.submit(&req)).unwrap();
        assert_eq!(refs_of(&second), first_refs, "new refs are cached");
        assert_eq!(
            second.0["asset_refs"]["references"][1],
            json!({"slot": "identity_ref", "hash": req.references[1].hash, "ref": first_refs[1]})
        );
    }

    /// FR-RGN-23: a refused submit (→ `failed`) and a submit timeout (→ `lost`). Only
    /// submit fails; no job is kept, so nothing is spent.
    #[test]
    fn fr_prv_04_fr_rgn_23_submit_failure_and_submit_timeout() {
        for (name, code) in [
            ("submit_failure", "provider_failure"),
            ("submit_timeout", "timeout"),
        ] {
            let p = case(name);
            let req = request(Some(1));
            run(p.list_models(&ModelFilter::default())).unwrap();
            run(p.estimate(&req)).unwrap();
            assert_eq!(run(p.submit(&req)).unwrap_err().code(), code, "{name}");
            run(p.balance()).unwrap();
            p.set_case(FakeCase::Success);
            let rec = run(p.submit(&req)).unwrap();
            assert!(
                rec.0["job_handle"]
                    .as_str()
                    .unwrap()
                    .starts_with("fake-job-1-"),
                "{name}: the failed submit made no job"
            );
        }
    }

    /// R48: a job submitted before an app restart is rebuilt from its record plus the copy
    /// under the jobs dir, and gives the same states and candidates as without a restart.
    #[test]
    fn fr_prv_03_r48_fake_jobs_survive_an_app_restart() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = dir.path().join("fake-jobs");
        let req = black(vec![
            reference("style_refs", ref_png(1)),
            reference("identity_ref", ref_png(2)),
        ]);
        let uninterrupted = {
            let p = case("slow(2)");
            let (s, rec) = states(&p, &req, 4);
            assert_eq!(s, [Queued, Running, Running, Succeeded]);
            run(p.fetch_candidates(&rec)).unwrap()
        };

        let before = FakeProvider::with_jobs_dir(jobs.clone());
        before.set_case(FakeCase::parse("slow(2)").unwrap());
        let (s, rec) = states(&before, &req, 2);
        assert_eq!(s, [Queued, Running]);
        let handle = rec.0["job_handle"].as_str().unwrap().to_owned();
        for f in ["region.png", "mask.png", "job.json"] {
            assert!(jobs.join(&handle).join(f).is_file(), "{f}");
        }
        drop(before);

        // A new app: the case is back to `success`, but the job keeps its own.
        let after = FakeProvider::with_jobs_dir(jobs.clone());
        let mut rec = rec;
        let mut s = vec![];
        for _ in 0..2 {
            let st = run(after.status(&rec)).unwrap();
            s.push(st.state);
            rec = st.provider;
        }
        assert_eq!(s, [Running, Succeeded]);
        assert_eq!(
            run(after.fetch_candidates(&rec)).unwrap().images,
            uninterrupted.images
        );
        // The restarted counter never reuses a handle that has a copy.
        let again = run(after.submit(&req)).unwrap();
        assert_ne!(again.0["job_handle"], json!(handle));
        // Without a copy the job can't be fetched: a provider failure, never a panic.
        let elsewhere = FakeProvider::default();
        assert_eq!(
            run(elsewhere.fetch_candidates(&rec)).unwrap_err().code(),
            "provider_failure"
        );
    }

    #[test]
    fn fr_prv_03_r48_a_cancel_survives_an_app_restart() {
        let dir = tempfile::tempdir().unwrap();
        let before = FakeProvider::with_jobs_dir(dir.path().to_path_buf());
        let rec = run(before.submit(&request(Some(1)))).unwrap();
        assert_eq!(run(before.cancel(&rec)).unwrap(), CancelAck::Canceled);
        drop(before);
        let after = FakeProvider::with_jobs_dir(dir.path().to_path_buf());
        assert_eq!(run(after.status(&rec)).unwrap().state, Canceled);
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
            "submit_failure",
            "submit_timeout",
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
                run(p.get_capabilities("fake-white-edit", Operation::EditMasked))
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

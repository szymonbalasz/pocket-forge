//! The fake provider (FR-PRV-03/04/05): deterministic, offline, free.
//!
//! The skeleton implements the `success` case. The other Appendix D cases, and capability
//! fixtures run through the real normaliser, land in PRV slice 1's completion and slice 3.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::io::Cursor;
use std::sync::Mutex;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    AssetRef, Balance, CancelAck, CandidateImage, Candidates, Capabilities, Control, ControlRole,
    ControlType, Cost, CostAmount, Estimate, InputPx, JobRequest, JobStatus, MaskConvention,
    MaskSupport, ModelFilter, ModelSummary, Operation, Provider, ProviderError, ProviderJobState,
    ProviderRecord, ProviderResult, SoftRule, UploadPurpose,
};

/// The unit the fake reports costs in.
pub const FAKE_UNIT: &str = "fake";

/// Fake behaviours (provider spec Appendix D). Only `success` exists in the skeleton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FakeCase {
    /// queued → running → succeeded; candidates per FR-PRV-03.
    #[default]
    Success,
}

impl FakeCase {
    /// Parses a case name (`PF_FAKE_CASE`, `provider_fake_set_case`).
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim() {
            "success" => Some(FakeCase::Success),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            FakeCase::Success => "success",
        }
    }
}

/// What the fake remembers about a submitted job (in memory, like a provider's server side).
#[derive(Debug, Clone)]
struct FakeJob {
    region_png: Vec<u8>,
    mask_png: Vec<u8>,
    candidate_count: u32,
    seed: u64,
}

#[derive(Debug, Default)]
struct FakeState {
    case: FakeCase,
    next_job: u64,
    uploads: HashMap<String, Vec<u8>>,
    jobs: HashMap<String, FakeJob>,
    canceled: HashSet<String>,
}

/// The deterministic stand-in provider. It has no transport, so it can't open a connection.
#[derive(Debug, Default)]
pub struct FakeProvider {
    state: Mutex<FakeState>,
}

/// The three fake models, one per mask convention (FR-PRV-03).
pub const FAKE_MODELS: [(&str, &str, MaskConvention, SoftRule); 3] = [
    (
        "fake-white-edit",
        "Fake (white = edit)",
        MaskConvention::WhiteEdit,
        SoftRule::Threshold(128),
    ),
    (
        "fake-black-edit",
        "Fake (black = edit)",
        MaskConvention::BlackEdit,
        SoftRule::Threshold(128),
    ),
    (
        "fake-alpha-edit",
        "Fake (transparent = edit)",
        MaskConvention::AlphaTransparentEdit,
        SoftRule::PassThrough,
    ),
];

impl FakeProvider {
    pub fn with_case(case: FakeCase) -> Self {
        let p = Self::default();
        p.set_case(case);
        p
    }

    pub fn set_case(&self, case: FakeCase) {
        self.lock().case = case;
    }

    pub fn case(&self) -> FakeCase {
        self.lock().case
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        // A panic while holding the lock can't leave the fake's maps inconsistent; keep going.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn capabilities_for(model_id: &str) -> Option<Capabilities> {
        let (id, _, convention, soft) = FAKE_MODELS.iter().find(|m| m.0 == model_id)?;
        let control = |name: &str, kind, role, required, cost_affecting| Control {
            name: name.into(),
            kind,
            role,
            min: None,
            max: None,
            step: None,
            allowed: vec![],
            default: None,
            required,
            cost_affecting,
        };
        let mut count = control(
            "num_samples",
            ControlType::Int,
            ControlRole::CandidateCount,
            false,
            true,
        );
        count.min = Some(1.0);
        count.max = Some(4.0);
        count.step = Some(1.0);
        count.default = Some(json!(3));
        Some(Capabilities {
            model_id: (*id).into(),
            operations: vec![Operation::EditMasked],
            controls: vec![
                control(
                    "image",
                    ControlType::Image,
                    ControlRole::Source,
                    true,
                    false,
                ),
                control("mask", ControlType::Mask, ControlRole::Mask, true, false),
                control(
                    "prompt",
                    ControlType::String,
                    ControlRole::Prompt,
                    true,
                    false,
                ),
                count,
                control("seed", ControlType::Seed, ControlRole::Seed, false, false),
            ],
            rules: vec![],
            mask: Some(MaskSupport {
                convention: *convention,
                soft: *soft,
            }),
            input_px: InputPx {
                min: 64,
                max: 4096,
                multiple_of: 8,
            },
            max_bytes: 50 * 1024 * 1024,
            seed: true,
            prompt_required: true,
            slots: vec![],
            warnings: vec![],
        })
    }

    /// Checks a request against the model and decodes its images.
    fn validate(request: &JobRequest) -> ProviderResult<(Rgb8, Grey8)> {
        let caps = Self::capabilities_for(&request.model_id).ok_or_else(|| invalid("model_id"))?;
        if !caps.operations.contains(&request.operation) {
            return Err(invalid("operation"));
        }
        if request.final_prompt.trim().is_empty() {
            return Err(invalid("prompt"));
        }
        if !(1..=4).contains(&request.candidate_count) {
            return Err(invalid("num_samples"));
        }
        let region = decode_rgb8(&request.region_png).ok_or_else(|| invalid("image"))?;
        let mask = decode_grey8(&request.mask_png).ok_or_else(|| invalid("mask"))?;
        let px = caps.input_px;
        let side_ok = |s: u32| s >= px.min && s <= px.max && s.is_multiple_of(px.multiple_of);
        if !side_ok(region.width) || !side_ok(region.height) {
            return Err(invalid("image"));
        }
        if (mask.width, mask.height) != (region.width, region.height) {
            return Err(invalid("mask"));
        }
        Ok((region, mask))
    }

    fn cost(candidate_count: u32) -> Cost {
        Cost {
            amount: CostAmount::Known(f64::from(candidate_count)),
            unit: FAKE_UNIT.into(),
        }
    }

    fn record(&self, request: &JobRequest, job_handle: Option<&str>, polls: u64) -> ProviderRecord {
        let case = self.case();
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
            "fake": {"case": case.name(), "polls": polls},
        });
        if let Some(handle) = job_handle {
            rec["job_handle"] = json!(handle);
        }
        ProviderRecord(rec)
    }
}

fn invalid(field: &str) -> ProviderError {
    ProviderError::InvalidRequest {
        field: field.into(),
    }
}

fn handle_of(job: &ProviderRecord) -> ProviderResult<String> {
    job.0
        .get("job_handle")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| invalid("job_handle"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Provider for FakeProvider {
    fn list_models(
        &self,
        filter: &ModelFilter,
    ) -> impl Future<Output = ProviderResult<Vec<ModelSummary>>> + Send {
        let out = FAKE_MODELS
            .iter()
            .filter(|_| matches!(filter.operation, None | Some(Operation::EditMasked)))
            .map(|(id, name, _, _)| ModelSummary {
                id: (*id).into(),
                name: (*name).into(),
                operations: vec![Operation::EditMasked],
            })
            .collect();
        std::future::ready(Ok(out))
    }

    fn get_capabilities(
        &self,
        model_id: &str,
    ) -> impl Future<Output = ProviderResult<Capabilities>> + Send {
        std::future::ready(Self::capabilities_for(model_id).ok_or_else(|| invalid("model_id")))
    }

    fn upload(
        &self,
        bytes: &[u8],
        _purpose: UploadPurpose,
    ) -> impl Future<Output = ProviderResult<AssetRef>> + Send {
        let hash = sha256_hex(bytes);
        let id = format!("fake-asset-{}", &hash[..16]);
        self.lock().uploads.insert(id.clone(), bytes.to_vec());
        std::future::ready(Ok(AssetRef(id)))
    }

    fn estimate(
        &self,
        request: &JobRequest,
    ) -> impl Future<Output = ProviderResult<Estimate>> + Send {
        let out = Self::validate(request).map(|_| Estimate {
            cost: Self::cost(request.candidate_count),
            provider: self.record(request, None, 0),
        });
        std::future::ready(out)
    }

    fn submit(
        &self,
        request: &JobRequest,
    ) -> impl Future<Output = ProviderResult<ProviderRecord>> + Send {
        let out = Self::validate(request).map(|_| {
            // The seed defaults to one derived from the request, so a repeat is identical.
            let seed = request.seed.unwrap_or_else(|| {
                let mut h = Sha256::new();
                h.update(&request.region_png);
                h.update(&request.mask_png);
                h.update(request.final_prompt.as_bytes());
                let d = h.finalize();
                u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]])
            });
            let mut state = self.lock();
            state.next_job += 1;
            let handle = format!(
                "fake-job-{}-{}",
                state.next_job,
                &sha256_hex(&request.region_png)[..8]
            );
            state.jobs.insert(
                handle.clone(),
                FakeJob {
                    region_png: request.region_png.clone(),
                    mask_png: request.mask_png.clone(),
                    candidate_count: request.candidate_count,
                    seed,
                },
            );
            drop(state);
            let mut rec = self.record(request, Some(&handle), 0);
            rec.0["request"]["seed"] = json!(seed);
            rec
        });
        std::future::ready(out)
    }

    fn status(
        &self,
        job: &ProviderRecord,
    ) -> impl Future<Output = ProviderResult<JobStatus>> + Send {
        let out = handle_of(job).map(|handle| {
            let polls = job.0["fake"]["polls"].as_u64().unwrap_or(0);
            let count = job.0["request"]["num_samples"].as_u64().unwrap_or(1) as u32;
            let canceled = self.lock().canceled.contains(&handle);
            let (state, progress) = match (canceled, polls) {
                (true, _) => (ProviderJobState::Canceled, 0.0),
                (false, 0) => (ProviderJobState::Queued, 0.0),
                (false, 1) => (ProviderJobState::Running, 0.5),
                (false, _) => (ProviderJobState::Succeeded, 1.0),
            };
            let mut rec = job.clone();
            rec.0["fake"]["polls"] = json!(polls + 1);
            rec.0["raw_status"] = json!(state);
            JobStatus {
                state,
                progress,
                cost: (state == ProviderJobState::Succeeded).then(|| Self::cost(count)),
                error: None,
                provider: rec,
            }
        });
        std::future::ready(out)
    }

    fn cancel(
        &self,
        job: &ProviderRecord,
    ) -> impl Future<Output = ProviderResult<CancelAck>> + Send {
        let out = handle_of(job).map(|handle| {
            self.lock().canceled.insert(handle);
            CancelAck::Canceled
        });
        std::future::ready(out)
    }

    fn fetch_candidates(
        &self,
        job: &ProviderRecord,
    ) -> impl Future<Output = ProviderResult<Candidates>> + Send {
        let out = handle_of(job).and_then(|handle| {
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
                    CandidateImage {
                        bytes: render_candidate(&region, &mask, seed),
                        seed: Some(seed),
                        provider_meta: json!({"asset_id": format!("{handle}-c{i}"), "seed": seed}),
                    }
                })
                .collect();
            Ok(Candidates {
                images,
                provider: job.clone(),
            })
        });
        std::future::ready(out)
    }

    fn balance(&self) -> impl Future<Output = ProviderResult<Balance>> + Send {
        std::future::ready(Ok(Balance::Known {
            remaining: 1000.0,
            unit: FAKE_UNIT.into(),
            period_end: None,
        }))
    }
}

struct Rgb8 {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

struct Grey8 {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

fn decode(bytes: &[u8]) -> Option<(u32, u32, png::ColorType, Vec<u8>)> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, info.color_type, buf))
}

fn decode_rgb8(bytes: &[u8]) -> Option<Rgb8> {
    let (width, height, color, buf) = decode(bytes)?;
    let data = match color {
        png::ColorType::Rgb => buf,
        png::ColorType::Rgba => buf
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g]).collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    Some(Rgb8 {
        width,
        height,
        data,
    })
}

fn decode_grey8(bytes: &[u8]) -> Option<Grey8> {
    let (width, height, color, data) = decode(bytes)?;
    (color == png::ColorType::Grayscale).then_some(Grey8 {
        width,
        height,
        data,
    })
}

/// SplitMix64: a tiny deterministic generator for the seed-derived pattern.
fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Masked pixels get a seed-derived pattern, blended by the mask value; unmasked pixels
/// (mask 0) are copied unchanged (FR-PRV-03).
fn render_candidate(region: &Rgb8, mask: &Grey8, seed: u64) -> Vec<u8> {
    let base = splitmix(seed);
    let tint = [
        (base & 0xFF) as u32,
        ((base >> 8) & 0xFF) as u32,
        ((base >> 16) & 0xFF) as u32,
    ];
    let mut out = region.data.clone();
    for (i, &m) in mask.data.iter().enumerate() {
        if m == 0 {
            continue;
        }
        let (x, y) = (
            i as u64 % u64::from(region.width),
            i as u64 / u64::from(region.width),
        );
        // 16 px checker of the tint plus seeded per-pixel noise.
        let checker = ((x / 16 + y / 16) % 2) as u32 * 64;
        let noise = (splitmix(base ^ (i as u64)) & 0x3F) as u32;
        let m = u32::from(m);
        for c in 0..3 {
            let pattern = (tint[c] / 2 + checker + noise).min(255);
            let orig = u32::from(out[i * 3 + c]);
            out[i * 3 + c] = ((orig * (255 - m) + pattern * m + 127) / 255) as u8;
        }
    }
    encode_rgb8(region.width, region.height, &out)
}

fn encode_rgb8(width: u32, height: u32, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    // Writing to a Vec with a matching buffer size can't fail.
    if let Ok(mut writer) = enc.write_header() {
        let _ = writer.write_image_data(data);
        let _ = writer.finish();
    }
    out
}

/// Canonical controls map helper for tests and tools.
pub fn no_controls() -> BTreeMap<String, Value> {
    BTreeMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on_ready as run;

    fn png(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(color);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(data).unwrap();
        w.finish().unwrap();
        out
    }

    /// A 64×64 gradient region and a mask covering the left half.
    fn request(seed: Option<u64>) -> JobRequest {
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
            region_png: png(w, h, png::ColorType::Rgb, &region),
            mask_png: png(w, h, png::ColorType::Grayscale, &mask),
            candidate_count: 3,
            seed,
        }
    }

    fn run_job(p: &FakeProvider, req: &JobRequest) -> Candidates {
        let mut rec = run(p.submit(req)).unwrap();
        loop {
            let st = run(p.status(&rec)).unwrap();
            rec = st.provider;
            if st.state == ProviderJobState::Succeeded {
                break;
            }
        }
        run(p.fetch_candidates(&rec)).unwrap()
    }

    #[test]
    fn fr_prv_03_three_models_one_per_mask_convention() {
        let p = FakeProvider::default();
        let models = run(p.list_models(&ModelFilter::default())).unwrap();
        assert_eq!(models.len(), 3);
        let conventions: HashSet<_> = models
            .iter()
            .map(|m| {
                let caps = run(p.get_capabilities(&m.id)).unwrap();
                format!("{:?}", caps.mask.unwrap().convention)
            })
            .collect();
        assert_eq!(conventions.len(), 3);
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
    fn fr_prv_04_success_case_goes_queued_running_succeeded() {
        let p = FakeProvider::with_case(FakeCase::parse("success").unwrap());
        let mut rec = run(p.submit(&request(None))).unwrap();
        let mut states = vec![];
        for _ in 0..3 {
            let st = run(p.status(&rec)).unwrap();
            states.push(st.state);
            rec = st.provider;
        }
        use ProviderJobState::*;
        assert_eq!(states, vec![Queued, Running, Succeeded]);
        assert!(FakeCase::parse("nope").is_none());
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
        req.mask_png = png(8, 8, png::ColorType::Grayscale, &[0; 64]);
        assert_eq!(run(p.submit(&req)).unwrap_err(), invalid("mask"));
    }

    #[test]
    fn cancel_then_status_is_canceled() {
        let p = FakeProvider::default();
        let rec = run(p.submit(&request(None))).unwrap();
        assert_eq!(run(p.cancel(&rec)).unwrap(), CancelAck::Canceled);
        assert_eq!(
            run(p.status(&rec)).unwrap().state,
            ProviderJobState::Canceled
        );
    }
}

//! Scenario job operations (PRV slice 4): uploads with the ref cache (FR-PRV-14), re-upload
//! (FR-PRV-15), estimate (FR-PRV-16), submit (FR-PRV-17), status (FR-PRV-18), cancel
//! (FR-PRV-20), fetch (FR-PRV-21) and balance (FR-PRV-22).
//!
//! Scenario shapes (research doc): `POST /assets {image: data-url, name, hide}` →
//! `{asset: {id}}`; `POST /uploads {fileName, contentType, kind, fileSize, parts}` →
//! `{upload: {id, parts: [{number, url}]}}`, then `PUT` each part and
//! `POST /uploads/{id}/action {action: "complete", parts}` → `{upload: {status, assetId}}`;
//! `POST /generate/custom/{model}[?dryRun=true]` → 269 `{creativeUnitsCost, costDetails}` or
//! 200 `{job: {jobId, status}}`; `GET /jobs/{id}` → `{job: {status, progress, metadata:
//! {assetIds, error, hint, input}, billing: {cuCost}}}`; `GET /assets/{id}` → `{asset: {url}}`;
//! `GET /teams/{id}/limits`.

use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::api::{
    Body, REQUEST_TIMEOUT, Retry, UPLOAD_PART_TIMEOUT, is_missing_asset, map_error, parse_json, seg,
};
use super::{ScenarioProvider, lock};
use crate::limits;
use crate::mask::convert_mask;
use crate::scenario::record::content_hash;
use crate::transport::Method;
use crate::{
    AssetRef, Balance, CancelAck, CandidateImage, Candidates, Capabilities, ControlRole, Cost,
    CostAmount, Credentials, Estimate, JobRequest, JobStatus, Operation, ProviderError,
    ProviderJobState, ProviderRecord, ProviderResult, UploadPurpose,
};

/// Files below this go to `POST /assets` (base64); the rest use multipart `/uploads`.
pub const SINGLE_UPLOAD_LIMIT: usize = 4 * 1024 * 1024;
/// Multipart part size (Scenario's minimum is 5 MB; the last part may be smaller).
pub const PART_SIZE: usize = 5 * 1024 * 1024;
/// Cost unit (Scenario's Compute Units).
pub const UNIT: &str = "CU";
/// Parallel candidate downloads (spec §12).
const PARALLEL_DOWNLOADS: usize = 4;

fn invalid(field: &str) -> ProviderError {
    ProviderError::InvalidRequest {
        field: field.into(),
    }
}

fn malformed(what: &str) -> ProviderError {
    ProviderError::ProviderFailure {
        hint: format!("the provider sent a malformed {what}"),
    }
}

fn purpose_name(p: UploadPurpose) -> &'static str {
    match p {
        UploadPurpose::Source => "source",
        UploadPurpose::Mask => "mask",
        UploadPurpose::Reference => "reference",
    }
}

/// Polls futures together (≤ 4 candidate downloads at once) without a runtime.
async fn join_all<'a, T>(futs: Vec<Pin<Box<dyn Future<Output = T> + Send + 'a>>>) -> Vec<T> {
    let mut futs: Vec<_> = futs.into_iter().map(Some).collect();
    let mut out: Vec<Option<T>> = futs.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (f, o) in futs.iter_mut().zip(out.iter_mut()) {
            if let Some(fut) = f {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(v) => {
                        *o = Some(v);
                        *f = None;
                    }
                    Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    out.into_iter().flatten().collect()
}

/// One candidate download: the bytes, or the HTTP status of a failed signed-URL fetch.
type Download<'a> = Pin<Box<dyn Future<Output = ProviderResult<Result<Vec<u8>, u16>>> + Send + 'a>>;

/// What a job request turns into before it is sent.
struct Prepared {
    caps: Capabilities,
    image: Vec<u8>,
    mask: Vec<u8>,
}

/// Uploaded refs for one job (Appendix B `asset_refs`).
#[derive(Clone)]
struct Refs {
    image: AssetRef,
    mask: AssetRef,
    hash_image: String,
    hash_mask: String,
}

impl Refs {
    fn json(&self) -> Value {
        json!({
            "image": self.image, "mask": self.mask,
            "hash_image": self.hash_image, "hash_mask": self.hash_mask,
        })
    }
}

/// Never sent, even if a model lists it: it charges even on a dry run (research doc).
const NEVER_SEND: [&str; 1] = ["ipDetection"];

impl ScenarioProvider {
    // ------------------------------------------------------------ uploads (FR-PRV-14)

    /// Uploads one converted file, at most once per content hash per session (R15).
    pub(crate) async fn upload_bytes(
        &self,
        creds: &Credentials,
        bytes: &[u8],
        purpose: UploadPurpose,
        force: bool,
    ) -> ProviderResult<(AssetRef, String)> {
        let hash = content_hash(bytes);
        if !force && let Some(r) = lock(&self.refs).get(&hash).cloned() {
            return Ok((r, hash));
        }
        let name = format!("pocket-forge-{}.png", purpose_name(purpose));
        let id = if bytes.len() < SINGLE_UPLOAD_LIMIT {
            self.upload_single(creds, bytes, &name).await?
        } else {
            self.upload_multipart(creds, bytes, &name).await?
        };
        let r = AssetRef(id);
        lock(&self.refs).insert(hash.clone(), r.clone());
        Ok((r, hash))
    }

    async fn upload_single(
        &self,
        creds: &Credentials,
        bytes: &[u8],
        name: &str,
    ) -> ProviderResult<String> {
        use base64::Engine as _;
        let data = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        let body = json!({"image": data, "name": name, "hide": true});
        let (_, out) = self
            .api
            .json(
                creds,
                Method::Post,
                &self.path("/assets"),
                Some(body),
                Retry::Idempotent,
            )
            .await?;
        out.pointer("/asset/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| malformed("upload response"))
    }

    async fn upload_multipart(
        &self,
        creds: &Credentials,
        bytes: &[u8],
        name: &str,
    ) -> ProviderResult<String> {
        let parts = bytes.len().div_ceil(PART_SIZE).max(1);
        let init = json!({
            "fileName": name, "contentType": "image/png", "kind": "image",
            "fileSize": bytes.len(), "parts": parts,
        });
        let (_, out) = self
            .api
            .json(
                creds,
                Method::Post,
                &self.path("/uploads"),
                Some(init),
                Retry::Idempotent,
            )
            .await?;
        let upload_id = out
            .pointer("/upload/id")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("upload response"))?
            .to_owned();
        let urls = out
            .pointer("/upload/parts")
            .and_then(Value::as_array)
            .filter(|p| p.len() == parts)
            .ok_or_else(|| malformed("upload part list"))?;
        let mut done = vec![];
        for (i, (part, chunk)) in urls.iter().zip(bytes.chunks(PART_SIZE)).enumerate() {
            let url = part
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("upload part url"))?;
            let number = part
                .get("number")
                .and_then(Value::as_u64)
                .unwrap_or(i as u64 + 1);
            // Presigned: never the auth header (FR-PRV-09).
            let resp = self
                .api
                .send(
                    None,
                    Method::Put,
                    url,
                    &Body::Bytes {
                        bytes: chunk.to_vec(),
                        content_type: "image/png".into(),
                    },
                    UPLOAD_PART_TIMEOUT,
                    Retry::Idempotent,
                )
                .await?;
            if !resp.is_success() {
                return Err(map_error(&resp, Some(creds), false));
            }
            done.push(json!({"number": number, "etag": resp.header("etag").unwrap_or("")}));
        }
        let path = self.path(&format!("/uploads/{}/action", seg(&upload_id)));
        let (_, out) = self
            .api
            .json(
                creds,
                Method::Post,
                &path,
                Some(json!({"action": "complete", "parts": done})),
                Retry::Idempotent,
            )
            .await?;
        let mut asset = out
            .pointer("/upload/assetId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        // Completion may still be validating: poll the upload a few times.
        for _ in 0..5 {
            if asset.is_some() {
                break;
            }
            self.api.sleeper.sleep(Duration::from_secs(1)).await;
            let path = self.path(&format!("/uploads/{}", seg(&upload_id)));
            let (_, out) = self
                .api
                .json(creds, Method::Get, &path, None, Retry::Idempotent)
                .await?;
            if out.pointer("/upload/status").and_then(Value::as_str) == Some("failed") {
                return Err(ProviderError::ProviderFailure {
                    hint: "the provider rejected the upload".into(),
                });
            }
            asset = out
                .pointer("/upload/assetId")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        asset.ok_or_else(|| ProviderError::ProviderFailure {
            hint: "the upload did not complete".into(),
        })
    }

    async fn upload_both(
        &self,
        creds: &Credentials,
        p: &Prepared,
        force: bool,
    ) -> ProviderResult<Refs> {
        let (image, hash_image) = self
            .upload_bytes(creds, &p.image, UploadPurpose::Source, force)
            .await?;
        let (mask, hash_mask) = self
            .upload_bytes(creds, &p.mask, UploadPurpose::Mask, force)
            .await?;
        Ok(Refs {
            image,
            mask,
            hash_image,
            hash_mask,
        })
    }

    // ------------------------------------------------------------ requests

    /// Local checks and conversion, before any network call (FR-PRV-12/13).
    async fn prepare(&self, request: &JobRequest) -> ProviderResult<Prepared> {
        if request.operation != Operation::EditMasked {
            return Err(invalid("operation"));
        }
        let caps = self.job_capabilities(&request.model_id).await?;
        let support = match (caps.operations.contains(&Operation::EditMasked), caps.mask) {
            (true, Some(m)) => m,
            _ => return Err(invalid("operation")),
        };
        limits::check(&caps, request)?;
        let values = self.values_for(&caps, request);
        if let Some(e) = caps.evaluate(&values).errors.first() {
            return Err(invalid(&e.control));
        }
        let mask = convert_mask(&request.mask_png, &request.region_png, support)?;
        limits::check_converted(&caps, &request.region_png, "image")?;
        limits::check_converted(&caps, &mask, "mask")?;
        Ok(Prepared {
            caps,
            image: request.region_png.clone(),
            mask,
        })
    }

    /// Control values by name: the caller's controls plus prompt, count and seed by role.
    fn values_for(
        &self,
        caps: &Capabilities,
        request: &JobRequest,
    ) -> std::collections::BTreeMap<String, Value> {
        let mut values = request.controls.clone();
        for c in &caps.controls {
            match c.role {
                ControlRole::Prompt => {
                    values.insert(c.name.clone(), json!(request.final_prompt));
                }
                ControlRole::CandidateCount => {
                    values.insert(c.name.clone(), json!(request.candidate_count));
                }
                ControlRole::Seed => match request.seed {
                    Some(s) => {
                        values.insert(c.name.clone(), json!(s));
                    }
                    None => {
                        values.remove(&c.name);
                    }
                },
                _ => {}
            }
        }
        values
    }

    /// The body, mapped through each control's role and input name (R28, FR-PRV-17).
    fn body(&self, p: &Prepared, request: &JobRequest, refs: Option<&Refs>) -> Value {
        let mut body = Map::new();
        let values = self.values_for(&p.caps, request);
        for c in &p.caps.controls {
            let v = match c.role {
                ControlRole::Source => refs.map(|r| json!(r.image)),
                ControlRole::Mask => refs.map(|r| json!(r.mask)),
                ControlRole::Reference => None,
                _ => values.get(&c.name).cloned(),
            };
            if let Some(v) = v.filter(|_| !NEVER_SEND.contains(&c.name.as_str())) {
                body.insert(c.name.clone(), v);
            }
        }
        Value::Object(body)
    }

    fn record(&self, request: &JobRequest, body: &Value, refs: Option<&Refs>) -> Value {
        json!({
            "v": 1,
            "name": "scenario",
            "model": request.model_id,
            "asset_refs": refs.map_or(json!({}), Refs::json),
            "request": body,
        })
    }

    /// Sends a generate call; on "asset missing" re-uploads once and retries once (FR-PRV-15).
    async fn generate(
        &self,
        creds: &Credentials,
        request: &JobRequest,
        p: &Prepared,
        mut refs: Option<Refs>,
        dry_run: bool,
    ) -> ProviderResult<(u16, Value, Value, Option<Refs>)> {
        let mut path = format!("/generate/custom/{}", seg(&request.model_id));
        if dry_run {
            path.push_str("?dryRun=true");
        }
        let url = format!("{}{}", self.api.base, self.path(&path));
        let retry = if dry_run {
            Retry::Idempotent
        } else {
            Retry::Once
        };
        for attempt in 0..2 {
            let body = self.body(p, request, refs.as_ref());
            let resp = self
                .api
                .send(
                    Some(creds),
                    Method::Post,
                    &url,
                    &Body::Json(body.clone()),
                    REQUEST_TIMEOUT,
                    retry,
                )
                .await?;
            if resp.is_success() {
                return Ok((resp.status, parse_json(&resp.body)?, body, refs));
            }
            if refs.is_some() && is_missing_asset(&resp) {
                if attempt == 0 {
                    tracing::info!("a provider asset is missing; re-uploading once");
                    refs = Some(self.upload_both(creds, p, true).await?);
                    continue;
                }
                let text = String::from_utf8_lossy(&resp.body).to_ascii_lowercase();
                return Err(invalid(if text.contains("mask") {
                    "mask"
                } else {
                    "image"
                }));
            }
            return Err(map_error(&resp, Some(creds), true));
        }
        Err(invalid("image"))
    }

    // ------------------------------------------------------------ operations

    pub(crate) async fn do_upload(
        &self,
        bytes: &[u8],
        purpose: UploadPurpose,
    ) -> ProviderResult<AssetRef> {
        let creds = self.credentials()?;
        Ok(self.upload_bytes(&creds, bytes, purpose, false).await?.0)
    }

    /// `?dryRun=true`, never `ipDetection`. 269 → cost; no cost field → unknown (FR-PRV-16).
    pub(crate) async fn do_estimate(&self, request: &JobRequest) -> ProviderResult<Estimate> {
        let p = self.prepare(request).await?;
        let creds = self.credentials()?;
        let with_assets = self
            .overlay
            .iter()
            .find(|r| r.model_id == request.model_id)
            .is_none_or(|r| r.dry_run_assets);
        let refs = if with_assets {
            Some(self.upload_both(&creds, &p, false).await?)
        } else {
            None
        };
        let (_, out, body, refs) = self.generate(&creds, request, &p, refs, true).await?;
        let amount = out
            .get("creativeUnitsCost")
            .and_then(Value::as_f64)
            .map_or(CostAmount::Unknown, CostAmount::Known);
        let mut rec = self.record(request, &body, refs.as_ref());
        if let Some(d) = out.get("costDetails") {
            rec["cost_raw"] = d.clone();
        }
        Ok(Estimate {
            cost: Cost {
                amount,
                unit: UNIT.into(),
            },
            provider: ProviderRecord(rec),
        })
    }

    /// Never retried once bytes were sent (FR-PRV-17).
    pub(crate) async fn do_submit(&self, request: &JobRequest) -> ProviderResult<ProviderRecord> {
        let p = self.prepare(request).await?;
        let creds = self.credentials()?;
        let refs = self.upload_both(&creds, &p, false).await?;
        let (_, out, body, refs) = self
            .generate(&creds, request, &p, Some(refs), false)
            .await?;
        let job = out.get("job").ok_or_else(|| malformed("job response"))?;
        let handle = job
            .get("jobId")
            .or_else(|| job.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("job response"))?;
        let mut rec = self.record(request, &body, refs.as_ref());
        rec["job_handle"] = json!(handle);
        if let Some(s) = job.get("status") {
            rec["raw_status"] = s.clone();
        }
        if let Some(c) = out.get("creativeUnitsCost") {
            rec["cost_raw"] = json!({"creativeUnitsCost": c});
        }
        Ok(ProviderRecord(rec))
    }

    async fn get_job(&self, creds: &Credentials, job: &ProviderRecord) -> ProviderResult<Value> {
        let handle = job
            .0
            .get("job_handle")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("job_handle"))?;
        let path = self.path(&format!("/jobs/{}", seg(handle)));
        let (_, out) = self
            .api
            .json(creds, Method::Get, &path, None, Retry::Idempotent)
            .await?;
        out.get("job").cloned().ok_or_else(|| malformed("job"))
    }

    /// Appendix C status map; progress clamped; cost from `billing.cuCost` (FR-PRV-18).
    pub(crate) async fn do_status(&self, job: &ProviderRecord) -> ProviderResult<JobStatus> {
        let creds = self.credentials()?;
        let j = match self.get_job(&creds, job).await {
            Ok(j) => j,
            Err(e @ (ProviderError::Offline | ProviderError::Timeout)) => {
                return Ok(JobStatus {
                    state: ProviderJobState::Unknown,
                    progress: 0.0,
                    cost: None,
                    error: Some(e),
                    provider: job.clone(),
                });
            }
            Err(e) => return Err(e),
        };
        let raw = j.get("status").and_then(Value::as_str).unwrap_or("");
        let state = match raw {
            "queued" | "warming-up" | "pending" => ProviderJobState::Queued,
            "in-progress" | "finalizing" => ProviderJobState::Running,
            "success" => ProviderJobState::Succeeded,
            "failure" => ProviderJobState::Failed,
            "canceled" | "cancelled" => ProviderJobState::Canceled,
            _ => ProviderJobState::Unknown,
        };
        let error = (state == ProviderJobState::Failed).then(|| {
            let text = ["/metadata/hint", "/metadata/error", "/error"]
                .iter()
                .find_map(|p| j.pointer(p).and_then(Value::as_str))
                .unwrap_or("the provider reported a failure");
            ProviderError::ProviderFailure {
                hint: super::api::clean_hint(text, Some(&creds)),
            }
        });
        let cost = j
            .pointer("/billing/cuCost")
            .and_then(Value::as_f64)
            .map(|v| Cost {
                amount: CostAmount::Known(v),
                unit: UNIT.into(),
            });
        let mut rec = job.clone();
        rec.0["raw_status"] = json!(raw);
        if let Some(b) = j.get("billing") {
            rec.0["cost_raw"] = b.clone();
        }
        Ok(JobStatus {
            state,
            progress: j
                .get("progress")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0),
            cost,
            error,
            provider: rec,
        })
    }

    /// "Not cancellable" → the ack, not an error (FR-PRV-20).
    pub(crate) async fn do_cancel(&self, job: &ProviderRecord) -> ProviderResult<CancelAck> {
        let creds = self.credentials()?;
        let handle = job
            .0
            .get("job_handle")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("job_handle"))?;
        let url = format!(
            "{}{}",
            self.api.base,
            self.path(&format!("/jobs/{}/action", seg(handle)))
        );
        let resp = self
            .api
            .send(
                Some(&creds),
                Method::Post,
                &url,
                &Body::Json(json!({"action": "cancel"})),
                REQUEST_TIMEOUT,
                Retry::Once,
            )
            .await?;
        if resp.is_success() {
            return Ok(CancelAck::Canceled);
        }
        let text = String::from_utf8_lossy(&resp.body).to_ascii_lowercase();
        if matches!(resp.status, 400 | 409 | 422)
            && (text.contains("cancel") || text.contains("not running"))
        {
            return Ok(CancelAck::NotCancellable);
        }
        Err(map_error(&resp, Some(&creds), true))
    }

    async fn resolve_asset(&self, creds: &Credentials, id: &str) -> ProviderResult<Value> {
        let path = self.path(&format!("/assets/{}", seg(id)));
        let (_, out) = self
            .api
            .json(creds, Method::Get, &path, None, Retry::Idempotent)
            .await?;
        out.get("asset").cloned().ok_or_else(|| malformed("asset"))
    }

    /// Downloads a signed URL (no auth). On 403/404 the caller re-resolves once.
    async fn download(&self, url: &str) -> ProviderResult<Result<Vec<u8>, u16>> {
        let resp = self
            .api
            .send(
                None,
                Method::Get,
                url,
                &Body::None,
                REQUEST_TIMEOUT,
                Retry::Idempotent,
            )
            .await?;
        Ok(match resp.status {
            200..=299 => Ok(resp.body),
            s => Err(s),
        })
    }

    /// Job asset ids → `GET /assets/{id}` → signed URL → bytes, untouched (FR-PRV-21).
    pub(crate) async fn do_fetch(&self, job: &ProviderRecord) -> ProviderResult<Candidates> {
        let creds = self.credentials()?;
        let j = self.get_job(&creds, job).await?;
        let ids: Vec<String> = j
            .pointer("/metadata/assetIds")
            .and_then(Value::as_array)
            .ok_or_else(|| malformed("job asset list"))?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let inputs = j.pointer("/metadata/input").cloned().unwrap_or(Value::Null);
        let mut assets = vec![];
        for id in &ids {
            assets.push(self.resolve_asset(&creds, id).await?);
        }
        let mut images = vec![];
        for chunk in ids
            .iter()
            .zip(&assets)
            .collect::<Vec<_>>()
            .chunks(PARALLEL_DOWNLOADS)
        {
            let futs: Vec<Download<'_>> = chunk
                .iter()
                .map(|(_, a)| {
                    let url = a
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    Box::pin(async move { self.download(&url).await }) as Download<'_>
                })
                .collect();
            for ((id, asset), got) in chunk.iter().zip(join_all(futs).await) {
                let bytes = match got? {
                    Ok(b) => b,
                    Err(403 | 404) => {
                        // The signed URL expired: re-resolve once.
                        let fresh = self.resolve_asset(&creds, id).await?;
                        let url = fresh.get("url").and_then(Value::as_str).unwrap_or("");
                        match self.download(url).await? {
                            Ok(b) => b,
                            Err(s) => {
                                return Err(ProviderError::ProviderFailure {
                                    hint: format!("candidate download failed (HTTP {s})"),
                                });
                            }
                        }
                    }
                    Err(s) => {
                        return Err(ProviderError::ProviderFailure {
                            hint: format!("candidate download failed (HTTP {s})"),
                        });
                    }
                };
                let seed = asset
                    .pointer("/metadata/seed")
                    .or_else(|| inputs.get("seed"))
                    .and_then(|v| {
                        v.as_u64()
                            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                    });
                images.push(CandidateImage {
                    bytes,
                    seed,
                    provider_meta: json!({"asset_id": id, "seed": seed, "inputs": inputs}),
                });
            }
        }
        Ok(Candidates {
            images,
            provider: job.clone(),
        })
    }

    /// Plan remaining + one-shot remaining, plus the period end. No team id, or 401/403 →
    /// `unknown` (FR-PRV-22, OQ-PRV-09).
    pub(crate) async fn do_balance(&self) -> ProviderResult<Balance> {
        let Some(team) = self.account().team_id else {
            return Ok(Balance::Unknown);
        };
        let creds = self.credentials()?;
        let path = self.path(&format!("/teams/{}/limits", seg(&team)));
        let out = match self
            .api
            .json(&creds, Method::Get, &path, None, Retry::Idempotent)
            .await
        {
            Ok((_, out)) => out,
            Err(ProviderError::Auth) => return Ok(Balance::Unknown),
            Err(e) => return Err(e),
        };
        let num = |p: &str| out.pointer(p).and_then(Value::as_f64);
        let plan = match (num("/limits/creativeUnits"), num("/values/creativeUnits")) {
            (Some(limit), Some(used)) => Some((limit - used).max(0.0)),
            (Some(limit), None) => Some(limit),
            _ => None,
        };
        let one_shot: Option<f64> = out
            .get("oneShotCreativeUnits")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.get("remaining").and_then(Value::as_f64))
                    .sum()
            });
        if plan.is_none() && one_shot.is_none() {
            return Ok(Balance::Unknown);
        }
        let period_end = [
            "/periodEnd",
            "/currentPeriodEnd",
            "/limits/periodEnd",
            "/values/periodEnd",
        ]
        .iter()
        .find_map(|p| out.pointer(p).and_then(Value::as_str))
        .map(str::to_owned);
        Ok(Balance::Known {
            remaining: plan.unwrap_or(0.0) + one_shot.unwrap_or(0.0),
            unit: UNIT.into(),
            period_end,
        })
    }
}

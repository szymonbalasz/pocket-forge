//! Scenario job operations (PRV slice 4): uploads with the ref cache (FR-PRV-14), re-upload
//! (FR-PRV-15), estimate (FR-PRV-16), submit (FR-PRV-17), status (FR-PRV-18), cancel
//! (FR-PRV-20), fetch (FR-PRV-21) and balance (FR-PRV-22). References (#51) go through the
//! same uploads and re-upload, into their slots' inputs (FR-PRV-33/34).
//!
//! Scenario shapes (docs.scenario.com, checked 2026-11-08): `POST /assets {image: data-url,
//! name, hide}` → `{asset: {id}}`; `POST /uploads {fileName, contentType, kind, fileSize, parts,
//! assetOptions: {hide}}` →
//! `{upload: {id, parts: [{number, url}]}}`, then `PUT` each part and
//! `POST /uploads/{id}/action {action: "complete"}` → `{upload: {status, entityId, errorMessage}}`
//! (poll `GET /uploads/{id}` while pending/validating; the asset id is `entityId`);
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
use crate::pixels::{ImageFormat, png_size, sniff};
use crate::scenario::record::content_hash;
use crate::transport::Method;
use crate::{
    AssetRef, Balance, CancelAck, CandidateImage, Candidates, Capabilities, ControlRole, Cost,
    CostAmount, Credentials, Estimate, JobRequest, JobStatus, Operation, OutputRequest,
    ProviderError, ProviderJobState, ProviderRecord, ProviderResult, UploadPurpose,
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

/// Upload polling after `complete` (validation can take a while): 1 s, then doubling to 8 s,
/// for at most 60 s of waiting in total.
const UPLOAD_POLL_FIRST: Duration = Duration::from_secs(1);
const UPLOAD_POLL_MAX: Duration = Duration::from_secs(8);
const UPLOAD_POLL_BUDGET: Duration = Duration::from_secs(60);

enum UploadState {
    /// The asset the upload became.
    Done(String),
    /// Still pending or validating; holds the status for the timeout hint.
    Waiting(String),
}

/// Reads an Upload object (`{upload: {status, entityId, errorMessage}}`). The asset id is
/// `entityId` (`assetId` is a fallback for older bodies). `failed` surfaces `errorMessage`;
/// a finished upload without an id is an error rather than a wait.
fn upload_state(out: &Value, creds: &Credentials) -> ProviderResult<UploadState> {
    let u = out
        .get("upload")
        .ok_or_else(|| malformed("upload response"))?;
    let status = u.get("status").and_then(Value::as_str).unwrap_or("unknown");
    if status == "failed" {
        let why = u
            .get("errorMessage")
            .and_then(Value::as_str)
            .map(|m| super::api::clean_hint(m, Some(creds)))
            .unwrap_or_else(|| "no reason given".into());
        return Err(ProviderError::ProviderFailure {
            hint: format!("the provider rejected the upload: {why}"),
        });
    }
    let id = ["entityId", "assetId"]
        .iter()
        .find_map(|k| u.get(*k).and_then(Value::as_str))
        .filter(|id| !id.is_empty());
    match (id, status) {
        (Some(id), _) => Ok(UploadState::Done(id.to_owned())),
        (None, "complete" | "imported") => Err(ProviderError::ProviderFailure {
            hint: format!("the upload finished ({status}) without an asset id"),
        }),
        (None, _) => Ok(UploadState::Waiting(status.to_owned())),
    }
}

/// A submit the provider may have accepted (and billed) without confirming it: after the
/// request was sent, anything but a clear refusal (R102, FR-PRV-17). RGN records it as `lost`
/// with the possible-charge notice and never offers a plain Retry (FR-RGN-23). `timeout`
/// because ADR 0001 fixes the error set and this is "no usable answer".
fn unconfirmed() -> ProviderError {
    ProviderError::Timeout
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
    operation: Operation,
    caps: Capabilities,
    /// Inputs that take a list of files (the source too, for GPT Image and Ideogram 4.5).
    arrays: Vec<String>,
    /// Empty for `generate` (no source, no mask).
    image: Vec<u8>,
    mask: Vec<u8>,
    /// `generate`: the output size asked for (checked against `output_size`).
    output: Option<OutputRequest>,
}

/// One uploaded reference: its slot, the caller's hash and the provider ref.
#[derive(Clone)]
struct ReferenceRef {
    slot: String,
    hash: String,
    r: AssetRef,
}

/// Uploaded source and mask (none for `generate`).
#[derive(Clone)]
struct Inputs {
    image: AssetRef,
    mask: AssetRef,
    hash_image: String,
    hash_mask: String,
}

/// Uploaded refs for one job (Appendix B `asset_refs`).
#[derive(Clone)]
struct Refs {
    inputs: Option<Inputs>,
    /// In request order.
    references: Vec<ReferenceRef>,
}

impl Refs {
    fn json(&self) -> Value {
        let mut v = match &self.inputs {
            Some(i) => json!({
                "image": i.image, "mask": i.mask,
                "hash_image": i.hash_image, "hash_mask": i.hash_mask,
            }),
            None => json!({}),
        };
        if !self.references.is_empty() {
            v["references"] = self
                .references
                .iter()
                .map(|x| json!({"slot": x.slot, "hash": x.hash, "ref": x.r}))
                .collect();
        }
        v
    }

    /// A slot's refs in request order.
    fn for_slot<'a>(&'a self, slot: &'a str) -> impl Iterator<Item = &'a AssetRef> + 'a {
        self.references
            .iter()
            .filter(move |x| x.slot == slot)
            .map(|x| &x.r)
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
        // Sources and masks are PNG; references may be JPEG or WebP (FR-PRV-33).
        let format = sniff(bytes).unwrap_or(ImageFormat::Png);
        let name = format!("pocket-forge-{}.{}", purpose_name(purpose), format.ext());
        let id = if bytes.len() < SINGLE_UPLOAD_LIMIT {
            self.upload_single(creds, bytes, &name, format).await?
        } else {
            self.upload_multipart(creds, bytes, &name, format).await?
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
        format: ImageFormat,
    ) -> ProviderResult<String> {
        use base64::Engine as _;
        let data = format!(
            "data:{};base64,{}",
            format.mime(),
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
        format: ImageFormat,
    ) -> ProviderResult<String> {
        let parts = bytes.len().div_ceil(PART_SIZE).max(1);
        let init = json!({
            "fileName": name, "contentType": format.mime(), "kind": "image",
            "fileSize": bytes.len(), "parts": parts, "assetOptions": {"hide": true},
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
        // Each part URL carries its part number; send chunk N to part N whatever the list order.
        let mut urls = urls
            .iter()
            .enumerate()
            .map(|(i, part)| {
                let url = part
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| malformed("upload part url"))?;
                let number = part
                    .get("number")
                    .and_then(Value::as_u64)
                    .unwrap_or(i as u64 + 1);
                Ok((number, url))
            })
            .collect::<ProviderResult<Vec<_>>>()?;
        urls.sort_by_key(|(n, _)| *n);
        for ((_, url), chunk) in urls.into_iter().zip(bytes.chunks(PART_SIZE)) {
            // Presigned: never the auth header (FR-PRV-09).
            let resp = self
                .api
                .send(
                    None,
                    Method::Put,
                    url,
                    &Body::Bytes {
                        bytes: chunk.to_vec(),
                        content_type: format.mime().into(),
                    },
                    UPLOAD_PART_TIMEOUT,
                    Retry::Idempotent,
                )
                .await?;
            if !resp.is_success() {
                return Err(map_error(&resp, Some(creds), false));
            }
        }
        // The documented complete body is only the action; Scenario lists the parts itself.
        let path = self.path(&format!("/uploads/{}/action", seg(&upload_id)));
        let (_, out) = self
            .api
            .json(
                creds,
                Method::Post,
                &path,
                Some(json!({"action": "complete"})),
                Retry::Idempotent,
            )
            .await?;
        // Validation runs after completion: poll the upload until it names its asset.
        let mut waited = Duration::ZERO;
        let mut delay = UPLOAD_POLL_FIRST;
        let mut out = out;
        loop {
            match upload_state(&out, creds)? {
                UploadState::Done(id) => return Ok(id),
                UploadState::Waiting(status) => {
                    if waited >= UPLOAD_POLL_BUDGET {
                        return Err(ProviderError::ProviderFailure {
                            hint: format!(
                                "the upload did not complete within {} s (last status: {status})",
                                UPLOAD_POLL_BUDGET.as_secs()
                            ),
                        });
                    }
                }
            }
            self.api.sleeper.sleep(delay).await;
            waited += delay;
            delay = (delay * 2).min(UPLOAD_POLL_MAX);
            let path = self.path(&format!("/uploads/{}", seg(&upload_id)));
            out = self
                .api
                .json(creds, Method::Get, &path, None, Retry::Idempotent)
                .await?
                .1;
        }
    }

    /// Uploads the image, the mask (not for `generate`) and every reference (once per content
    /// hash, FR-PRV-34).
    async fn upload_all(
        &self,
        creds: &Credentials,
        p: &Prepared,
        request: &JobRequest,
        force: bool,
    ) -> ProviderResult<Refs> {
        let inputs = if p.operation == Operation::Generate {
            None
        } else {
            let (image, hash_image) = self
                .upload_bytes(creds, &p.image, UploadPurpose::Source, force)
                .await?;
            let (mask, hash_mask) = self
                .upload_bytes(creds, &p.mask, UploadPurpose::Mask, force)
                .await?;
            Some(Inputs {
                image,
                mask,
                hash_image,
                hash_mask,
            })
        };
        let mut references = Vec::with_capacity(request.references.len());
        let mut forced = std::collections::HashSet::new();
        for x in &request.references {
            // A forced re-upload still sends identical bytes only once.
            let again = force && forced.insert(content_hash(&x.bytes));
            let (r, _) = self
                .upload_bytes(creds, &x.bytes, UploadPurpose::Reference, again)
                .await?;
            references.push(ReferenceRef {
                slot: x.slot.clone(),
                hash: x.hash.clone(),
                r,
            });
        }
        Ok(Refs { inputs, references })
    }

    // ------------------------------------------------------------ requests

    /// Local checks and conversion, before any network call (FR-PRV-12/13).
    async fn prepare(&self, request: &JobRequest) -> ProviderResult<Prepared> {
        match request.operation {
            Operation::EditMasked => self.prepare_edit(request).await,
            Operation::Generate => self.prepare_generate(request).await,
            _ => Err(invalid("operation")),
        }
    }

    /// `generate` (GEN OQ-GEN-01): no source, no mask; the output size must match the model's
    /// `output_size`; references are backstopped as for edits (FR-PRV-33).
    async fn prepare_generate(&self, request: &JobRequest) -> ProviderResult<Prepared> {
        let caps = self
            .job_capabilities(&request.model_id, Operation::Generate)
            .await?;
        if !caps.operations.contains(&Operation::Generate) {
            return Err(invalid("operation"));
        }
        limits::check_output(&caps, request)?;
        limits::check_references(&caps, request)?;
        let values = self.values_for(&caps, request);
        if let Some(e) = caps.evaluate(&values).errors.first() {
            return Err(invalid(&e.control));
        }
        let arrays = lock(&self.arrays)
            .get(&request.model_id)
            .cloned()
            .unwrap_or_default();
        Ok(Prepared {
            operation: Operation::Generate,
            caps,
            arrays,
            image: vec![],
            mask: vec![],
            output: request.output.clone(),
        })
    }

    async fn prepare_edit(&self, request: &JobRequest) -> ProviderResult<Prepared> {
        let caps = self
            .job_capabilities(&request.model_id, Operation::EditMasked)
            .await?;
        let support = match (caps.operations.contains(&Operation::EditMasked), caps.mask) {
            (true, Some(m)) => m,
            _ => return Err(invalid("operation")),
        };
        limits::check(&caps, request)?;
        limits::check_references(&caps, request)?;
        let values = self.values_for(&caps, request);
        if let Some(e) = caps.evaluate(&values).errors.first() {
            return Err(invalid(&e.control));
        }
        let mask = convert_mask(&request.mask_png, &request.region_png, support)?;
        limits::check_converted(&caps, &request.region_png, "image")?;
        limits::check_converted(&caps, &mask, "mask")?;
        let arrays = lock(&self.arrays)
            .get(&request.model_id)
            .cloned()
            .unwrap_or_default();
        Ok(Prepared {
            operation: Operation::EditMasked,
            caps,
            arrays,
            image: request.region_png.clone(),
            mask,
            output: None,
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

    /// The body, mapped through each control's role and input name (R28, FR-PRV-17). A
    /// reference slot gets its refs in request order: an array for `file_array`, one ref for
    /// `file`; a slot with none is omitted (FR-PRV-33).
    fn body(&self, p: &Prepared, request: &JobRequest, refs: Option<&Refs>) -> Value {
        let mut body = Map::new();
        let values = self.values_for(&p.caps, request);
        for c in &p.caps.controls {
            let v = match c.role {
                // A list input (GPT Image's `referenceImages`) gets the source as its first item.
                ControlRole::Source if p.arrays.contains(&c.name) => refs
                    .and_then(|r| r.inputs.as_ref())
                    .map(|i| json!([i.image])),
                ControlRole::Source => refs.and_then(|r| r.inputs.as_ref()).map(|i| json!(i.image)),
                ControlRole::Mask => refs.and_then(|r| r.inputs.as_ref()).map(|i| json!(i.mask)),
                ControlRole::Reference => refs.and_then(|r| {
                    let sent: Vec<&AssetRef> = r.for_slot(&c.name).collect();
                    let array = p.caps.slots.iter().any(|s| s.name == c.name && s.array);
                    match (sent.first(), array) {
                        (None, _) => None,
                        (Some(_), true) => Some(json!(sent)),
                        (Some(one), false) => Some(json!(one)),
                    }
                }),
                _ => values.get(&c.name).cloned(),
            };
            if let Some(v) = v.filter(|_| !NEVER_SEND.contains(&c.name.as_str())) {
                body.insert(c.name.clone(), v);
            }
        }
        // Inputs the overlay fills: fixed values, and the sent size (edit) or the output size
        // asked for (generate). Never controls.
        if let Some(row) = self.row(&request.model_id, p.operation) {
            for (name, value) in row.fixed {
                body.insert((*name).to_owned(), json!(value));
            }
            let size = match &p.output {
                Some(OutputRequest::Wh { w, h }) => Some((*w, *h)),
                Some(OutputRequest::Aspect { .. }) => None,
                None => png_size(&p.image),
            };
            if let (Some((w, h)), Some((sw, sh))) = (row.size_inputs, size) {
                body.insert(w.to_owned(), json!(sw));
                body.insert(h.to_owned(), json!(sh));
            }
            // A named-size input (`imageSize`) gets the ratio's name (FR-PRV-37).
            if let (Some(a), Some(OutputRequest::Aspect { aspect })) = (row.aspect_input, &p.output)
                && let Some(v) = row.aspect_value(aspect)
            {
                body.insert(a.to_owned(), json!(v));
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
    /// A real (not dry-run) call that may have created the job without saying so (5xx, a 2xx
    /// that can't be read, a broken connection) is `timeout` (R102, see [`unconfirmed`]).
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
                let out = match parse_json(&resp.body) {
                    Err(_) if !dry_run => return Err(unconfirmed()),
                    other => other?,
                };
                return Ok((resp.status, out, body, refs));
            }
            if !dry_run && resp.status >= 500 {
                return Err(unconfirmed());
            }
            if refs.is_some() && is_missing_asset(&resp) {
                if attempt == 0 {
                    tracing::info!("a provider asset is missing; re-uploading once");
                    refs = Some(self.upload_all(creds, p, request, true).await?);
                    continue;
                }
                let text = String::from_utf8_lossy(&resp.body).to_ascii_lowercase();
                return Err(invalid(if p.operation == Operation::Generate {
                    "reference"
                } else if text.contains("mask") {
                    "mask"
                } else if text.contains("reference") {
                    "reference"
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
            .row(&request.model_id, p.operation)
            .is_none_or(|r| r.dry_run_assets);
        let refs = if with_assets {
            Some(self.upload_all(&creds, &p, request, false).await?)
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
        let refs = self.upload_all(&creds, &p, request, false).await?;
        let (_, out, body, refs) = self
            .generate(&creds, request, &p, Some(refs), false)
            .await?;
        // Accepted (2xx) but no handle to follow: the job may exist and be billed (R102).
        let handle = out
            .get("job")
            .and_then(|job| job.get("jobId").or_else(|| job.get("id")))
            .and_then(Value::as_str)
            .ok_or_else(unconfirmed)?;
        let job = &out["job"];
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

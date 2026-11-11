//! The Scenario implementation (ADR 0001 over REST, research doc). Scenario shapes stay in this
//! module (`pub(crate)`); only [`ScenarioProvider`] and its config are public (FR-PRV-01).

pub(crate) mod api;
pub(crate) mod cache;
pub(crate) mod jobs;
pub(crate) mod normalise;
pub(crate) mod overlay;
pub mod record;

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod generate_tests;
#[cfg(test)]
mod reference_tests;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

pub use self::api::DEFAULT_BASE_URL;
use self::api::{Api, Retry, seg};
use self::cache::{CacheFile, ModelCache};
use self::overlay::{OverlayRow, SCENARIO_ROWS, row_for, unique_ids};
pub use self::overlay::{scenario_model_ids, scenario_model_ids_for};
use crate::credentials::CredentialStore;
use crate::replay::Recording;
use crate::transport::{HttpTransport, Method, RuntimeSleeper, Sleeper, Transport};
use crate::{
    AssetRef, Balance, CancelAck, Candidates, Capabilities, Connectivity, ConnectivityCell,
    Credentials, Estimate, JobRequest, JobStatus, ModelFilter, ModelList, ModelSummary, Operation,
    Provider, ProviderAccount, ProviderError, ProviderRecord, ProviderResult, UploadPurpose,
};

/// How a [`ScenarioProvider`] is wired.
pub struct ScenarioConfig {
    pub base_url: String,
    pub transport: Arc<dyn Transport>,
    pub sleeper: Arc<dyn Sleeper>,
    pub credentials: Arc<dyn CredentialStore>,
    /// `<data-dir>/cache/provider/`; `None` = no disk cache.
    pub cache_dir: Option<PathBuf>,
}

impl ScenarioConfig {
    /// The real wiring: Scenario's API over HTTPS with real sleeps.
    pub fn real(credentials: Arc<dyn CredentialStore>, cache_dir: Option<PathBuf>) -> Self {
        ScenarioConfig {
            base_url: api::DEFAULT_BASE_URL.into(),
            transport: Arc::new(HttpTransport::default()),
            sleeper: Arc::new(RuntimeSleeper),
            credentials,
            cache_dir,
        }
    }
}

/// The Scenario provider.
pub struct ScenarioProvider {
    pub(crate) api: Api,
    store: Arc<dyn CredentialStore>,
    creds: Mutex<Option<Credentials>>,
    account: Mutex<ProviderAccount>,
    cache: ModelCache,
    /// Capabilities fetched this session, per model and operation (estimate/submit read these
    /// first).
    caps: Mutex<HashMap<(String, Operation), Capabilities>>,
    /// Per model: the inputs that take a list of files (from the same fetch as `caps`).
    pub(crate) arrays: Mutex<HashMap<String, Vec<String>>>,
    /// Upload ref cache: content hash → asset ref, per session (R15).
    pub(crate) refs: Mutex<HashMap<String, AssetRef>>,
    overlay: Vec<OverlayRow>,
    pub(crate) connectivity: Arc<ConnectivityCell>,
}

impl std::fmt::Debug for ScenarioProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScenarioProvider")
            .field("base", &self.api.base)
            .field("cache", &self.cache.path())
            .finish_non_exhaustive()
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

macro_rules! fixture_table {
    ($($model:literal / $case:literal),* $(,)?) => {
        &[$(($model, $case, include_str!(concat!("../../../../fixtures/provider/", $model, "/", $case, ".json")))),*]
    };
}

/// Committed fixtures the app itself uses: the fake models' capabilities (FR-PRV-03).
const APP_FIXTURES: &[(&str, &str, &str)] = fixture_table![
    "fake-white-edit" / "get-model",
    "fake-black-edit" / "get-model",
    "fake-alpha-edit" / "get-model",
    "fake-gen-wh" / "get-model",
    "fake-gen-aspect" / "get-model",
    "fake-gen-refs" / "get-model",
];

#[cfg(test)]
const TEST_FIXTURES: &[(&str, &str, &str)] = fixture_table![
    "model_openai-gpt-image-2-5-flare" / "get-model",
    "model_openai-gpt-image-2-5-sunburst" / "get-model",
    "model_ideogram-v4-5" / "get-model",
    "model_ideogram-v4-5-precise-edit" / "get-model",
    "model_ideogram-v3-balanced" / "get-model",
    "model_ideogram-v3-quality" / "get-model",
    "model_ideogram-v3-turbo" / "get-model",
    "model_ideogram-character" / "get-model",
    "model_openai-gpt-image-2-6-flare" / "get-model",
    "model_openai-gpt-image-2-6-sunburst" / "get-model",
    "model_bfl-flux-1-pro-fill" / "get-model",
    "model_ideogram-v4-tiling" / "get-model",
    "model_ideogram-v4" / "get-model",
    "model_bfl-flux-2-pro-editing" / "get-model",
    "model_bfl-flux-2-max-editing" / "get-model",
    "model_bfl-flux-3-image" / "get-model",
    "model_google-gemini-nano-banana-2-1" / "get-model",
    "model_bytedance-seedream-5-0-lite" / "get-model",
    "model_krea-3" / "get-model",
    "model_recraft-v4-1" / "get-model",
    "model_ideogram-v3-balanced" / "contract",
    "model_ideogram-v3-balanced" / "dry-run-no-assets",
    "model_ideogram-v3-balanced" / "upload-multipart",
    "model_ideogram-v3-balanced" / "reupload",
    "model_ideogram-v3-balanced" / "fetch-expired-url",
    "model_ideogram-v3-balanced" / "cancel",
    "model_ideogram-character" / "references",
];

/// A committed recording by model and case.
pub(crate) fn fixture_recording(model: &str, case: &str) -> Option<Recording> {
    #[cfg(test)]
    let tables: [&[(&str, &str, &str)]; 2] = [APP_FIXTURES, TEST_FIXTURES];
    #[cfg(not(test))]
    let tables: [&[(&str, &str, &str)]; 1] = [APP_FIXTURES];
    tables
        .iter()
        .flat_map(|t| t.iter())
        .find(|(m, c, _)| *m == model && *c == case)
        .and_then(|(_, _, json)| Recording::parse(json).ok())
}

impl ScenarioProvider {
    pub fn new(config: ScenarioConfig) -> Self {
        Self::with_overlay(config, SCENARIO_ROWS.to_vec())
    }

    pub(crate) fn with_overlay(config: ScenarioConfig, overlay: Vec<OverlayRow>) -> Self {
        let connectivity = Arc::new(ConnectivityCell::default());
        ScenarioProvider {
            api: Api::new(&config.base_url, config.transport, config.sleeper)
                .with_connectivity(connectivity.clone()),
            store: config.credentials,
            creds: Mutex::new(None),
            account: Mutex::new(ProviderAccount::default()),
            cache: config
                .cache_dir
                .as_deref()
                .map_or_else(ModelCache::none, ModelCache::in_dir),
            caps: Mutex::new(HashMap::new()),
            arrays: Mutex::new(HashMap::new()),
            refs: Mutex::new(HashMap::new()),
            overlay,
            connectivity,
        }
    }

    pub fn connectivity(&self) -> Connectivity {
        self.connectivity.get()
    }

    /// After a credential change: forget the cached credentials and upload refs (R15).
    pub fn credentials_changed(&self) {
        *lock(&self.creds) = None;
        lock(&self.refs).clear();
        self.connectivity.set(Connectivity::Unknown);
    }

    pub fn set_account(&self, account: ProviderAccount) {
        *lock(&self.account) = account;
    }

    pub(crate) fn account(&self) -> ProviderAccount {
        lock(&self.account).clone()
    }

    /// The stored credentials. None stored (or the store is unavailable) → `auth`.
    pub(crate) fn credentials(&self) -> ProviderResult<Credentials> {
        if let Some(c) = lock(&self.creds).clone() {
            return Ok(c);
        }
        match self.store.load() {
            Ok(Some(c)) => {
                *lock(&self.creds) = Some(c.clone());
                Ok(c)
            }
            Ok(None) | Err(_) => Err(ProviderError::Auth),
        }
    }

    /// An API path with the optional `projectId` query (FR-PRV-29).
    pub(crate) fn path(&self, path: &str) -> String {
        match self.account().project_id {
            Some(p) => {
                let sep = if path.contains('?') { '&' } else { '?' };
                format!("{path}{sep}projectId={}", seg(&p))
            }
            None => path.to_owned(),
        }
    }

    pub(crate) fn row(&self, model_id: &str, operation: Operation) -> Option<&OverlayRow> {
        row_for(&self.overlay, model_id, operation)
    }

    /// `GET /models/{id}` → normalised once per overlay row of the model (one per operation).
    /// 404 → `None`.
    async fn fetch_model(
        &self,
        creds: &Credentials,
        model_id: &str,
    ) -> ProviderResult<Option<(String, Vec<(Operation, Capabilities)>)>> {
        let path = self.path(&format!("/models/{}", seg(model_id)));
        let body = match self
            .api
            .json(creds, Method::Get, &path, None, Retry::Idempotent)
            .await
        {
            Ok((_, body)) => body,
            Err(ProviderError::InvalidRequest { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        let rows: Vec<&OverlayRow> = self
            .overlay
            .iter()
            .filter(|r| r.model_id == model_id)
            .collect();
        let mut name = model_id.to_owned();
        let mut all = vec![];
        // No row: normalised once as an edit model (not offered, with the warning).
        let ops: Vec<Option<&OverlayRow>> = if rows.is_empty() {
            vec![None]
        } else {
            rows.into_iter().map(Some).collect()
        };
        for row in ops {
            let op = row.map_or(Operation::EditMasked, |r| r.operation);
            let n = normalise::normalise(model_id, &body, row)?;
            lock(&self.caps).insert((model_id.to_owned(), op), n.capabilities.clone());
            lock(&self.arrays).insert(model_id.to_owned(), n.array_inputs.clone());
            name = n.name;
            all.push((op, n.capabilities));
        }
        Ok(Some((name, all)))
    }

    /// The models offering the filtered operation (FR-PRV-25; `generate`: GEN OQ-GEN-01), from
    /// the network, or the cache when offline. Each entry lists every operation it offers.
    pub async fn list_models_cached(&self, filter: &ModelFilter) -> ProviderResult<ModelList> {
        // An operation no row offers (upscale, …): nothing to fetch.
        if let Some(op) = filter.operation
            && !self.overlay.iter().any(|r| r.operation == op)
        {
            return Ok(ModelList {
                models: vec![],
                stale: false,
            });
        }
        let creds = self.credentials()?;
        let mut file = CacheFile::default();
        let mut failure = None;
        for model_id in unique_ids(&self.overlay) {
            match self.fetch_model(&creds, model_id).await {
                Ok(Some((name, all))) => {
                    let mut operations: Vec<Operation> =
                        all.iter().flat_map(|(_, c)| c.operations.clone()).collect();
                    operations.sort();
                    operations.dedup();
                    if !operations.is_empty() {
                        file.models.push(ModelSummary {
                            id: model_id.into(),
                            name,
                            operations,
                        });
                    }
                    for (op, caps) in all {
                        file.capabilities.insert(cache::key(model_id, op), caps);
                    }
                }
                Ok(None) => {}
                Err(ProviderError::ProviderFailure { hint }) => {
                    tracing::warn!(model = model_id, %hint, "model skipped");
                }
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            }
        }
        let offers =
            |m: &ModelSummary| filter.operation.is_none_or(|op| m.operations.contains(&op));
        match failure {
            None => {
                self.cache.store(&file);
                Ok(ModelList {
                    models: file.models.into_iter().filter(offers).collect(),
                    stale: false,
                })
            }
            // A cached operation no row has any more (removed from the overlay, e.g. GPT Image
            // 2.5's masked edit) is never offered; a cached model left with none is dropped.
            Some(ProviderError::Offline | ProviderError::Timeout) => match self.cache.load() {
                Some(cached) => Ok(ModelList {
                    models: cached
                        .models
                        .into_iter()
                        .filter_map(|mut m| {
                            m.operations.retain(|op| self.has_row(&m.id, *op));
                            (!m.operations.is_empty()).then_some(m)
                        })
                        .filter(offers)
                        .collect(),
                    stale: true,
                }),
                None => Err(ProviderError::Offline),
            },
            Some(e) => Err(e),
        }
    }

    /// Whether the overlay has a row for this model and operation.
    fn has_row(&self, model_id: &str, operation: Operation) -> bool {
        self.overlay
            .iter()
            .any(|r| r.model_id == model_id && r.operation == operation)
    }

    /// Capabilities for one operation, fresh from the network (the cache when offline). A
    /// model with no row at all (never offered, or removed from the overlay) →
    /// `invalid_request(model_id)`; a model with no row for that operation (e.g. GPT Image's
    /// masked edit, removed 2026-11-11) → `invalid_request(operation)`. Both with no network
    /// call and no cache read, so a cached copy never brings the operation back.
    pub async fn fetch_capabilities(
        &self,
        model_id: &str,
        operation: Operation,
    ) -> ProviderResult<Capabilities> {
        if !self.overlay.iter().any(|r| r.model_id == model_id) {
            return Err(ProviderError::InvalidRequest {
                field: "model_id".into(),
            });
        }
        if !self.has_row(model_id, operation) {
            return Err(ProviderError::InvalidRequest {
                field: "operation".into(),
            });
        }
        let creds = self.credentials()?;
        match self.fetch_model(&creds, model_id).await {
            Ok(Some((_, all))) => {
                for (op, caps) in &all {
                    self.cache
                        .put_capabilities(&cache::key(model_id, *op), caps);
                }
                let mine = all
                    .into_iter()
                    .find(|(op, _)| *op == operation)
                    .map(|(_, c)| c);
                mine.ok_or_else(|| ProviderError::InvalidRequest {
                    field: "operation".into(),
                })
            }
            Ok(None) => Err(ProviderError::InvalidRequest {
                field: "model_id".into(),
            }),
            Err(e @ (ProviderError::Offline | ProviderError::Timeout)) => self
                .cache
                .load()
                .and_then(|f| {
                    f.capabilities
                        .get(&cache::key(model_id, operation))
                        .cloned()
                })
                .ok_or(e),
            Err(e) => Err(e),
        }
    }

    /// Capabilities for a job: this session's copy, else fetched (estimate/submit).
    pub(crate) async fn job_capabilities(
        &self,
        model_id: &str,
        operation: Operation,
    ) -> ProviderResult<Capabilities> {
        let key = (model_id.to_owned(), operation);
        if let Some(c) = lock(&self.caps).get(&key).cloned() {
            return Ok(c);
        }
        self.fetch_capabilities(model_id, operation).await
    }

    /// One free call (`GET /models?pageSize=1`) that proves the credentials work.
    pub async fn test_connection(&self) -> ProviderResult<()> {
        let creds = self.credentials()?;
        let path = self.path("/models?pageSize=1");
        self.api
            .json(&creds, Method::Get, &path, None, Retry::Idempotent)
            .await
            .map(|_: (u16, Value)| ())
    }
}

impl Provider for ScenarioProvider {
    async fn list_models(&self, filter: &ModelFilter) -> ProviderResult<Vec<ModelSummary>> {
        self.list_models_cached(filter).await.map(|l| l.models)
    }

    async fn get_capabilities(
        &self,
        model_id: &str,
        operation: Operation,
    ) -> ProviderResult<Capabilities> {
        self.fetch_capabilities(model_id, operation).await
    }

    async fn upload(&self, bytes: &[u8], purpose: UploadPurpose) -> ProviderResult<AssetRef> {
        self.do_upload(bytes, purpose).await
    }

    async fn estimate(&self, request: &JobRequest) -> ProviderResult<Estimate> {
        self.do_estimate(request).await
    }

    async fn submit(&self, request: &JobRequest) -> ProviderResult<ProviderRecord> {
        self.do_submit(request).await
    }

    async fn status(&self, job: &ProviderRecord) -> ProviderResult<JobStatus> {
        self.do_status(job).await
    }

    async fn cancel(&self, job: &ProviderRecord) -> ProviderResult<CancelAck> {
        self.do_cancel(job).await
    }

    async fn fetch_candidates(&self, job: &ProviderRecord) -> ProviderResult<Candidates> {
        self.do_fetch(job).await
    }

    async fn balance(&self) -> ProviderResult<Balance> {
        self.do_balance().await
    }
}

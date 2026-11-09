//! The provider adapter (ADR 0001, [provider spec](../../../docs/specs/provider.md)).
//!
//! - [`Provider`] has exactly the nine ADR 0001 operations with generic inputs and outputs
//!   (FR-PRV-01). Nothing here exposes Scenario concepts.
//! - [`AnyProvider`] is the enum the core dispatches through (OQ-PRV-14).
//! - [`fake::FakeProvider`] is the deterministic stand-in for dev and tests. It opens no network
//!   connection and never spends credits (AGENTS §3.7, FR-PRV-05).
//! - [`poll_delay`] / [`long_running`] are the pure polling schedule; the job runner (RGN) owns
//!   the loop (FR-PRV-19, R14).
//!
//! This crate must not depend on `pf-core` or Tauri (AGENTS §9 dependency direction).

pub mod credentials;
mod error;
mod evaluate;
pub mod fake;
pub mod fixtures;
mod limits;
pub mod mask;
mod pixels;
mod poll;
pub mod real_call;
pub mod replay;
pub mod scenario;
mod secret;
pub mod transport;
mod types;

use std::future::Future;

pub use credentials::{CredentialStore, KeyringStore, MemoryStore, StoreUnavailable};
pub use error::ProviderError;
pub use evaluate::{ControlError, ControlErrorCode, EffectiveControl, Evaluation};
pub use poll::{long_running, poll_delay};
pub use scenario::{ScenarioConfig, ScenarioProvider};
pub use secret::{Credentials, Secret};
pub use types::*;

/// Result of every provider operation.
pub type ProviderResult<T> = Result<T, ProviderError>;

/// The provider-agnostic adapter (ADR 0001 operation table).
///
/// Every operation that concerns a job takes and returns the opaque [`ProviderRecord`]; the
/// caller persists it verbatim on the job record and never reads its fields (FR-PRV-26).
pub trait Provider {
    /// Models offering the filtered operation.
    fn list_models(
        &self,
        filter: &ModelFilter,
    ) -> impl Future<Output = ProviderResult<Vec<ModelSummary>>> + Send;

    /// Normalised model capabilities for one operation: roles, fixed inputs and slots may
    /// differ per operation (GEN OQ-GEN-01).
    fn get_capabilities(
        &self,
        model_id: &str,
        operation: Operation,
    ) -> impl Future<Output = ProviderResult<Capabilities>> + Send;

    /// Uploads image bytes and returns the provider asset ref (a disposable cache, #20).
    fn upload(
        &self,
        bytes: &[u8],
        purpose: UploadPurpose,
    ) -> impl Future<Output = ProviderResult<AssetRef>> + Send;

    /// Pre-flight cost of a job, without running it.
    fn estimate(
        &self,
        request: &JobRequest,
    ) -> impl Future<Output = ProviderResult<Estimate>> + Send;

    /// Submits a job. The returned record holds the job handle.
    fn submit(
        &self,
        request: &JobRequest,
    ) -> impl Future<Output = ProviderResult<ProviderRecord>> + Send;

    /// Current state of a submitted job.
    fn status(
        &self,
        job: &ProviderRecord,
    ) -> impl Future<Output = ProviderResult<JobStatus>> + Send;

    /// Asks the provider to cancel a job.
    fn cancel(
        &self,
        job: &ProviderRecord,
    ) -> impl Future<Output = ProviderResult<CancelAck>> + Send;

    /// Candidate images of a succeeded job, untouched, with provider metadata.
    fn fetch_candidates(
        &self,
        job: &ProviderRecord,
    ) -> impl Future<Output = ProviderResult<Candidates>> + Send;

    /// Remaining provider units, if the provider reports them.
    fn balance(&self) -> impl Future<Output = ProviderResult<Balance>> + Send;
}

/// Which provider implementation runs (FR-PRV-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMode {
    Fake,
    Scenario,
}

impl ProviderMode {
    /// Reads `PF_PROVIDER` (`fake|scenario`). Debug and test builds default to `fake`, release
    /// builds to `scenario` (OQ-PRV-20). An unknown value is an error, never a silent default.
    pub fn from_env_value(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim) {
            None | Some("") => Ok(if cfg!(debug_assertions) {
                ProviderMode::Fake
            } else {
                ProviderMode::Scenario
            }),
            Some("fake") => Ok(ProviderMode::Fake),
            Some("scenario") => Ok(ProviderMode::Scenario),
            Some(other) => Err(format!(
                "PF_PROVIDER must be fake or scenario, not {other:?}"
            )),
        }
    }
}

/// What the app last learned about reaching the provider (the status chip, FR-PRV-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Connectivity {
    /// Nothing called yet since start or the last credential change.
    #[default]
    Unknown,
    Ok,
    Offline,
    AuthError,
}

/// Tracks [`Connectivity`] from operation outcomes.
#[derive(Debug, Default)]
pub struct ConnectivityCell(std::sync::Mutex<Connectivity>);

impl ConnectivityCell {
    pub fn get(&self) -> Connectivity {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set(&self, c: Connectivity) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = c;
    }

    /// Updates from an error: auth → `auth_error`, offline/timeout → `offline`. Other errors
    /// came back from a reachable, authenticated provider → `ok`.
    pub fn note_error(&self, e: &ProviderError) {
        self.set(match e {
            ProviderError::Auth => Connectivity::AuthError,
            ProviderError::Offline | ProviderError::Timeout => Connectivity::Offline,
            _ => Connectivity::Ok,
        });
    }

    /// Updates from an operation result.
    pub fn note<T>(&self, r: &ProviderResult<T>) {
        match r {
            Ok(_) => self.set(Connectivity::Ok),
            Err(e) => self.note_error(e),
        }
    }
}

/// Account settings from the `provider` section of `settings.json` (FR-PRV-29). PRV writes no
/// settings file of its own; the core pushes these in on start and on change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderAccount {
    pub team_id: Option<String>,
    pub project_id: Option<String>,
}

/// The dispatch enum the core holds (OQ-PRV-14).
#[derive(Debug)]
pub enum AnyProvider {
    Fake(fake::FakeProvider),
    Scenario(ScenarioProvider),
}

impl AnyProvider {
    /// The fake provider (default case `success`).
    pub fn fake() -> Self {
        AnyProvider::Fake(fake::FakeProvider::default())
    }

    /// The provider for a mode with the real wiring: Scenario reads credentials from `store`
    /// and caches models under `cache_dir` (`<data-dir>/cache/provider/`); the fake copies its
    /// jobs to `<cache_dir>/fake-jobs/` so they survive a restart (R48). No network call
    /// happens until an operation runs.
    pub fn for_mode(
        mode: ProviderMode,
        store: std::sync::Arc<dyn CredentialStore>,
        cache_dir: Option<std::path::PathBuf>,
    ) -> Self {
        match mode {
            ProviderMode::Fake => match cache_dir {
                Some(dir) => {
                    AnyProvider::Fake(fake::FakeProvider::with_jobs_dir(dir.join("fake-jobs")))
                }
                None => Self::fake(),
            },
            ProviderMode::Scenario => AnyProvider::Scenario(ScenarioProvider::new(
                ScenarioConfig::real(store, cache_dir),
            )),
        }
    }

    pub fn mode(&self) -> ProviderMode {
        match self {
            AnyProvider::Fake(_) => ProviderMode::Fake,
            AnyProvider::Scenario(_) => ProviderMode::Scenario,
        }
    }

    pub fn as_fake(&self) -> Option<&fake::FakeProvider> {
        match self {
            AnyProvider::Fake(p) => Some(p),
            AnyProvider::Scenario(_) => None,
        }
    }

    /// Last known connectivity (FR-PRV-07).
    pub fn connectivity(&self) -> Connectivity {
        match self {
            AnyProvider::Fake(p) => p.connectivity.get(),
            AnyProvider::Scenario(p) => p.connectivity(),
        }
    }

    /// The core calls this after the credentials were set or cleared: the in-memory upload ref
    /// cache is dropped (R15) and connectivity goes back to `unknown`.
    pub fn credentials_changed(&self) {
        match self {
            AnyProvider::Fake(p) => p.connectivity.set(Connectivity::Unknown),
            AnyProvider::Scenario(p) => p.credentials_changed(),
        }
    }

    /// Pushes the `provider` settings section in (FR-PRV-29).
    pub fn set_account(&self, account: ProviderAccount) {
        if let AnyProvider::Scenario(p) = self {
            p.set_account(account);
        }
    }

    /// `list_models`, plus `stale: true` when the cached list was returned offline (FR-PRV-25).
    pub async fn list_models_cached(&self, filter: &ModelFilter) -> ProviderResult<ModelList> {
        match self {
            AnyProvider::Fake(p) => p.list_models(filter).await.map(|models| ModelList {
                models,
                stale: false,
            }),
            AnyProvider::Scenario(p) => p.list_models_cached(filter).await,
        }
    }

    /// One free call that proves the credentials work (`provider_test_connection`).
    pub async fn test_connection(&self) -> ProviderResult<()> {
        match self {
            AnyProvider::Fake(p) => p.list_models(&ModelFilter::default()).await.map(|_| ()),
            AnyProvider::Scenario(p) => p.test_connection().await,
        }
    }
}

/// Each provider tracks its own connectivity (Scenario per HTTP outcome, the fake per case).
macro_rules! dispatch {
    ($self:ident, $p:ident => $call:expr) => {
        match $self {
            AnyProvider::Fake($p) => $call.await,
            AnyProvider::Scenario($p) => $call.await,
        }
    };
}

impl Provider for AnyProvider {
    async fn list_models(&self, filter: &ModelFilter) -> ProviderResult<Vec<ModelSummary>> {
        dispatch!(self, p => p.list_models(filter))
    }

    async fn get_capabilities(
        &self,
        model_id: &str,
        operation: Operation,
    ) -> ProviderResult<Capabilities> {
        dispatch!(self, p => p.get_capabilities(model_id, operation))
    }

    async fn upload(&self, bytes: &[u8], purpose: UploadPurpose) -> ProviderResult<AssetRef> {
        dispatch!(self, p => p.upload(bytes, purpose))
    }

    async fn estimate(&self, request: &JobRequest) -> ProviderResult<Estimate> {
        dispatch!(self, p => p.estimate(request))
    }

    async fn submit(&self, request: &JobRequest) -> ProviderResult<ProviderRecord> {
        dispatch!(self, p => p.submit(request))
    }

    async fn status(&self, job: &ProviderRecord) -> ProviderResult<JobStatus> {
        dispatch!(self, p => p.status(job))
    }

    async fn cancel(&self, job: &ProviderRecord) -> ProviderResult<CancelAck> {
        dispatch!(self, p => p.cancel(job))
    }

    async fn fetch_candidates(&self, job: &ProviderRecord) -> ProviderResult<Candidates> {
        dispatch!(self, p => p.fetch_candidates(job))
    }

    async fn balance(&self) -> ProviderResult<Balance> {
        dispatch!(self, p => p.balance())
    }
}

/// Runs a provider future to completion on the calling thread, parking between polls. Core
/// commands are synchronous and call this; it works on any thread, including inside another
/// async runtime (network work runs on the provider's own runtime).
pub fn block_on<F: Future>(fut: F) -> F::Output {
    struct ThreadWaker(std::thread::Thread);
    impl std::task::Wake for ThreadWaker {
        fn wake(self: std::sync::Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &std::sync::Arc<Self>) {
            self.0.unpark();
        }
    }
    let mut fut = std::pin::pin!(fut);
    let waker = std::task::Waker::from(std::sync::Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    loop {
        if let std::task::Poll::Ready(out) = fut.as_mut().poll(&mut cx) {
            return out;
        }
        std::thread::park();
    }
}

/// Old name of [`block_on`] (the skeleton's fake-only helper).
pub fn block_on_ready<F: Future>(fut: F) -> F::Output {
    block_on(fut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prv_02_mode_defaults_to_fake_in_debug_and_test_builds() {
        assert_eq!(
            ProviderMode::from_env_value(None).unwrap(),
            ProviderMode::Fake
        );
        assert_eq!(
            ProviderMode::from_env_value(Some("scenario")).unwrap(),
            ProviderMode::Scenario
        );
        assert!(ProviderMode::from_env_value(Some("real")).is_err());
    }

    #[test]
    fn fr_prv_03_r48_the_app_fake_copies_jobs_under_the_cache_dir() {
        let store = std::sync::Arc::new(MemoryStore::default());
        let dir = std::path::PathBuf::from("data/cache/provider");
        let p = AnyProvider::for_mode(ProviderMode::Fake, store.clone(), Some(dir.clone()));
        assert_eq!(
            p.as_fake().unwrap().jobs_dir(),
            Some(dir.join("fake-jobs").as_path())
        );
        let p = AnyProvider::for_mode(ProviderMode::Fake, store, None);
        assert_eq!(p.as_fake().unwrap().jobs_dir(), None);
    }

    #[test]
    fn fr_prv_02_scenario_mode_without_credentials_makes_no_call() {
        let store = std::sync::Arc::new(MemoryStore::default());
        let p = AnyProvider::for_mode(ProviderMode::Scenario, store, None);
        assert_eq!(p.mode(), ProviderMode::Scenario);
        assert_eq!(
            block_on(p.test_connection()).unwrap_err(),
            ProviderError::Auth
        );
        assert_eq!(p.connectivity(), Connectivity::Unknown);
        assert!(p.as_fake().is_none());
    }
}

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

mod error;
pub mod fake;
mod poll;
mod types;

use std::future::Future;

pub use error::ProviderError;
pub use poll::{long_running, poll_delay};
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

    /// Normalised model capabilities.
    fn get_capabilities(
        &self,
        model_id: &str,
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

/// The dispatch enum the core holds (OQ-PRV-14).
#[derive(Debug)]
pub enum AnyProvider {
    Fake(fake::FakeProvider),
    /// The mode has no implementation in this build yet (Scenario lands in PRV slices 3–4).
    /// Every operation fails with `provider_failure`; nothing is faked silently.
    Unavailable(ProviderMode),
}

impl AnyProvider {
    /// The provider for a mode.
    pub fn for_mode(mode: ProviderMode) -> Self {
        match mode {
            ProviderMode::Fake => AnyProvider::Fake(fake::FakeProvider::default()),
            ProviderMode::Scenario => AnyProvider::Unavailable(mode),
        }
    }

    pub fn mode(&self) -> ProviderMode {
        match self {
            AnyProvider::Fake(_) => ProviderMode::Fake,
            AnyProvider::Unavailable(mode) => *mode,
        }
    }
}

fn unavailable(mode: ProviderMode) -> ProviderError {
    ProviderError::ProviderFailure {
        hint: format!("the {mode:?} provider is not available in this build"),
    }
}

macro_rules! dispatch {
    ($self:ident, $p:ident => $call:expr) => {
        match $self {
            AnyProvider::Fake($p) => $call.await,
            AnyProvider::Unavailable(mode) => Err(unavailable(*mode)),
        }
    };
}

impl Provider for AnyProvider {
    async fn list_models(&self, filter: &ModelFilter) -> ProviderResult<Vec<ModelSummary>> {
        dispatch!(self, p => p.list_models(filter))
    }

    async fn get_capabilities(&self, model_id: &str) -> ProviderResult<Capabilities> {
        dispatch!(self, p => p.get_capabilities(model_id))
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

/// Runs a future that never waits (the fake provider's) to completion. Test and tooling helper.
pub fn block_on_ready<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(out) = fut.as_mut().poll(&mut cx) {
            return out;
        }
        std::thread::yield_now();
    }
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
    fn fr_prv_02_unavailable_mode_fails_every_operation_without_faking() {
        let p = AnyProvider::for_mode(ProviderMode::Scenario);
        assert_eq!(p.mode(), ProviderMode::Scenario);
        let err = block_on_ready(p.balance()).unwrap_err();
        assert_eq!(err.code(), "provider_failure");
    }
}

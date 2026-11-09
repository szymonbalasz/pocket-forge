//! Provider commands (provider spec §9). Re-exported from [`crate::commands`], which holds the
//! registry. The trait operations, `poll_delay` and `Capabilities::evaluate` are a Rust API for
//! RGN, not commands (R4).
//!
//! Every provider call goes through [`provider_call`], which fires
//! `provider:status-changed {connectivity}` when auth or offline is detected or clears.

use std::future::Future;

use pf_provider::{
    Balance, Capabilities, Connectivity, Credentials, ModelFilter, ModelSummary, Operation,
    Provider, ProviderMode, ProviderResult, StoreUnavailable,
};
use serde::{Deserialize, Serialize};

use crate::commands::{NoInput, Ok_};
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::settings;

/// Event name (R21).
pub const STATUS_CHANGED: &str = "provider:status-changed";

fn store_error(e: StoreUnavailable) -> CoreError {
    CoreError::new(ErrorCode::StoreUnavailable, e.to_string())
}

/// Runs a provider future and emits `provider:status-changed` if connectivity changed.
pub fn provider_call<T>(
    core: &Core,
    fut: impl Future<Output = ProviderResult<T>>,
) -> CoreResult<T> {
    let before = core.ctx.provider.connectivity();
    let out = pf_provider::block_on(fut);
    emit_if_changed(core, before);
    out.map_err(CoreError::from)
}

fn emit_if_changed(core: &Core, before: Connectivity) {
    let after = core.ctx.provider.connectivity();
    if after != before {
        core.ctx
            .emit(STATUS_CHANGED, serde_json::json!({ "connectivity": after }));
    }
}

// ---------------------------------------------------------------- status

pub struct ProviderStatus;

#[derive(Debug, Serialize)]
pub struct ProviderStatusOutput {
    pub mode: ProviderMode,
    /// The fake is always configured; Scenario needs stored credentials (FR-PRV-07).
    pub configured: bool,
    /// Last 4 characters of the stored key id, if any. Never more (FR-PRV-07).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_suffix: Option<String>,
    pub connectivity: Connectivity,
    /// The OS credential store couldn't be read (status shows "not configured").
    pub store_unavailable: bool,
}

impl Command for ProviderStatus {
    const NAME: &'static str = "provider_status";
    type Input = NoInput;
    type Output = ProviderStatusOutput;

    fn run(core: &Core, _: NoInput) -> CoreResult<ProviderStatusOutput> {
        let mode = core.ctx.provider.mode();
        let (stored, store_unavailable) = match core.ctx.credentials.load() {
            Ok(c) => (c, false),
            Err(e) => {
                tracing::warn!(error = %e, "credential store unavailable");
                (None, true)
            }
        };
        Ok(ProviderStatusOutput {
            mode,
            configured: mode == ProviderMode::Fake || stored.is_some(),
            key_suffix: stored.as_ref().map(Credentials::key_suffix),
            connectivity: core.ctx.provider.connectivity(),
            store_unavailable,
        })
    }
}

// ---------------------------------------------------------------- credentials

pub struct ProviderCredentialsSet;

/// `{key, secret}`. `Debug` is redacted so neither value can reach a log.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialsInput {
    pub key: String,
    pub secret: String,
}

impl std::fmt::Debug for CredentialsInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialsInput { <redacted> }")
    }
}

const MAX_CREDENTIAL_CHARS: usize = 256;

fn check_credential(field: &str, v: &str) -> CoreResult<()> {
    let n = v.trim().chars().count();
    if n == 0 || n > MAX_CREDENTIAL_CHARS || v.contains('\0') {
        return Err(CoreError::invalid(
            field,
            format!("the {field} must be 1 to {MAX_CREDENTIAL_CHARS} characters"),
        ));
    }
    Ok(())
}

impl Validate for CredentialsInput {
    fn validate(&self) -> CoreResult<()> {
        check_credential("key", &self.key)?;
        check_credential("secret", &self.secret)
    }
}

impl Command for ProviderCredentialsSet {
    const NAME: &'static str = "provider_credentials_set";
    type Input = CredentialsInput;
    type Output = Ok_;

    fn run(core: &Core, input: CredentialsInput) -> CoreResult<Ok_> {
        let creds = Credentials::new(input.key.trim(), input.secret.trim());
        core.ctx.credentials.save(&creds).map_err(store_error)?;
        let before = core.ctx.provider.connectivity();
        core.ctx.provider.credentials_changed();
        emit_if_changed(core, before);
        Ok(Ok_ {})
    }
}

pub struct ProviderCredentialsClear;

impl Command for ProviderCredentialsClear {
    const NAME: &'static str = "provider_credentials_clear";
    type Input = NoInput;
    type Output = Ok_;

    fn run(core: &Core, _: NoInput) -> CoreResult<Ok_> {
        core.ctx.credentials.clear().map_err(store_error)?;
        let before = core.ctx.provider.connectivity();
        core.ctx.provider.credentials_changed();
        emit_if_changed(core, before);
        Ok(Ok_ {})
    }
}

pub struct ProviderTestConnection;

impl Command for ProviderTestConnection {
    const NAME: &'static str = "provider_test_connection";
    type Input = NoInput;
    type Output = Ok_;

    fn run(core: &Core, _: NoInput) -> CoreResult<Ok_> {
        if core.ctx.provider.mode() == ProviderMode::Scenario
            && core.ctx.credentials.load().map_err(store_error)?.is_none()
        {
            return Err(CoreError::new(
                ErrorCode::NotConfigured,
                "enter a Scenario API key and secret first",
            ));
        }
        provider_call(core, core.ctx.provider.test_connection())?;
        Ok(Ok_ {})
    }
}

// ---------------------------------------------------------------- settings (FR-PRV-29)

pub struct ProviderSettingsGet;

/// `{team_id?, project_id?}`: ≤ 64 chars of `[A-Za-z0-9_-]`; `null` or `""` clears.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSettingsIo {
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
}

fn check_account_id(field: &str, v: &Option<String>) -> CoreResult<()> {
    match v.as_deref().map(str::trim) {
        None | Some("") => Ok(()),
        Some(s)
            if s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') =>
        {
            Ok(())
        }
        Some(_) => Err(CoreError::invalid(
            field,
            "use up to 64 letters, digits, '_' or '-'",
        )),
    }
}

impl Validate for ProviderSettingsIo {
    fn validate(&self) -> CoreResult<()> {
        check_account_id("team_id", &self.team_id)?;
        check_account_id("project_id", &self.project_id)
    }
}

impl Command for ProviderSettingsGet {
    const NAME: &'static str = "provider_settings_get";
    type Input = NoInput;
    type Output = ProviderSettingsIo;

    fn run(core: &Core, _: NoInput) -> CoreResult<ProviderSettingsIo> {
        let state = core.state();
        Ok(ProviderSettingsIo {
            team_id: state.settings.provider.team_id.clone(),
            project_id: state.settings.provider.project_id.clone(),
        })
    }
}

pub struct ProviderSettingsSet;

impl Command for ProviderSettingsSet {
    const NAME: &'static str = "provider_settings_set";
    type Input = ProviderSettingsIo;
    type Output = ProviderSettingsIo;

    fn run(core: &Core, input: ProviderSettingsIo) -> CoreResult<ProviderSettingsIo> {
        let clean = |v: Option<String>| v.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        let mut state = core.state();
        let mut next = state.settings.clone();
        next.provider.team_id = clean(input.team_id);
        next.provider.project_id = clean(input.project_id);
        settings::save(&core.ctx, &next)?;
        core.ctx.provider.set_account(next.provider.account());
        state.settings = next;
        Ok(ProviderSettingsIo {
            team_id: state.settings.provider.team_id.clone(),
            project_id: state.settings.provider.project_id.clone(),
        })
    }
}

// ---------------------------------------------------------------- models

pub struct ProviderListModels;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListModelsInput {
    pub operation: Operation,
}

impl Validate for ListModelsInput {}

#[derive(Debug, Serialize)]
pub struct ListModelsOutput {
    pub models: Vec<ModelSummary>,
    /// Cached data returned while offline (FR-PRV-25).
    pub stale: bool,
}

impl Command for ProviderListModels {
    const NAME: &'static str = "provider_list_models";
    type Input = ListModelsInput;
    type Output = ListModelsOutput;

    fn run(core: &Core, input: ListModelsInput) -> CoreResult<ListModelsOutput> {
        let filter = ModelFilter {
            operation: Some(input.operation),
        };
        let list = provider_call(core, core.ctx.provider.list_models_cached(&filter))?;
        Ok(ListModelsOutput {
            models: list.models,
            stale: list.stale,
        })
    }
}

pub struct ProviderGetCapabilities;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetCapabilitiesInput {
    pub model_id: String,
    /// The operation the capabilities are for (GEN OQ-GEN-01); `edit_masked` when omitted.
    #[serde(default = "edit_masked")]
    pub operation: Operation,
}

fn edit_masked() -> Operation {
    Operation::EditMasked
}

impl Validate for GetCapabilitiesInput {
    fn validate(&self) -> CoreResult<()> {
        let m = self.model_id.trim();
        if m.is_empty() || m.len() > 200 || m.contains(['/', '\\', '\0']) {
            return Err(CoreError::invalid("model_id", "not a valid model id"));
        }
        Ok(())
    }
}

impl Command for ProviderGetCapabilities {
    const NAME: &'static str = "provider_get_capabilities";
    type Input = GetCapabilitiesInput;
    type Output = Capabilities;

    fn run(core: &Core, input: GetCapabilitiesInput) -> CoreResult<Capabilities> {
        provider_call(
            core,
            core.ctx
                .provider
                .get_capabilities(input.model_id.trim(), input.operation),
        )
    }
}

pub struct ProviderBalance;

impl Command for ProviderBalance {
    const NAME: &'static str = "provider_balance";
    type Input = NoInput;
    type Output = Balance;

    /// Never fails: any provider error → `unknown` (spec §9).
    fn run(core: &Core, _: NoInput) -> CoreResult<Balance> {
        Ok(
            provider_call(core, core.ctx.provider.balance()).unwrap_or_else(|e| {
                tracing::info!(code = %e.code, "balance unavailable");
                Balance::Unknown
            }),
        )
    }
}

// ---------------------------------------------------------------- fake

pub struct ProviderFakeSetCase;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FakeSetCaseInput {
    pub case: String,
}

impl Validate for FakeSetCaseInput {
    fn validate(&self) -> CoreResult<()> {
        pf_provider::fake::FakeCase::parse(&self.case)
            .map(|_| ())
            .ok_or_else(|| CoreError::invalid("case", "unknown fake case"))
    }
}

impl Command for ProviderFakeSetCase {
    const NAME: &'static str = "provider_fake_set_case";
    type Input = FakeSetCaseInput;
    type Output = Ok_;

    fn run(core: &Core, input: FakeSetCaseInput) -> CoreResult<Ok_> {
        let fake = core.ctx.provider.as_fake().ok_or_else(|| {
            CoreError::new(
                ErrorCode::NotFakeMode,
                "fake cases only exist in fake provider mode",
            )
        })?;
        // Validated above, so the parse succeeds.
        if let Some(case) = pf_provider::fake::FakeCase::parse(&input.case) {
            fake.set_case(case);
        }
        Ok(Ok_ {})
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use pf_provider::{AnyProvider, CredentialStore, MemoryStore};
    use serde_json::{Value, json};

    use crate::context::{Context, RecordedEvents};
    use crate::time::Timestamp;
    use crate::{Core, ErrorCode};

    fn core_with(
        mode: ProviderMode,
        store: Arc<MemoryStore>,
    ) -> (tempfile::TempDir, Core, Arc<RecordedEvents>) {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(1_800_000_000_000));
        ctx.provider = AnyProvider::for_mode(mode, store.clone(), None);
        ctx.credentials = store;
        let events = Arc::new(RecordedEvents::default());
        ctx.events = events.clone();
        (dir, Core::new(ctx).unwrap(), events)
    }

    use super::*;

    #[test]
    fn fr_prv_07_status_exposes_configured_and_key_suffix_only() {
        let store = Arc::new(MemoryStore::default());
        let (_d, core, _) = core_with(ProviderMode::Scenario, store.clone());
        let st = core.dispatch("provider_status", json!({})).unwrap();
        assert_eq!(st["mode"], "scenario");
        assert_eq!(st["configured"], false);
        assert!(st.get("key_suffix").is_none());

        core.dispatch(
            "provider_credentials_set",
            json!({"key": "  key-id-0000abcd ", "secret": "the-secret-value"}),
        )
        .unwrap();
        let st = core.dispatch("provider_status", json!({})).unwrap();
        assert_eq!(st["configured"], true);
        assert_eq!(st["key_suffix"], "abcd");
        let text = st.to_string();
        assert!(!text.contains("the-secret-value") && !text.contains("key-id-0000"));
        // Trimmed on the way in.
        assert_eq!(
            store.load().unwrap().unwrap().key.expose(),
            "key-id-0000abcd"
        );
    }

    #[test]
    fn fr_prv_06_credentials_validate_clear_idempotently_and_report_store_unavailable() {
        let store = Arc::new(MemoryStore::default());
        let (_d, core, _) = core_with(ProviderMode::Scenario, store.clone());
        let e = core
            .dispatch(
                "provider_credentials_set",
                json!({"key": " ", "secret": "s"}),
            )
            .unwrap_err();
        assert_eq!(
            (e.code, e.field.as_deref()),
            (ErrorCode::InvalidInput, Some("key"))
        );
        let long = "x".repeat(257);
        let e = core
            .dispatch(
                "provider_credentials_set",
                json!({"key": "k", "secret": long}),
            )
            .unwrap_err();
        assert_eq!(e.field.as_deref(), Some("secret"));
        assert!(
            !e.message.contains("xxxx"),
            "error text never echoes the value"
        );

        core.dispatch("provider_credentials_clear", json!({}))
            .unwrap();
        core.dispatch("provider_credentials_clear", json!({}))
            .unwrap();

        store.unavailable.store(true, Ordering::Relaxed);
        let e = core
            .dispatch(
                "provider_credentials_set",
                json!({"key": "k", "secret": "s"}),
            )
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::StoreUnavailable);
        let st = core.dispatch("provider_status", json!({})).unwrap();
        assert_eq!(
            (st["configured"].clone(), st["store_unavailable"].clone()),
            (json!(false), json!(true))
        );
    }

    #[test]
    fn fr_prv_06_nothing_reaches_the_data_dir() {
        let store = Arc::new(MemoryStore::default());
        let (d, core, _) = core_with(ProviderMode::Scenario, store);
        core.dispatch(
            "provider_credentials_set",
            json!({"key": "dummy-key-9876", "secret": "dummy-secret-5432"}),
        )
        .unwrap();
        core.dispatch("provider_settings_set", json!({"team_id": "team_1"}))
            .unwrap();
        for entry in walk(d.path()) {
            let bytes = std::fs::read(&entry).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                !text.contains("dummy-secret-5432") && !text.contains("dummy-key-9876"),
                "{} contains a credential",
                entry.display()
            );
        }
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = vec![];
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }

    #[test]
    fn fr_prv_29_team_and_project_ids_live_in_the_provider_settings_section() {
        let (d, core, _) = core_with(ProviderMode::Fake, Arc::new(MemoryStore::default()));
        let out = core
            .dispatch(
                "provider_settings_set",
                json!({"team_id": " team_ABC-1 ", "project_id": ""}),
            )
            .unwrap();
        assert_eq!(out, json!({"team_id": "team_ABC-1", "project_id": null}));
        let saved: Value =
            serde_json::from_slice(&std::fs::read(d.path().join("data/settings.json")).unwrap())
                .unwrap();
        assert_eq!(saved["provider"], json!({"team_id": "team_ABC-1"}));
        assert_eq!(
            core.dispatch("provider_settings_get", json!({})).unwrap()["team_id"],
            "team_ABC-1"
        );
        let e = core
            .dispatch("provider_settings_set", json!({"team_id": "has space"}))
            .unwrap_err();
        assert_eq!(e.field.as_deref(), Some("team_id"));
        let e = core
            .dispatch(
                "provider_settings_set",
                json!({"project_id": "x".repeat(65)}),
            )
            .unwrap_err();
        assert_eq!(e.field.as_deref(), Some("project_id"));
        // No settings file of PRV's own.
        let names: Vec<_> = std::fs::read_dir(d.path().join("data"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["settings.json"]);
    }

    #[test]
    fn test_connection_needs_credentials_in_scenario_mode() {
        let (_d, core, _) = core_with(ProviderMode::Scenario, Arc::new(MemoryStore::default()));
        let e = core
            .dispatch("provider_test_connection", json!({}))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NotConfigured);
    }

    #[test]
    fn fr_prv_04_fake_set_case_only_in_fake_mode() {
        let (_d, core, _) = core_with(ProviderMode::Fake, Arc::new(MemoryStore::default()));
        core.dispatch("provider_fake_set_case", json!({"case": "success"}))
            .unwrap();
        let e = core
            .dispatch("provider_fake_set_case", json!({"case": "nope"}))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidInput);
        let (_d, core, _) = core_with(ProviderMode::Scenario, Arc::new(MemoryStore::default()));
        let e = core
            .dispatch("provider_fake_set_case", json!({"case": "success"}))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFakeMode);
    }

    #[test]
    fn fake_mode_lists_models_and_reports_balance_and_status_change_event() {
        let (_d, core, events) = core_with(ProviderMode::Fake, Arc::new(MemoryStore::default()));
        let st = core.dispatch("provider_status", json!({})).unwrap();
        assert_eq!(
            (st["configured"].clone(), st["connectivity"].clone()),
            (json!(true), json!("unknown"))
        );
        let list = core
            .dispatch("provider_list_models", json!({"operation": "edit_masked"}))
            .unwrap();
        assert_eq!(list["models"].as_array().unwrap().len(), 3);
        assert_eq!(list["stale"], false);
        assert_eq!(events.names(), [STATUS_CHANGED]);
        let caps = core
            .dispatch(
                "provider_get_capabilities",
                json!({"model_id": "fake-white-edit"}),
            )
            .unwrap();
        assert_eq!(caps["mask"]["convention"], "white_edit");
        let bal = core.dispatch("provider_balance", json!({})).unwrap();
        assert_eq!(bal["kind"], "known");
        core.dispatch("provider_test_connection", json!({}))
            .unwrap();
    }

    /// GEN FR-GEN-05/06 through the core commands: `generate` lists the two fake generate
    /// models, and their capabilities carry `output_size` (no mask); omitting the operation
    /// still means `edit_masked`.
    #[test]
    fn fr_gen_05_list_and_capabilities_take_the_generate_operation() {
        let (_d, core, _events) = core_with(ProviderMode::Fake, Arc::new(MemoryStore::default()));
        let list = core
            .dispatch("provider_list_models", json!({"operation": "generate"}))
            .unwrap();
        let ids: Vec<_> = list["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids, ["fake-gen-wh", "fake-gen-aspect"]);
        let caps = core
            .dispatch(
                "provider_get_capabilities",
                json!({"model_id": "fake-gen-wh", "operation": "generate"}),
            )
            .unwrap();
        assert_eq!(
            caps["output_size"],
            json!({"mode": "wh", "min": 64, "max": 2048, "step": 16})
        );
        assert_eq!(caps["structure"], json!({"slot": "refs"}));
        assert!(caps["mask"].is_null());
        let err = core
            .dispatch(
                "provider_get_capabilities",
                json!({"model_id": "fake-gen-wh"}),
            )
            .unwrap_err();
        assert_eq!(err.detail["provider"]["field"], "model_id");
    }
}

//! FR-PRV-08 log capture (spec §13 integration): a full fake job and a full Scenario replay
//! job, plus every error case, logged at TRACE; the capture contains neither the key, the
//! secret nor an `Authorization` header.
//!
//! Its own test binary with one test and a global subscriber, so concurrent tests can't race
//! tracing's callsite cache and make the capture miss events.

// Helpers outside #[test] fns unwrap too; this file is test-only.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use pf_provider::fake::{FakeCase, FakeProvider};
use pf_provider::fixtures::{disc_mask_png, room_png};
use pf_provider::replay::{
    Exchange, RecordedRequest, RecordedResponse, Recording, ReplayTransport,
};
use pf_provider::transport::{Method, RecordingSleeper};
use pf_provider::{
    Credentials, JobRequest, MemoryStore, ModelFilter, Operation, Provider, ProviderAccount,
    ProviderJobState, ProviderRecord, ScenarioConfig, ScenarioProvider, block_on,
};
use serde_json::{Value, json};

const BASE: &str = pf_provider::scenario::DEFAULT_BASE_URL;
const CONTRACT: &str = include_str!("../../../fixtures/provider/model_ideogram-v3/contract.json");

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn creds() -> Credentials {
    Credentials::new("log-test-key-0000wxyz", "log-test-secret-value")
}

fn scenario(exchanges: Vec<Exchange>) -> ScenarioProvider {
    let p = ScenarioProvider::new(ScenarioConfig {
        base_url: BASE.into(),
        transport: Arc::new(ReplayTransport::from_exchanges(BASE, exchanges)),
        sleeper: Arc::new(RecordingSleeper::default()),
        credentials: Arc::new(MemoryStore::with(&creds())),
        cache_dir: None,
    });
    p.set_account(ProviderAccount {
        team_id: Some("<team-id>".into()),
        project_id: None,
    });
    p
}

fn request(model: &str) -> JobRequest {
    JobRequest {
        operation: Operation::EditMasked,
        model_id: model.into(),
        final_prompt: "a mossy flagstone floor".into(),
        controls: BTreeMap::new(),
        references: vec![],
        region_png: room_png(256),
        mask_png: disc_mask_png(256),
        candidate_count: 2,
        seed: Some(7),
    }
}

fn full_job<P: Provider>(p: &P, model: &str) {
    block_on(p.get_capabilities(model)).unwrap();
    block_on(p.estimate(&request(model))).unwrap();
    let mut rec: ProviderRecord = block_on(p.submit(&request(model))).unwrap();
    for _ in 0..3 {
        let st = block_on(p.status(&rec)).unwrap();
        rec = st.provider;
        if st.state == ProviderJobState::Succeeded {
            break;
        }
    }
    let c = block_on(p.fetch_candidates(&rec)).unwrap();
    tracing::debug!(provider = ?c.provider, "job record");
    block_on(p.balance()).unwrap();
}

fn get(path: &str, status: u16, body: Value) -> Exchange {
    Exchange {
        request: RecordedRequest {
            method: Method::Get,
            path: path.into(),
            query: [("pageSize".to_owned(), "1".to_owned())].into(),
            body: None,
        },
        response: RecordedResponse {
            status,
            headers: BTreeMap::new(),
            body,
        },
    }
}

#[test]
fn fr_prv_08_log_capture_over_a_full_fake_and_replay_job_has_no_credentials() {
    let buf = Captured::default();
    let writer = buf.clone();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .init();

    // A full Scenario replay job (the contract recording minus its list_models call).
    let rec = Recording::parse(CONTRACT).unwrap();
    full_job(&scenario(rec.exchanges[1..].to_vec()), "model_ideogram-v3");

    // A full fake job, then every fake error case.
    let fake = FakeProvider::default();
    block_on(fake.list_models(&ModelFilter::default())).unwrap();
    full_job(&fake, "fake-alpha-edit");
    for case in [
        "auth",
        "insufficient_balance",
        "offline",
        "rate_limited_once",
        "asset_missing_once",
        "failure",
    ] {
        let p = FakeProvider::with_case(FakeCase::parse(case).unwrap());
        let e = block_on(p.submit(&request("fake-alpha-edit")));
        tracing::warn!(?e, "fake {case}");
    }

    // Every error-map row through the real request layer, with bodies that echo headers.
    for status in [401, 402, 429, 400, 404, 422, 500, 503, 0] {
        let body = if status == 0 {
            json!("offline")
        } else {
            json!({"reason": "echo Authorization: Basic eA=="})
        };
        let p = scenario(
            (0..3)
                .map(|_| get("/models", status, body.clone()))
                .collect(),
        );
        let e = p.test_connection();
        let e = block_on(e).unwrap_err();
        tracing::error!(%e, ?e, "scenario error");
    }
    tracing::info!(creds = ?creds(), "credentials debug");

    let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(
        text.contains("provider request"),
        "the capture saw the requests"
    );
    assert!(text.contains("scenario error"));
    let c = creds();
    for leak in [
        c.key.expose(),
        c.secret.expose(),
        c.basic_auth().expose(),
        "Authorization",
        "Basic ",
    ] {
        assert!(!text.contains(leak), "{leak:?} in the log");
    }
}

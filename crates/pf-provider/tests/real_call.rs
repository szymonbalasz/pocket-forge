//! Real Scenario calls (FR-PRV-28). Never run in CI: they need cargo feature `real-call`
//! **and** `PF_REAL_CALL=1`, and they fail if `CI` is set together with either. Without
//! `PF_REAL_CALL=1` each test prints why it skipped and passes.
//!
//! Owner, locally (runbook: `docs/guides/real-provider.md`):
//!
//! ```text
//! PF_REAL_CALL=1 cargo test -p pf-provider --features real-call --test real_call -- --nocapture
//! PF_REAL_CALL=1 PF_REAL_SPEND_CU=20 PF_REAL_MODEL=model_ideogram-v3-balanced \
//!     cargo test -p pf-provider --features real-call --test real_call -- --nocapture --ignored
//! ```
//!
//! The free tests make only free calls (`GET /models`, `GET /models/{id}`, dry runs). The one
//! paid test is `#[ignore]` and also needs `PF_REAL_SPEND_CU` (its spend cap in CU).
#![cfg(feature = "real-call")]

use std::sync::Arc;
use std::time::Duration;

use pf_provider::real_call::{self, Polarity};
use pf_provider::transport::RuntimeSleeper;
use pf_provider::{
    KeyringStore, Operation, ProviderAccount, ProviderJobState, ScenarioConfig, ScenarioProvider,
    block_on,
};

/// The gate. `None` = skip (printed); panics when CI sets the feature or `PF_REAL_CALL`.
fn provider(test: &str) -> Option<ScenarioProvider> {
    match real_call::real_calls_enabled() {
        Err(e) => panic!("{e}"),
        Ok(false) => {
            eprintln!("{test}: skipped (set {}=1 to run)", real_call::ENV_GATE);
            None
        }
        Ok(true) => {
            let store = Arc::new(KeyringStore::for_provider("scenario"));
            let p = ScenarioProvider::new(ScenarioConfig::real(store, None));
            let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
            p.set_account(ProviderAccount {
                team_id: var("PF_SCENARIO_TEAM_ID"),
                project_id: var("PF_SCENARIO_PROJECT_ID"),
            });
            Some(p)
        }
    }
}

/// Free: the key works (`GET /models?pageSize=1`).
#[test]
fn fr_prv_28_real_test_connection() {
    let Some(p) = provider("fr_prv_28_real_test_connection") else {
        return;
    };
    block_on(p.test_connection())
        .unwrap_or_else(|e| panic!("test connection: {}", real_call::error_text(&e)));
}

/// Free (GEN §10): every generate row is found, offers generate (`txt2img`), and its
/// `generate` dry run returns a cost. Prints one line per model.
#[test]
fn fr_gen_05_real_generate_rows_and_dry_runs() {
    let Some(p) = provider("fr_gen_05_real_generate_rows_and_dry_runs") else {
        return;
    };
    let mut bad = vec![];
    for model in pf_provider::scenario::scenario_model_ids_for(Operation::Generate) {
        let c = block_on(real_call::check_model(&p, model, Operation::Generate, true));
        println!("{}", c.summary());
        if !(c.found && c.offered() && matches!(c.estimate, Some(Ok(_)))) {
            bad.push(model);
        }
    }
    assert!(bad.is_empty(), "generate rows to fix: {bad:?}");
}

/// Free: every masked-edit overlay row is found, offers masked edit, and its dry run returns a
/// cost. Prints one line per model; fails listing the rows to fix.
#[test]
fn fr_prv_28_real_overlay_rows_and_dry_runs() {
    let Some(p) = provider("fr_prv_28_real_overlay_rows_and_dry_runs") else {
        return;
    };
    let mut bad = vec![];
    for model in pf_provider::scenario::scenario_model_ids_for(Operation::EditMasked) {
        let c = block_on(real_call::check_model(
            &p,
            model,
            Operation::EditMasked,
            true,
        ));
        println!("{}", c.summary());
        let ok = c.found && c.offered() && matches!(c.estimate, Some(Ok(_)));
        if !ok {
            bad.push(model);
        }
    }
    assert!(bad.is_empty(), "overlay rows to fix: {bad:?}");
}

/// Free: the balance reads (needs `PF_SCENARIO_TEAM_ID`; `unknown` otherwise).
#[test]
fn fr_prv_28_real_balance() {
    let Some(p) = provider("fr_prv_28_real_balance") else {
        return;
    };
    use pf_provider::Provider as _;
    let b = block_on(p.balance()).unwrap_or_else(|e| panic!("{}", real_call::error_text(&e)));
    println!("balance: {b:?}");
}

/// PAID: one masked edit on `PF_REAL_MODEL` (default Ideogram V3 Balanced) under
/// `PF_REAL_SPEND_CU`. Refused before submit if the estimate is unknown or over the cap.
#[test]
#[ignore = "spends real credits: PF_REAL_CALL=1 PF_REAL_SPEND_CU=<CU> … -- --ignored"]
fn fr_prv_28_real_paid_masked_edit_under_a_cap() {
    let Some(p) = provider("fr_prv_28_real_paid_masked_edit_under_a_cap") else {
        return;
    };
    let cap: f64 = std::env::var("PF_REAL_SPEND_CU")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|c: &f64| *c > 0.0)
        .expect("set PF_REAL_SPEND_CU=<CU> (the spend cap for this one job)");
    let model =
        std::env::var("PF_REAL_MODEL").unwrap_or_else(|_| "model_ideogram-v3-balanced".into());
    let out = block_on(real_call::masked_edit(
        &p,
        &model,
        cap,
        &RuntimeSleeper,
        Duration::from_secs(15 * 60),
        |l| println!("{l}"),
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    println!("{}", real_call::record_line(&out.record));
    assert_eq!(out.state, ProviderJobState::Succeeded, "{:?}", out.error);
    assert!(!out.candidates.is_empty(), "no candidates");
    for (i, c) in out.candidates.iter().enumerate() {
        println!("candidate {}: {}", i + 1, c.polarity.text());
        assert!(
            !matches!(c.polarity, Polarity::Outside { .. }),
            "candidate {}: {}",
            i + 1,
            c.polarity.text()
        );
    }
}

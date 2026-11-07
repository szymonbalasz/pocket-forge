//! `pf-record`: records free Scenario calls into `fixtures/provider/<model>/<case>.json`
//! (FR-PRV-27/28). Built only with `--features real-call`; never in CI.
//!
//! ```text
//! cargo run -p pf-provider --features real-call --bin pf-record -- \
//!     --model <model-id> --case get-model|dry-run [--out fixtures/provider] [--allow-spend <CU>]
//! ```
//!
//! Credentials come from the OS credential store (`pocket-forge/scenario`), set through the
//! app's provider settings. Free cases only: paid cases need `--allow-spend` and land with
//! PRV slice 5 (owner).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use pf_provider::fixtures;
use pf_provider::scenario::record::{FixtureNames, RecordingTransport, content_hash};
use pf_provider::transport::{HttpTransport, RuntimeSleeper};
use pf_provider::{
    JobRequest, KeyringStore, Operation, Provider, ProviderAccount, ScenarioConfig,
    ScenarioProvider, block_on,
};

const FREE_CASES: [&str; 2] = ["get-model", "dry-run"];

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("pf-record: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    if std::env::var_os("CI").is_some() {
        return Err("refusing to run in CI (AGENTS §3.7)".into());
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = arg(&args, "--model").ok_or("--model is required")?;
    let case = arg(&args, "--case").ok_or("--case is required")?;
    let out_root = PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "fixtures/provider".into()));
    if !FREE_CASES.contains(&case.as_str()) {
        return Err(match arg(&args, "--allow-spend") {
            None => format!("{case:?} may spend credits; pass --allow-spend <CU> (PRV slice 5)"),
            Some(_) => format!("paid case {case:?} isn't recordable yet (PRV slice 5)"),
        });
    }

    let store = Arc::new(KeyringStore::for_provider("scenario"));
    let live = Arc::new(HttpTransport::default());
    let recorder = Arc::new(RecordingTransport::new(live));
    let base = pf_provider::scenario::DEFAULT_BASE_URL.to_owned();
    let provider = ScenarioProvider::new(ScenarioConfig {
        base_url: base.clone(),
        transport: recorder.clone(),
        sleeper: Arc::new(RuntimeSleeper),
        credentials: store,
        cache_dir: None,
    });
    let team = std::env::var("PF_SCENARIO_TEAM_ID").ok();
    let project = std::env::var("PF_SCENARIO_PROJECT_ID").ok();
    provider.set_account(ProviderAccount {
        team_id: team.clone(),
        project_id: project.clone(),
    });

    let room = fixtures::room_png(512);
    let disc = fixtures::disc_mask_png(512);
    let names: FixtureNames = [
        (content_hash(&room), "room-512.png".to_owned()),
        (content_hash(&disc), "disc-512.png".to_owned()),
    ]
    .into();

    match case.as_str() {
        "get-model" => {
            block_on(provider.get_capabilities(&model)).map_err(|e| e.to_string())?;
        }
        "dry-run" => {
            let request = JobRequest {
                operation: Operation::EditMasked,
                model_id: model.clone(),
                final_prompt: "a mossy flagstone floor".into(),
                controls: BTreeMap::new(),
                references: vec![],
                region_png: room,
                mask_png: disc,
                candidate_count: 1,
                seed: Some(1),
            };
            block_on(provider.estimate(&request)).map_err(|e| e.to_string())?;
        }
        _ => unreachable!("checked above"),
    }

    let mut ids: Vec<(&str, &'static str)> = vec![];
    if let Some(t) = team.as_deref() {
        ids.push((t, "team"));
    }
    if let Some(p) = project.as_deref() {
        ids.push((p, "project"));
    }
    let today = std::env::var("PF_RECORDED_ON").unwrap_or_else(|_| "unknown".into());
    let recording = recorder.to_recording(&base, &model, &case, &today, &names, &ids);
    let dir = out_root.join(&model);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{case}.json"));
    let json = serde_json::to_string_pretty(&recording).map_err(|e| e.to_string())?;
    std::fs::write(&path, json + "\n").map_err(|e| e.to_string())?;
    println!(
        "wrote {} ({} exchanges); run scripts/check-hygiene.sh before committing",
        path.display(),
        recording.exchanges.len()
    );
    Ok(())
}

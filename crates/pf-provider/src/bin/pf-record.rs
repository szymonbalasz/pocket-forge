//! `pf-record`: records real Scenario calls into `fixtures/provider/<model>/<case>.json`
//! (FR-PRV-27/28, PRV slice 5). Built only with `--features real-call`; refuses to run in CI.
//! Owner runbook: `docs/guides/real-provider.md`.
//!
//! ```text
//! cargo run -p pf-provider --features real-call --bin pf-record -- [--model <id>|all]
//!     [--case free|get-model|dry-run|dry-run-generate|masked-edit] [--allow-spend <CU>]
//!     [--out fixtures/provider] [--candidates .pf-dev/real-call]
//! ```
//!
//! - `free` (default; `--model all` by default): `get-model` + `dry-run` for every overlay
//!   model, plus `dry-run-generate` for every model with a generate row. Spends nothing
//!   (`?dryRun=true`, never `ipDetection`).
//! - `dry-run-generate` (free, GEN §10): one `generate` dry run per model with a generate row:
//!   the prompt, 1024 × 1024 (`wh`) or `1:1` (`aspect`), one result, no sketch, no references
//!   unless a slot is required. Recorded as `dry-run-generate.json`.
//! - `masked-edit` spends credits: it needs one `--model` and `--allow-spend <CU>`. The dry
//!   run's estimate must be known and ≤ the cap, or nothing is submitted. Candidates go to
//!   `--candidates/<model>/` (outside the repo's tracked files), with a polarity verdict.
//!
//! A failed call is still written, as `<case>.failed.json`, so it can be read and fixed;
//! committed fixtures are only overwritten by successful calls. Credentials come from the OS
//! credential store (`pocket-forge/scenario`, set in the app's provider settings); team and
//! project ids from `PF_SCENARIO_TEAM_ID` / `PF_SCENARIO_PROJECT_ID` (both optional).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pf_provider::fixtures;
use pf_provider::real_call::{self, ModelCheck};
use pf_provider::scenario::record::{FixtureNames, RecordingTransport, content_hash};
use pf_provider::transport::{HttpTransport, RuntimeSleeper};
use pf_provider::{
    KeyringStore, Operation, ProviderAccount, ScenarioConfig, ScenarioProvider, block_on,
};

/// The longest the paid edit is polled before giving up (the job may still finish).
const MAX_WAIT: Duration = Duration::from_secs(15 * 60);

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(e) => {
            eprintln!("pf-record: {e}");
            std::process::exit(1);
        }
    }
}

struct Ctx {
    base: String,
    account: ProviderAccount,
    out_root: PathBuf,
    names: FixtureNames,
    today: String,
}

impl Ctx {
    /// A fresh provider whose exchanges are captured (one recording per model and case).
    fn session(&self) -> (ScenarioProvider, Arc<RecordingTransport>) {
        let recorder = Arc::new(RecordingTransport::new(Arc::new(HttpTransport::default())));
        let p = ScenarioProvider::new(ScenarioConfig {
            base_url: self.base.clone(),
            transport: recorder.clone(),
            sleeper: Arc::new(RuntimeSleeper),
            credentials: Arc::new(KeyringStore::for_provider("scenario")),
            cache_dir: None,
        });
        p.set_account(self.account.clone());
        (p, recorder)
    }

    /// Writes `<out>/<model>/<case>.json`, or `<case>.failed.json` when `ok` is false.
    fn save(&self, rec: &RecordingTransport, model: &str, case: &str, ok: bool) -> PathBuf {
        let mut ids: Vec<(&str, &'static str)> = vec![];
        if let Some(t) = self.account.team_id.as_deref() {
            ids.push((t, "team"));
        }
        if let Some(p) = self.account.project_id.as_deref() {
            ids.push((p, "project"));
        }
        let recording = rec.to_recording(&self.base, model, case, &self.today, &self.names, &ids);
        let dir = self.out_root.join(model);
        let file = if ok {
            format!("{case}.json")
        } else {
            format!("{case}.failed.json")
        };
        let path = dir.join(file);
        let written = std::fs::create_dir_all(&dir).and_then(|()| {
            let json = serde_json::to_string_pretty(&recording).map_err(std::io::Error::other)?;
            std::fs::write(&path, json + "\n")
        });
        if let Err(e) = written {
            eprintln!("  could not write {}: {e}", path.display());
        }
        path
    }
}

fn run() -> Result<bool, String> {
    if std::env::var_os("CI").is_some_and(|v| !v.is_empty()) {
        return Err("refusing to run in CI (FR-PRV-28, AGENTS §3.7)".into());
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let case = arg(&args, "--case").unwrap_or_else(|| "free".into());
    let model = arg(&args, "--model").unwrap_or_else(|| "all".into());
    let models: Vec<String> = if model == "all" {
        pf_provider::scenario::scenario_model_ids()
            .map(str::to_owned)
            .collect()
    } else {
        vec![model.clone()]
    };
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let room = fixtures::room_png(512);
    let disc = fixtures::disc_mask_png(512);
    let ctx = Ctx {
        base: pf_provider::scenario::DEFAULT_BASE_URL.to_owned(),
        account: ProviderAccount {
            team_id: var("PF_SCENARIO_TEAM_ID"),
            project_id: var("PF_SCENARIO_PROJECT_ID"),
        },
        out_root: PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "fixtures/provider".into())),
        names: [
            (content_hash(&room), "room-512.png".to_owned()),
            (content_hash(&disc), "disc-512.png".to_owned()),
        ]
        .into(),
        today: var("PF_RECORDED_ON").unwrap_or_else(today),
    };

    match case.as_str() {
        "free" | "get-model" | "dry-run" | "dry-run-generate" => {
            if arg(&args, "--allow-spend").is_some() {
                return Err(format!("{case:?} is free; --allow-spend is not used"));
            }
            println!("pf-record: free calls only (no credits are spent)");
            let mut all_ok = true;
            let generate: Vec<&str> =
                pf_provider::scenario::scenario_model_ids_for(Operation::Generate).collect();
            for m in &models {
                if case != "dry-run-generate" {
                    all_ok &= free(&ctx, m, &case);
                }
                if matches!(case.as_str(), "free" | "dry-run-generate") {
                    if generate.contains(&m.as_str()) {
                        all_ok &= free_generate(&ctx, m);
                    } else if case == "dry-run-generate" {
                        println!("  {m:<40} no generate row (not offered for generate)");
                    }
                }
            }
            println!("Run `bash scripts/check-hygiene.sh` before committing the recordings.");
            Ok(all_ok)
        }
        "masked-edit" => {
            let cap: f64 = arg(&args, "--allow-spend")
                .ok_or("masked-edit spends credits: pass --allow-spend <CU>")?
                .parse()
                .map_err(|_| "--allow-spend takes a number of CU")?;
            if !(cap > 0.0 && cap.is_finite()) {
                return Err("--allow-spend must be a positive number of CU".into());
            }
            if model == "all" {
                return Err("masked-edit records one model: pass --model <id>".into());
            }
            let dir = PathBuf::from(
                arg(&args, "--candidates").unwrap_or_else(|| ".pf-dev/real-call".into()),
            )
            .join(&model);
            paid(&ctx, &model, cap, &dir)
        }
        other => Err(format!(
            "unknown --case {other:?} (free, get-model, dry-run, dry-run-generate, masked-edit)"
        )),
    }
}

/// `get-model` and/or `dry-run` for one model; prints a summary line. True if all went well.
/// A generate-only model (no edit row) records `get-model` through its generate row and has
/// no edit `dry-run` (its `dry-run-generate` is [`free_generate`]).
fn free(ctx: &Ctx, model: &str, case: &str) -> bool {
    let edit =
        pf_provider::scenario::scenario_model_ids_for(Operation::EditMasked).any(|m| m == model);
    if !edit && case == "dry-run" {
        println!("  {model:<40} no edit row (generate only): no edit dry run");
        return true;
    }
    let mut check: Option<ModelCheck> = None;
    if case != "dry-run" {
        let (p, rec) = ctx.session();
        let op = if edit {
            Operation::EditMasked
        } else {
            Operation::Generate
        };
        let c = block_on(real_call::check_model(&p, model, op, false));
        ctx.save(&rec, model, "get-model", c.found);
        check = Some(c);
    }
    if edit && case != "get-model" && check.as_ref().is_none_or(ModelCheck::offered) {
        let (p, rec) = ctx.session();
        let c = block_on(real_call::check_model(
            &p,
            model,
            Operation::EditMasked,
            true,
        ));
        let ok = matches!(c.estimate, Some(Ok(_)));
        if c.estimate.is_some() {
            ctx.save(&rec, model, "dry-run", ok);
        }
        check = Some(c);
    }
    let c = check.expect("one case ran");
    println!("  {}", c.summary());
    c.found && c.offered() && c.estimate.as_ref().is_none_or(Result::is_ok)
}

/// One free `generate` dry run for one model (`dry-run-generate`); prints a summary line.
fn free_generate(ctx: &Ctx, model: &str) -> bool {
    let (p, rec) = ctx.session();
    let c = block_on(real_call::check_model(&p, model, Operation::Generate, true));
    let ok = matches!(c.estimate, Some(Ok(_)));
    if c.estimate.is_some() {
        ctx.save(&rec, model, "dry-run-generate", ok);
    }
    println!("  generate: {}", c.summary());
    c.found && c.offered() && ok
}

/// One paid masked edit under `cap`, recorded; candidates and the sent files go to `dir`.
fn paid(ctx: &Ctx, model: &str, cap: f64, dir: &Path) -> Result<bool, String> {
    println!("pf-record: ONE PAID masked edit on {model}, at most {cap} CU (real credits)");
    let (p, rec) = ctx.session();
    let out = block_on(real_call::masked_edit(
        &p,
        model,
        cap,
        &RuntimeSleeper,
        MAX_WAIT,
        |line| println!("  {line}"),
    ));
    let ok = matches!(&out, Ok(o) if !o.candidates.is_empty());
    let path = ctx.save(&rec, model, "masked-edit", ok);
    println!("  recording: {}", path.display());
    let out = out?;
    println!("  {}", real_call::record_line(&out.record));
    println!(
        "  final state {:?} after {} s; estimate {}, actual {}",
        out.state,
        out.waited.as_secs(),
        real_call::cost_text(&out.estimate),
        out.actual
            .as_ref()
            .map_or("not reported".into(), real_call::cost_text)
    );
    if let Some(e) = &out.error {
        println!("  provider error: {e}");
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let write = |name: String, bytes: &[u8]| {
        let path = dir.join(name);
        match std::fs::write(&path, bytes) {
            Ok(()) => println!("  wrote {}", path.display()),
            Err(e) => eprintln!("  could not write {}: {e}", path.display()),
        }
    };
    write("sent.png".into(), &out.request.region_png);
    write("mask-white-is-edit.png".into(), &out.request.mask_png);
    for (i, c) in out.candidates.iter().enumerate() {
        write(format!("candidate-{}.{}", i + 1, c.ext), &c.bytes);
        println!("  candidate {}: {}", i + 1, c.polarity.text());
    }
    Ok(ok)
}

/// Today's UTC date (`YYYY-MM-DD`) for `recorded_on`.
fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Civil from days (H. Hinnant).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

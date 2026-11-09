//! RGN slices 3–5 integration tests: pf-core + a real asset project on disk + the **fake
//! provider** (R17 cases). Generated images only (KQ-22). No real provider, no spend.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use image::{Rgb, RgbImage};
use pf_provider::AnyProvider;
use pf_provider::fake::{FakeCase, FakeProvider};
use serde_json::{Value, json};
use ulid::Ulid;

use super::runner;
use crate::context::{Clock, Context, ManualClock, RecordedEvents, SeededRng, UlidGen};
use crate::core::Core;
use crate::create::{CreateItem, create_projects};
use crate::error::{CoreError, ErrorCode};
use crate::job::{JOB_FILE, JobFile, JobState};
use crate::manifest::{CandidateStatus, Kind, MANIFEST_FILE, Rect, job_dir};
use crate::project::store;
use crate::time::Timestamp;

pub const W: u32 = 1200;
pub const H: u32 = 900;
pub const INNER: Rect = Rect {
    x: 400,
    y: 300,
    w: 300,
    h: 200,
};
const T0: i64 = 1_800_000_000_000;

/// A textured "map": every pixel differs from its neighbours.
pub fn original() -> RgbImage {
    RgbImage::from_fn(W, H, |x, y| {
        Rgb([
            (x % 251) as u8,
            (y % 241) as u8,
            ((x * 7 + y * 13) % 256) as u8,
        ])
    })
}

/// A disc painted in the middle of the region (255 inside, soft edge).
pub fn disc_mask(w: u32, h: u32) -> Vec<u8> {
    let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    let r = f64::from(w.min(h)) / 3.0;
    (0..w * h)
        .map(|i| {
            let (x, y) = (f64::from(i % w) + 0.5, f64::from(i / w) + 0.5);
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            ((r + 2.0 - d) / 4.0 * 255.0).clamp(0.0, 255.0) as u8
        })
        .collect()
}

/// A manual clock the test keeps a handle to.
struct SharedClock(Arc<ManualClock>);

impl Clock for SharedClock {
    fn now(&self) -> Timestamp {
        self.0.now()
    }
}

pub(crate) fn context(
    root: &Path,
    events: Arc<RecordedEvents>,
    clock: Arc<ManualClock>,
    seed: u64,
) -> Context {
    let mut ctx = Context::deterministic(root.join("data"), clock.now());
    ctx.library_override = Some(root.join("lib"));
    ctx.clock = Box::new(SharedClock(clock));
    ctx.ids = Box::new(UlidGen);
    ctx.rng = Box::new(SeededRng::new(seed));
    ctx.events = events;
    ctx
}

/// A core with one open Maps project, a stored mask and a spend cap.
pub struct Lab {
    pub dir: tempfile::TempDir,
    pub core: Core,
    pub events: Arc<RecordedEvents>,
    pub project_dir: PathBuf,
    pub mask_id: Ulid,
    pub clock: Arc<ManualClock>,
    generation: u64,
}

impl Lab {
    pub fn new() -> Lab {
        Self::with_original(original())
    }

    pub fn with_original(img: RgbImage) -> Lab {
        Self::build(img, |_| {})
    }

    /// [`Lab::new`] with the context adjusted before the core starts (PRM: the chat seams).
    pub fn with_context(tweak: impl FnOnce(&mut crate::context::Context)) -> Lab {
        Self::build(original(), tweak)
    }

    fn build(img: RgbImage, tweak: impl FnOnce(&mut crate::context::Context)) -> Lab {
        let dir = tempfile::tempdir().unwrap();
        let events = Arc::new(RecordedEvents::default());
        let clock = Arc::new(ManualClock::new(Timestamp(T0)));
        let mut ctx = context(dir.path(), events.clone(), clock.clone(), 1);
        tweak(&mut ctx);
        let core = Core::new(ctx).unwrap();
        let src = dir.path().join("outside").join("rooms.png");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        img.save(&src).unwrap();
        let settings = core.state().settings.clone();
        let items = create_projects(
            &core.ctx,
            &settings,
            Kind::Map,
            &[src],
            &AtomicBool::new(false),
            None,
        )
        .unwrap();
        let project_dir = match &items[0] {
            CreateItem::Created { path, .. } => path.clone(),
            other => panic!("{other:?}"),
        };
        let mut lab = Lab {
            dir,
            core,
            events,
            project_dir,
            mask_id: Ulid::nil(),
            clock,
            generation: 1,
        };
        lab.open();
        lab.mask_id = lab.put_mask(INNER, &disc_mask(INNER.w, INNER.h));
        lab.core
            .dispatch("rgn_spend_set", json!({"cap": 100, "threshold": 50}))
            .unwrap();
        lab
    }

    pub fn put_mask(&self, rect: Rect, grey: &[u8]) -> Ulid {
        let mut state = self.core.state();
        let install = state.settings.install_id;
        let p = state.project.as_mut().unwrap();
        store::put_mask(&self.core.ctx, install, p, rect, grey.to_vec())
            .unwrap()
            .id
    }

    pub fn open(&self) -> Value {
        self.core
            .dispatch(
                "project_open",
                json!({"path": self.project_dir.to_string_lossy()}),
            )
            .unwrap()
    }

    pub fn close(&self) {
        self.core.dispatch("project_close", json!({})).unwrap();
    }

    pub fn fake(&self) -> &FakeProvider {
        self.core.ctx.provider.as_fake().unwrap()
    }

    pub fn set_case(&self, case: &str) {
        self.fake().set_case(FakeCase::parse(case).unwrap());
    }

    /// Quits the app: a new core on the same data dir and library. The fake provider moves over,
    /// like a provider's server that still knows the job.
    pub fn restart(mut self) -> Lab {
        // The context is shared (`Arc`) with background threads; none run in these tests.
        let ctx = std::sync::Arc::get_mut(&mut self.core.ctx).expect("the context is not shared");
        let provider = std::mem::replace(&mut ctx.provider, AnyProvider::fake());
        self.generation += 1;
        let events = Arc::new(RecordedEvents::default());
        // The app is restarted an hour later.
        self.clock.advance(std::time::Duration::from_secs(3600));
        let mut ctx = context(
            self.dir.path(),
            events.clone(),
            self.clock.clone(),
            self.generation,
        );
        ctx.provider = provider;
        let Lab {
            dir,
            project_dir,
            mask_id,
            clock,
            generation,
            ..
        } = self;
        Lab {
            core: Core::new(ctx).unwrap(),
            events,
            dir,
            project_dir,
            mask_id,
            clock,
            generation,
        }
    }

    pub fn draft(&self, prompt: &str) -> Value {
        json!({
            "region": {"inner": {"x": INNER.x, "y": INNER.y, "w": INNER.w, "h": INNER.h}, "margin": 128},
            "mask_id": self.mask_id,
            "model_id": "fake-white-edit",
            "final_prompt": prompt,
            "controls": {},
            "seed": null,
        })
    }

    pub fn estimate(&self, draft: &Value) -> Value {
        self.core
            .dispatch("rgn_job_estimate", json!({ "draft": draft }))
            .unwrap()
    }

    pub fn submit_draft(&self, draft: Value) -> Result<Ulid, CoreError> {
        // Time moves on between submits, so "newest" is well defined.
        self.clock.advance(std::time::Duration::from_secs(60));
        let est = self.estimate(&draft);
        let confirmations = est["needs"].clone();
        self.core
            .dispatch(
                "rgn_job_submit",
                json!({"draft": draft, "estimate_id": est["estimate_id"], "confirmations": confirmations}),
            )
            .map(|v| v["job_id"].as_str().unwrap().parse().unwrap())
    }

    pub fn submit(&self, prompt: &str) -> Ulid {
        self.submit_draft(self.draft(prompt)).unwrap()
    }

    /// Submits and drives the job to its end.
    pub fn run(&self, prompt: &str) -> Ulid {
        let id = self.submit(prompt);
        self.drive();
        id
    }

    pub fn drive(&self) {
        let left = runner::drive(&self.core, 50);
        assert!(left < 50, "the runner never finished");
    }

    /// The job file on disk (authoritative, R2).
    pub fn job(&self, id: Ulid) -> JobFile {
        let path = self.project_dir.join(job_dir(id)).join(JOB_FILE);
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    pub fn manifest(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.project_dir.join(MANIFEST_FILE)).unwrap())
            .unwrap()
    }

    pub fn candidates(&self, job: Ulid) -> Vec<crate::manifest::Candidate> {
        let state = self.core.state();
        state
            .project
            .as_ref()
            .unwrap()
            .manifest
            .candidates
            .iter()
            .filter(|c| c.job_id == job)
            .cloned()
            .collect()
    }

    pub fn events_named(&self, name: &str) -> Vec<Value> {
        self.events
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .collect()
    }

    /// What a failed command must leave alone (FR-GEN-35, FR-PRM-40): every file in the library
    /// (the project, its chat thread, new projects) with its bytes, `settings.json`, and the
    /// settings and manifest in memory.
    pub fn persisted(&self) -> Value {
        fn files(dir: &Path, out: &mut std::collections::BTreeMap<String, String>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    files(&p, out);
                } else {
                    let bytes = std::fs::read(&p).unwrap_or_default();
                    use sha2::Digest;
                    let hash: String = sha2::Sha256::digest(&bytes)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    out.insert(p.to_string_lossy().replace('\\', "/"), hash);
                }
            }
        }
        let mut lib = std::collections::BTreeMap::new();
        files(&self.dir.path().join("lib"), &mut lib);
        let settings_file = std::fs::read_to_string(crate::settings::settings_path(
            &self.dir.path().join("data"),
        ))
        .ok();
        let state = self.core.state();
        json!({
            "library": lib,
            "settings_file": settings_file,
            "settings": serde_json::to_value(&state.settings).unwrap(),
            "manifest": state.project.as_ref().map(|p| serde_json::to_value(&p.manifest).unwrap()),
        })
    }

    pub fn states_emitted(&self, job: Ulid) -> Vec<String> {
        let mut out: Vec<String> = self
            .events_named(runner::EVENT_JOB_STATE)
            .iter()
            .filter(|e| e["job_id"] == json!(job))
            .map(|e| e["state"].as_str().unwrap().to_owned())
            .collect();
        out.dedup();
        out
    }
}

fn code(r: Result<impl std::fmt::Debug, CoreError>) -> ErrorCode {
    r.unwrap_err().code
}

// ---------------------------------------------------------------- slice 3: submit

#[test]
fn fr_rgn_17_submit_commits_requested_then_submitted_with_the_handle() {
    let lab = Lab::new();
    let id = lab.submit("turn this room into a kitchen");
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Submitted);
    assert!(job.state_times.contains_key("requested"));
    assert!(job.state_times.contains_key("submitted"));
    assert!(
        job.provider["job_handle"].is_string(),
        "PRV's opaque record holds the handle"
    );
    assert_eq!(
        job.references,
        Vec::<Value>::new(),
        "FR-RGN-43: [] when none"
    );
    for f in job.input_files() {
        assert!(lab.project_dir.join(f).is_file(), "inputs as sent");
    }
    assert_eq!(job.fit["sent_w"], json!(560), "grown to multiple_of");
    assert_eq!(job.region["margin"], json!(128));
    assert_eq!(job.candidate_count, 3);
    assert!(job.estimate["amount"] == json!(3.0));
    assert!(job.capability_snapshot["model_id"] == json!("fake-white-edit"));
    // The manifest index row (R2).
    assert!(
        lab.manifest()["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == json!(id))
    );
    // Session spend counts the estimate (FR-RGN-14).
    assert_eq!(
        lab.core.dispatch("rgn_spend_get", json!({})).unwrap()["spend"],
        json!(3.0)
    );
}

#[test]
fn fr_rgn_17_one_active_job_per_project() {
    let lab = Lab::new();
    lab.submit("one");
    assert_eq!(
        code(lab.submit_draft(lab.draft("two"))),
        ErrorCode::JobActive
    );
}

#[test]
fn fr_rgn_13_submit_with_a_stale_estimate_id_fails() {
    let lab = Lab::new();
    let draft = lab.draft("a");
    let est = lab.estimate(&draft);
    let mut changed = draft.clone();
    changed["region"]["margin"] = json!(64);
    let r = lab.core.dispatch(
        "rgn_job_submit",
        json!({"draft": changed, "estimate_id": est["estimate_id"], "confirmations": []}),
    );
    assert_eq!(code(r), ErrorCode::EstimateStale);
    assert!(
        lab.manifest()["jobs"].as_array().unwrap().is_empty(),
        "nothing recorded"
    );
}

#[test]
fn fr_rgn_14_15_submit_checks_the_cap_and_confirmations() {
    let lab = Lab::new();
    lab.core
        .dispatch("rgn_spend_set", json!({"cap": 2, "threshold": 1}))
        .unwrap();
    let draft = lab.draft("a");
    let est = lab.estimate(&draft);
    let submit = |c: Value| {
        lab.core.dispatch(
            "rgn_job_submit",
            json!({"draft": draft, "estimate_id": est["estimate_id"], "confirmations": c}),
        )
    };
    assert_eq!(code(submit(json!(["threshold"]))), ErrorCode::OverSpendCap);
    lab.core
        .dispatch("rgn_spend_set", json!({"cap": 10}))
        .unwrap();
    assert_eq!(code(submit(json!([]))), ErrorCode::ConfirmationMissing);
    assert!(submit(json!(["threshold"])).is_ok());
}

#[test]
fn fr_rgn_15_unknown_cost_needs_its_confirmation() {
    let lab = Lab::new();
    lab.set_case("estimate_unknown");
    let draft = lab.draft("a");
    let est = lab.estimate(&draft);
    assert_eq!(est["needs"], json!(["unknown_cost"]));
    let r = lab.core.dispatch(
        "rgn_job_submit",
        json!({"draft": draft, "estimate_id": est["estimate_id"], "confirmations": []}),
    );
    assert_eq!(code(r), ErrorCode::ConfirmationMissing);
    assert!(lab.submit_draft(lab.draft("a")).is_ok());
}

#[test]
fn fr_rgn_09_owner_55_with_the_blend_instruction_off_the_typed_prompt_is_sent_byte_for_byte() {
    let lab = Lab::new();
    let prompt = "  Turn this room into a kitchen — ünïcode, trailing  ";
    let mut d = lab.draft(prompt);
    d["blend_instruction"] = json!(false);
    let id = lab.submit_draft(d).unwrap();
    let job = lab.job(id);
    assert_eq!(job.final_prompt, prompt);
    assert_eq!(job.provider["request"]["prompt"], json!(prompt));
    assert_eq!(job.extra["blend_instruction"], json!(false));
}

/// What a Maps project appends with the default text (#55).
fn map_instruction() -> String {
    format!(
        "{} Top-down battlemap view.",
        super::instruction::DEFAULT_BLEND_INSTRUCTION
    )
}

#[test]
fn owner_55_on_by_default_appended_once_and_recorded_on_the_job() {
    let lab = Lab::new();
    // No `blend_instruction` in the draft: on.
    let id = lab.submit("a stone well");
    let job = lab.job(id);
    let sent = format!(
        "a stone well

{}",
        map_instruction()
    );
    assert_eq!(job.final_prompt, sent, "the prompt as sent");
    assert_eq!(job.provider["request"]["prompt"], json!(sent));
    assert_eq!(job.extra["blend_instruction"], json!(true));
    assert_eq!(job.extra["typed_prompt"], json!("a stone well"));
    assert_eq!(job.final_prompt.matches("battlemap").count(), 1);
    lab.drive();

    // Retry prefills the typed prompt, so a resubmit appends the instruction once.
    let retry = lab
        .core
        .dispatch("rgn_job_retry_draft", json!({"job_id": id}))
        .unwrap();
    assert_eq!(retry["final_prompt"], "a stone well");
    assert_eq!(retry["blend_instruction"], json!(true));
    // The iteration is named after the typed prompt, not the instruction.
    let dir = lab.job(id).iteration.unwrap().dir;
    assert_eq!(dir, "results/001 a stone well");
    let id2 = lab.submit_draft(retry).unwrap();
    assert_eq!(lab.job(id2).final_prompt, sent);
    // A prompt that already ends with the instruction is not extended again.
    let id3 = {
        lab.drive();
        lab.submit(&sent)
    };
    assert_eq!(lab.job(id3).final_prompt, sent);
}

#[test]
fn owner_55_the_estimate_does_not_depend_on_the_blend_instruction() {
    let lab = Lab::new();
    let on = lab.estimate(&lab.draft("a well"));
    let mut d = lab.draft("a well");
    d["blend_instruction"] = json!(false);
    let off = lab.estimate(&d);
    assert_eq!(on["estimate_id"], off["estimate_id"]);
    assert_eq!(on["cost"], off["cost"]);
}

#[test]
fn owner_55_settings_text_is_used_and_reset_to_default() {
    let lab = Lab::new();
    let got = lab
        .core
        .dispatch("rgn_blend_instruction_get", json!({}))
        .unwrap();
    assert_eq!(got["is_default"], json!(true));
    assert_eq!(got["for_project"], json!(map_instruction()));
    let set = lab
        .core
        .dispatch(
            "rgn_blend_instruction_set",
            json!({"text": "Match the ink."}),
        )
        .unwrap();
    assert_eq!(set["text"], "Match the ink.");
    assert_eq!(set["is_default"], json!(false));
    assert_eq!(lab.events_named("rgn:blend-instruction").len(), 1);
    // Stored per machine in settings.json `region_edit`.
    let file: Value = serde_json::from_slice(
        &std::fs::read(lab.dir.path().join("data").join("settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(file["region_edit"]["blend_instruction"], "Match the ink.");

    let id = lab.submit("a well");
    assert_eq!(
        lab.job(id).final_prompt,
        "a well

Match the ink. Top-down battlemap view."
    );
    let reset = lab
        .core
        .dispatch("rgn_blend_instruction_set", json!({"text": null}))
        .unwrap();
    assert_eq!(reset["is_default"], json!(true));
    assert_eq!(
        reset["text"],
        json!(super::instruction::DEFAULT_BLEND_INSTRUCTION)
    );
    let e = lab
        .core
        .dispatch(
            "rgn_blend_instruction_set",
            json!({"text": "x".repeat(2001)}),
        )
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidInput);
}

#[test]
fn owner_55_the_draft_keeps_the_setting_per_project() {
    let lab = Lab::new();
    lab.core
        .dispatch(
            "project_put_draft",
            json!({"draft": {"final_prompt": "x", "blend_instruction": false}}),
        )
        .unwrap();
    assert_eq!(lab.manifest()["draft"]["blend_instruction"], json!(false));
    lab.close();
    let snap = lab.open();
    assert_eq!(snap["draft"]["blend_instruction"], json!(false));
}

#[test]
fn fr_rgn_10_the_seed_used_is_recorded_random_or_pinned() {
    let lab = Lab::new();
    let id = lab.run("random");
    let job = lab.job(id);
    let used = job
        .seed
        .requested
        .expect("a random seed is drawn and recorded");
    assert_eq!(job.provider["request"]["seed"], json!(used));
    assert!(job.seed.returned.is_some());
    let mut d = lab.draft("pinned");
    d["seed"] = json!(1234);
    let id = lab.submit_draft(d).unwrap();
    assert_eq!(lab.job(id).seed.requested, Some(1234));
}

// ---------------------------------------------------------------- slice 3: the runner

#[test]
fn fr_rgn_35_every_transition_is_committed_and_emitted() {
    let lab = Lab::new();
    lab.set_case("slow(2)");
    let id = lab.submit("kitchen");
    runner::tick(&lab.core, true);
    assert_eq!(lab.job(id).state, JobState::Queued);
    runner::tick(&lab.core, true);
    assert_eq!(lab.job(id).state, JobState::Running);
    lab.drive();
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Fetched);
    for s in [
        "requested",
        "submitted",
        "queued",
        "running",
        "succeeded",
        "fetched",
    ] {
        assert!(job.state_times.contains_key(s), "{s} stamped");
    }
    assert_eq!(
        lab.states_emitted(id),
        [
            "requested",
            "submitted",
            "queued",
            "running",
            "succeeded",
            "fetched"
        ]
    );
    assert_eq!(job.cost["amount"], json!(3.0), "actual cost from status");
    let progress: Vec<f64> = lab
        .events_named(runner::EVENT_JOB_STATE)
        .iter()
        .filter_map(|e| e["progress"].as_f64())
        .collect();
    assert!(progress.iter().any(|p| *p > 0.0 && *p < 1.0));
    assert!(lab.core.state().rgn.runner.tracked().is_empty());
}

#[test]
fn fr_rgn_18_long_running_turns_on_at_180_s_on_the_clock_seam() {
    let lab = Lab::new();
    lab.set_case("slow(10)");
    let id = lab.submit("kitchen");
    let submitted = lab.job(id).state_times["submitted"];
    let last_long_running = || {
        lab.events_named(runner::EVENT_JOB_STATE)
            .iter()
            .rfind(|e| e["job_id"] == json!(id))
            .map(|e| e["long_running"].clone())
            .unwrap()
    };
    runner::tick(&lab.core, true);
    assert_eq!(last_long_running(), json!(false));
    lab.clock
        .set(submitted.plus(std::time::Duration::from_secs(179)));
    runner::tick(&lab.core, true);
    assert_eq!(lab.job(id).state, JobState::Running);
    assert_eq!(last_long_running(), json!(false), "179 s is not yet long");
    lab.clock
        .set(submitted.plus(std::time::Duration::from_secs(180)));
    runner::tick(&lab.core, true);
    assert!(!lab.job(id).state.is_terminal());
    assert_eq!(last_long_running(), json!(true), "180 s shows the notice");
    lab.drive();
    assert_eq!(lab.job(id).state, JobState::Fetched);
    assert_eq!(last_long_running(), json!(false), "never on a finished job");
}

#[test]
fn fr_rgn_35_polling_continues_after_the_project_closes() {
    let lab = Lab::new();
    let id = lab.submit("kitchen");
    runner::tick(&lab.core, true);
    lab.close();
    lab.drive();
    // Job state went straight to job.json; the candidates wait in the spool.
    assert_eq!(lab.job(id).state, JobState::Succeeded);
    let pending = lab.dir.path().join("data/pending");
    assert!(pending.is_dir());
    lab.open();
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Fetched, "replayed on open");
    assert_eq!(lab.candidates(id).len(), 3);
    assert!(
        !lab.dir
            .path()
            .join("data/pending")
            .join(lab.manifest()["project_id"].as_str().unwrap())
            .exists()
    );
}

#[test]
fn fr_rgn_22_resume_after_recreating_the_core_mid_running() {
    let lab = Lab::new();
    lab.set_case("slow(3)");
    let id = lab.submit("kitchen");
    runner::tick(&lab.core, true);
    runner::tick(&lab.core, true);
    assert_eq!(lab.job(id).state, JobState::Running);
    let lab = lab.restart();
    assert!(lab.core.state().rgn.runner.tracked().is_empty());
    let snap = lab.open();
    assert!(
        snap["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "resumable")
    );
    assert_eq!(lab.core.state().rgn.runner.tracked(), vec![id]);
    lab.drive();
    assert_eq!(lab.job(id).state, JobState::Fetched);
    assert_eq!(lab.candidates(id).len(), 3);
}

#[test]
fn fr_rgn_22_succeeded_job_fetches_on_open() {
    let lab = Lab::new();
    let id = lab.submit("kitchen");
    runner::tick(&lab.core, true);
    runner::tick(&lab.core, true);
    runner::tick(&lab.core, true);
    assert_eq!(lab.job(id).state, JobState::Succeeded);
    // Quit before the fetch.
    let lab = lab.restart();
    lab.open();
    lab.drive();
    assert_eq!(lab.job(id).state, JobState::Fetched);
}

#[test]
fn fr_rgn_22_a_crash_before_the_handle_is_lost_with_a_possible_charge_notice() {
    let lab = Lab::new();
    // A `requested` record without a handle, as a crash right before submit returns leaves it.
    let id = {
        let mut state = lab.core.state();
        let install = state.settings.install_id;
        let p = state.project.as_mut().unwrap();
        let new = store::NewJob {
            operation: "edit_masked".into(),
            model_id: "fake-white-edit".into(),
            final_prompt: "kitchen".into(),
            mask_id: Some(lab.mask_id),
            ..Default::default()
        };
        store::record_job(&lab.core.ctx, install, p, new, vec![1], vec![2])
            .unwrap()
            .id
    };
    let lab = lab.restart();
    let snap = lab.open();
    assert_eq!(lab.job(id).state, JobState::Lost);
    assert!(
        snap["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "lost" && w["detail"]["job_ids"][0] == json!(id))
    );
    assert!(
        lab.core.state().rgn.runner.tracked().is_empty(),
        "never resubmitted"
    );
    // R108: no handle, so the strip says the result can't be recovered.
    let rows = lab.core.dispatch("rgn_iterations_list", json!({})).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["job_id"] == json!(id))
        .unwrap();
    assert_eq!(row["handle_kept"], false);
    assert_eq!(row["error"]["code"], "lost");
}

/// A `requested` record without a handle, as a submit in flight (or a crash before it
/// returned) leaves it.
fn record_requested(lab: &Lab, prompt: &str) -> Ulid {
    let mut state = lab.core.state();
    let install = state.settings.install_id;
    let p = state.project.as_mut().unwrap();
    let new = store::NewJob {
        operation: "edit_masked".into(),
        model_id: "fake-white-edit".into(),
        final_prompt: prompt.into(),
        mask_id: Some(lab.mask_id),
        ..Default::default()
    };
    store::record_job(&lab.core.ctx, install, p, new, vec![1], vec![2])
        .unwrap()
        .id
}

fn has_lost_warning(snap: &Value, id: Ulid) -> bool {
    snap["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["code"] == "lost" && w["detail"]["job_ids"][0] == json!(id))
}

/// Review item 3 / R102: the open sweep marks a `requested` job `lost` only if this process
/// isn't submitting it and it's older than the submit timeout + 60 s. A young one (another
/// machine may be submitting it) ages out in the runner, with the possible-charge notice.
#[test]
fn fr_prj_29_a_requested_job_is_lost_only_when_not_in_flight_and_old() {
    let lab = Lab::new();
    let id = record_requested(&lab, "kitchen");
    // In flight here: never lost, however old, across a close and reopen.
    let guard = lab.core.ctx.submitting.begin(id);
    lab.close();
    lab.clock.advance(std::time::Duration::from_secs(3600));
    let snap = lab.open();
    assert_eq!(lab.job(id).state, JobState::Requested);
    assert!(!has_lost_warning(&snap, id));
    runner::tick(&lab.core, true);
    assert_eq!(
        lab.job(id).state,
        JobState::Requested,
        "the runner waits too"
    );
    drop(guard);

    // Not in flight, but young (another machine may be submitting it): kept.
    let lab = Lab::new();
    let id = record_requested(&lab, "kitchen");
    lab.close();
    lab.clock.advance(std::time::Duration::from_secs(60));
    let snap = lab.open();
    assert_eq!(lab.job(id).state, JobState::Requested);
    assert!(!has_lost_warning(&snap, id));
    runner::tick(&lab.core, true);
    assert_eq!(lab.job(id).state, JobState::Requested);
    // Once older than the submit timeout + 60 s, the runner marks it lost.
    lab.clock.advance(std::time::Duration::from_secs(61));
    runner::tick(&lab.core, true);
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Lost);
    assert!(
        job.error.as_ref().unwrap()["hint"]
            .as_str()
            .unwrap()
            .contains("charged")
    );
    assert!(lab.core.state().rgn.runner.tracked().is_empty());
    let last = lab.events_named(runner::EVENT_JOB_STATE).pop().unwrap();
    assert_eq!(last["state"], "lost");
}

/// Review item 3 / R102: a submit that returns a handle for a job already marked `lost` saves
/// the handle and resumes polling (`lost -> submitted` only for that case); the candidates are
/// fetched, nothing is submitted twice.
#[test]
fn fr_rgn_23_a_submit_returning_for_a_lost_job_saves_the_handle_and_polls() {
    let lab = Lab::new();
    // A real handle the fake knows (its job is done, so it fetches at once).
    let first = lab.run("first");
    let returned = pf_provider::ProviderRecord(lab.job(first).provider.clone());
    let id = record_requested(&lab, "kitchen");
    let (pid, dir, requested) = {
        let state = lab.core.state();
        let p = state.project.as_ref().unwrap();
        (p.manifest.project_id, p.dir.clone(), p.jobs[&id].clone())
    };
    // Another machine's sweep (or an older build) marked it lost meanwhile.
    lab.close();
    lab.clock.advance(std::time::Duration::from_secs(3600));
    lab.open();
    assert_eq!(lab.job(id).state, JobState::Lost);
    let submits = lab.fake().call_count();
    let out = super::jobs::finish_submit(
        &lab.core,
        requested,
        pid,
        dir,
        pf_provider::CostAmount::Known(1.0),
        Ok(returned.clone()),
    )
    .unwrap();
    assert_eq!(out.job_id, id);
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Submitted, "lost -> submitted");
    assert_eq!(job.provider["job_handle"], returned.0["job_handle"]);
    assert!(job.error.is_none(), "the lost notice is cleared");
    assert_eq!(lab.core.state().rgn.runner.tracked(), vec![id]);
    lab.drive();
    assert_eq!(lab.job(id).state, JobState::Fetched);
    assert!(!lab.candidates(id).is_empty());
    assert_eq!(
        lab.fake().call_count() - submits,
        2,
        "one status and one fetch; no second submit"
    );
    // A lost job that had a handle (polling gave up) is never revived this way.
    let mut j = lab.job(first);
    j.state = JobState::Lost;
    assert!(j.transition(JobState::Submitted, Timestamp(T0)).is_err());
}

#[test]
fn fr_rgn_19_cancel_on_ack_and_not_cancellable() {
    let lab = Lab::new();
    let id = lab.submit("kitchen");
    let out = lab
        .core
        .dispatch("rgn_job_cancel", json!({"job_id": id}))
        .unwrap();
    assert_eq!(out, json!({"result": "canceled"}));
    assert_eq!(lab.job(id).state, JobState::Canceled);
    assert!(lab.core.state().rgn.runner.tracked().is_empty());
    lab.drive();
    assert_eq!(lab.job(id).state, JobState::Canceled);

    lab.set_case("not_cancellable");
    let id = lab.submit("again");
    let out = lab
        .core
        .dispatch("rgn_job_cancel", json!({"job_id": id}))
        .unwrap();
    assert_eq!(out, json!({"result": "not_cancellable"}));
    lab.drive();
    assert_eq!(lab.job(id).state, JobState::Fetched, "the job continues");
    assert_eq!(
        code(lab.core.dispatch("rgn_job_cancel", json!({"job_id": id}))),
        ErrorCode::InvalidInput
    );
}

#[test]
fn fr_rgn_19_provider_side_cancel_ends_the_job() {
    let lab = Lab::new();
    lab.set_case("canceled");
    let id = lab.run("kitchen");
    assert_eq!(lab.job(id).state, JobState::Canceled);
}

#[test]
fn fr_rgn_20_a_failed_job_keeps_its_error_and_is_never_resubmitted() {
    let lab = Lab::new();
    lab.set_case("failure");
    let id = lab.run("kitchen");
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.error.as_ref().unwrap()["code"], "provider_failure");
    assert!(
        job.error.as_ref().unwrap()["hint"]
            .as_str()
            .unwrap()
            .contains("failed")
    );
    assert_eq!(
        lab.manifest()["jobs"].as_array().unwrap().len(),
        1,
        "no resubmit"
    );
}

#[test]
fn fr_rgn_20_insufficient_balance_fails_before_anything_is_recorded() {
    let lab = Lab::new();
    let draft = lab.draft("kitchen");
    let est = lab.estimate(&draft);
    lab.set_case("insufficient_balance");
    let e = lab
        .core
        .dispatch(
            "rgn_job_submit",
            json!({"draft": draft, "estimate_id": est["estimate_id"], "confirmations": []}),
        )
        .unwrap_err();
    assert_eq!(e.detail["provider"]["code"], "insufficient_balance");
    assert!(lab.manifest()["jobs"].as_array().unwrap().is_empty());
    assert_eq!(
        lab.core.dispatch("rgn_spend_get", json!({})).unwrap()["spend"],
        json!(0.0)
    );
}

#[test]
fn fr_rgn_09_33_a_blank_required_prompt_never_reaches_the_provider() {
    let lab = Lab::new();
    // The prompt isn't a cost input: estimate with one, then submit a blank one.
    let draft = lab.draft("kitchen");
    let est = lab.estimate(&draft);
    let mut blank = draft.clone();
    blank["final_prompt"] = json!("   ");
    let e = lab
        .core
        .dispatch(
            "rgn_job_submit",
            json!({"draft": blank, "estimate_id": est["estimate_id"], "confirmations": []}),
        )
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidControl);
    assert_eq!(e.detail["reason"], "required");
    assert!(
        lab.manifest()["jobs"].as_array().unwrap().is_empty(),
        "nothing recorded"
    );
}

#[test]
fn fr_rgn_23_offline_mid_job_is_unknown_until_reconnect() {
    let lab = Lab::new();
    let id = lab.submit("kitchen");
    runner::tick(&lab.core, true);
    lab.set_case("offline");
    runner::tick(&lab.core, true);
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Unknown);
    assert_eq!(job.error.as_ref().unwrap()["code"], "offline");
    runner::tick(&lab.core, true);
    assert_eq!(lab.job(id).state, JobState::Unknown, "still waiting");
    lab.set_case("success");
    lab.drive();
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Fetched);
    assert!(job.error.is_none(), "cleared on reconnect");
}

/// Review item 2 / R102: a paid `succeeded` job never becomes `failed` on a fetch error. It
/// stays `succeeded` with the error shown, keeps retrying with backoff, and "Fetch again"
/// fetches at once without a new submit.
#[test]
fn fr_rgn_24_a_fetch_error_keeps_the_job_succeeded_and_fetch_again_never_resubmits() {
    let lab = Lab::new();
    let id = lab.submit("kitchen");
    for _ in 0..3 {
        runner::tick(&lab.core, true);
    }
    assert_eq!(lab.job(id).state, JobState::Succeeded);
    let calls_before = lab.fake().call_count();
    lab.set_case("auth");
    let t0 = lab.core.ctx.now();
    let mut waits = vec![];
    for _ in 0..4 {
        runner::tick(&lab.core, true);
        let job = lab.job(id);
        assert_eq!(job.state, JobState::Succeeded, "never failed");
        assert_eq!(job.error.as_ref().unwrap()["code"], "auth");
        waits.push(runner::tick(&lab.core, false).unwrap());
    }
    assert!(
        waits.windows(2).all(|w| w[1] >= w[0]) && waits[3] > waits[0],
        "backs off: {waits:?}"
    );
    assert_eq!(lab.core.ctx.now(), t0);
    let shown = lab.events_named(runner::EVENT_JOB_STATE).pop().unwrap();
    assert_eq!(shown["state"], "succeeded");
    assert_eq!(shown["error"]["code"], "auth");
    // A failed job can't be fetched again; a succeeded one can, at once.
    lab.set_case("success");
    lab.core
        .dispatch("rgn_job_fetch_again", json!({"job_id": id}))
        .unwrap();
    assert_eq!(runner::tick(&lab.core, false), None, "fetched at once");
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Fetched);
    assert!(job.error.is_none(), "the fetch error is cleared");
    assert_eq!(lab.candidates(id).len(), 3);
    assert_eq!(
        lab.fake().call_count() - calls_before,
        5,
        "four failed fetches and one good one; no submit"
    );
    assert_eq!(
        code(
            lab.core
                .dispatch("rgn_job_fetch_again", json!({"job_id": id}))
        ),
        ErrorCode::InvalidInput,
        "only a succeeded job"
    );
}

/// Review item 6 / R102: non-transient poll errors are counted; after 10 in a row the job goes
/// `lost` with its handle kept (no endless loop that blocks the project). A good poll resets
/// the count.
#[test]
fn fr_rgn_23_ten_poll_errors_in_a_row_end_the_job_as_lost_with_its_handle() {
    let lab = Lab::new();
    let id = lab.submit("kitchen");
    runner::tick(&lab.core, true);
    assert_eq!(lab.job(id).state, JobState::Queued);
    lab.set_case("auth");
    for _ in 0..9 {
        runner::tick(&lab.core, true);
    }
    assert_eq!(
        lab.job(id).state,
        JobState::Queued,
        "9 errors: still polling"
    );
    // One good poll resets the count.
    lab.set_case("slow(30)");
    runner::tick(&lab.core, true);
    lab.set_case("auth");
    for _ in 0..9 {
        runner::tick(&lab.core, true);
    }
    assert!(lab.core.state().rgn.runner.tracked().contains(&id));
    runner::tick(&lab.core, true);
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Lost, "the 10th error in a row ends it");
    assert!(job.has_handle(), "the handle is kept");
    assert_eq!(job.error.as_ref().unwrap()["code"], "auth");
    assert!(lab.core.state().rgn.runner.tracked().is_empty());
    let last = lab.events_named(runner::EVENT_JOB_STATE).pop().unwrap();
    assert_eq!(last["state"], "lost");
    // R108: the strip learns the handle is kept, and the hint, from the iteration row.
    let rows = lab.core.dispatch("rgn_iterations_list", json!({})).unwrap();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["job_id"] == json!(id))
        .unwrap();
    assert_eq!(row["handle_kept"], true);
    assert_eq!(row["error"]["code"], "auth");
    assert!(
        row["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("check the provider")
    );
}

#[test]
fn fr_rgn_21_retry_draft_prefills_with_a_new_seed_unless_pinned() {
    let lab = Lab::new();
    let mut d = lab.draft("kitchen");
    d["controls"] = json!({});
    let id = lab.run("kitchen");
    let draft = lab
        .core
        .dispatch("rgn_job_retry_draft", json!({"job_id": id}))
        .unwrap();
    assert_eq!(draft["final_prompt"], "kitchen");
    assert_eq!(draft["mask_id"], json!(lab.mask_id));
    assert_eq!(draft["model_id"], "fake-white-edit");
    assert_eq!(draft["region"]["inner"]["x"], json!(400.0));
    assert_eq!(draft["region"]["margin"], json!(128));
    assert_eq!(draft["seed"], Value::Null, "random again");

    let mut pinned = lab.draft("pinned");
    pinned["seed"] = json!(77);
    let id2 = lab.submit_draft(pinned).unwrap();
    lab.drive();
    let draft = lab
        .core
        .dispatch("rgn_job_retry_draft", json!({"job_id": id2}))
        .unwrap();
    assert_eq!(draft["seed"], json!(77));
    // Nothing was submitted by the retry draft.
    assert_eq!(lab.manifest()["jobs"].as_array().unwrap().len(), 2);
}

#[test]
fn fr_rgn_21_owner_73_retry_restores_the_blend_toggle_off_the_model_and_the_typed_prompt() {
    let lab = Lab::new();
    let mut d = lab.draft("  a stone well  ");
    d["model_id"] = json!("fake-black-edit");
    d["blend_instruction"] = json!(false);
    let id = lab.submit_draft(d).unwrap();
    lab.drive();
    let draft = lab
        .core
        .dispatch("rgn_job_retry_draft", json!({"job_id": id}))
        .unwrap();
    assert_eq!(draft["model_id"], "fake-black-edit");
    assert_eq!(draft["blend_instruction"], json!(false), "off stays off");
    assert_eq!(
        draft["final_prompt"], "  a stone well  ",
        "the typed prompt, as typed"
    );
    // On is restored as on.
    let id = lab.submit("a kitchen");
    lab.drive();
    let draft = lab
        .core
        .dispatch("rgn_job_retry_draft", json!({"job_id": id}))
        .unwrap();
    assert_eq!(draft["blend_instruction"], json!(true));
    assert_eq!(draft["model_id"], "fake-white-edit");
    assert_eq!(
        draft["final_prompt"], "a kitchen",
        "no blend text folded in"
    );
}

/// Walk owner-UX M1: the count is sent as `candidate_count`, not in the job's `controls`, so
/// Retry must put it back under the count control's name, or it falls back to the default.
#[test]
fn fr_rgn_21_retry_restores_the_candidate_count_the_user_set() {
    let lab = Lab::new();
    let mut d = lab.draft("four");
    d["controls"] = json!({"num_samples": 4});
    let id = lab.submit_draft(d).unwrap();
    lab.drive();
    let job = lab.job(id);
    assert_eq!(job.candidate_count, 4);
    assert!(
        job.controls.is_empty(),
        "the count isn't a sent `other` control"
    );
    let draft = lab
        .core
        .dispatch("rgn_job_retry_draft", json!({"job_id": id}))
        .unwrap();
    assert_eq!(draft["controls"], json!({"num_samples": 4}));
    // The restored draft estimates and submits as the job did.
    let id2 = lab.submit_draft(draft).unwrap();
    assert_eq!(lab.job(id2).candidate_count, 4);
}

/// FR-RGN-11: a remembered count the model now rejects (a single-output model, or a lower
/// maximum) is dropped / reset by `rgn_controls_evaluate`, which drives the panel's notice.
#[test]
fn fr_rgn_11_a_restored_count_the_capabilities_reject_resets_with_a_notice() {
    let lab = Lab::new();
    let mut d = lab.draft("four");
    d["controls"] = json!({"num_samples": 4});
    let id = lab.submit_draft(d).unwrap();
    lab.drive();
    let draft = lab
        .core
        .dispatch("rgn_job_retry_draft", json!({"job_id": id}))
        .unwrap();
    let values: std::collections::BTreeMap<String, Value> =
        serde_json::from_value(draft["controls"].clone()).unwrap();
    let mut caps = pf_provider::block_on(pf_provider::Provider::get_capabilities(
        &FakeProvider::default(),
        "fake-white-edit",
        pf_provider::Operation::EditMasked,
    ))
    .unwrap();
    let mut lower = caps.clone();
    for c in &mut lower.controls {
        if c.role == pf_provider::ControlRole::CandidateCount {
            c.max = Some(3.0);
        }
    }
    let e = super::controls::evaluate_controls(&lower, &values);
    let count = e.controls.iter().find(|c| c.name == "num_samples").unwrap();
    assert!(count.reset, "notice: reset to the default");
    assert_eq!(count.value, Some(json!(3)), "the default");
    caps.controls
        .retain(|c| c.role != pf_provider::ControlRole::CandidateCount);
    let e = super::controls::evaluate_controls(&caps, &values);
    assert_eq!(e.unknown, vec!["num_samples".to_owned()], "notice: dropped");
}

/// Walk owner-UX L2: with the folder moved, estimate and submit refuse with `folder_moved`
/// (readable, no path), not the OS text of the mask read failing; nothing is spent.
#[test]
fn fr_rgn_31_a_moved_folder_refuses_estimate_and_submit_with_folder_moved() {
    let lab = Lab::new();
    let draft = lab.draft("kitchen");
    let est = lab.estimate(&draft);
    crate::test_support::rename_dir(
        &lab.project_dir,
        &lab.project_dir.with_file_name("Elsewhere"),
    );
    for e in [
        lab.core
            .dispatch("rgn_job_estimate", json!({ "draft": draft }))
            .unwrap_err(),
        lab.core
            .dispatch(
                "rgn_job_submit",
                json!({"draft": draft, "estimate_id": est["estimate_id"], "confirmations": est["needs"]}),
            )
            .unwrap_err(),
    ] {
        assert!(crate::project::is_folder_moved(&e), "{e:?}");
        assert!(!e.message.contains("os error"), "{}", e.message);
    }
    assert_eq!(lab.core.state().rgn.ledger.total(), 0.0, "nothing spent");
}

#[test]
fn fr_rgn_38_read_only_project_refuses_submit_and_dismiss() {
    let lab = Lab::new();
    let id = lab.run("kitchen");
    let draft = lab.draft("again");
    let est = lab.estimate(&draft);
    lab.core.state().project.as_mut().unwrap().read_only =
        Some(crate::project::ReadOnlyReason::OriginalChanged);
    let r = lab.core.dispatch(
        "rgn_job_submit",
        json!({"draft": draft, "estimate_id": est["estimate_id"], "confirmations": []}),
    );
    assert_eq!(code(r), ErrorCode::ReadOnly);
    assert_eq!(
        code(lab.core.dispatch("rgn_job_dismiss", json!({"job_id": id}))),
        ErrorCode::ReadOnly
    );
}

#[test]
fn fr_rgn_37_a_conflict_spools_the_candidates_and_replays_after_resolution() {
    let lab = Lab::new();
    let id = lab.submit("kitchen");
    runner::tick(&lab.core, true);
    runner::tick(&lab.core, true);
    runner::tick(&lab.core, true);
    assert_eq!(lab.job(id).state, JobState::Succeeded);
    // Another machine committed: project.json's seq moved on.
    let path = lab.project_dir.join(MANIFEST_FILE);
    let ours = std::fs::read(&path).unwrap();
    let mut theirs: Value = serde_json::from_slice(&ours).unwrap();
    theirs["commit"]["seq"] = json!(theirs["commit"]["seq"].as_u64().unwrap() + 7);
    std::fs::write(&path, serde_json::to_vec_pretty(&theirs).unwrap()).unwrap();
    lab.drive();
    assert_eq!(lab.job(id).state, JobState::Succeeded, "not committed yet");
    assert!(lab.candidates(id).is_empty());
    let pid = lab.manifest()["project_id"].as_str().unwrap().to_owned();
    let spool = lab.dir.path().join("data/pending").join(&pid);
    assert!(spool.join("1.json").is_file(), "paid output spooled");
    // User writes fail with `conflict` meanwhile.
    assert_eq!(
        code(lab.core.dispatch("rgn_job_dismiss", json!({"job_id": id}))),
        ErrorCode::JobActive
    );
    // Resolved by the user (FR-PRJ-36): the resolve runs the runner tick, which replays (R48).
    let rev = lab
        .core
        .dispatch("project_resolve_conflict", json!({"keep": "this"}))
        .unwrap();
    assert_eq!(rev["reason"], "conflict");
    assert_eq!(lab.job(id).state, JobState::Fetched);
    assert_eq!(lab.candidates(id).len(), 3);
    assert!(!spool.exists());
}

// ---------------------------------------------------------------- slice 4: candidates

#[test]
fn fr_rgn_24_39_candidates_land_untouched_in_the_iteration_folder() {
    let lab = Lab::new();
    let id = lab.run("Turn this room into a Kitchen!");
    let job = lab.job(id);
    let it = job.iteration.clone().unwrap();
    assert_eq!(it.n, 1);
    assert_eq!(it.dir, "results/001 turn this room into a kitchen");
    let cands = lab.candidates(id);
    assert_eq!(cands.len(), 3);
    for (i, c) in cands.iter().enumerate() {
        assert_eq!(c.file, format!("{}/{:02}.png", it.dir, i + 1));
        assert_eq!(c.status, CandidateStatus::New);
        assert!(lab.project_dir.join(&c.file).is_file());
        assert_eq!((c.width, c.height), (560, 456));
        assert!(c.provider_meta["asset_id"].is_string());
    }
    assert_eq!(
        job.candidate_ids,
        cands.iter().map(|c| c.id).collect::<Vec<_>>()
    );
    let ready = lab.events_named(runner::EVENT_CANDIDATE_READY);
    assert_eq!(ready.len(), 3);
    assert!(ready.iter().all(|e| e["status"] == "ready"));
    // The next job gets 002.
    let id2 = lab.run("a second idea");
    assert_eq!(
        lab.job(id2).iteration.unwrap().dir,
        "results/002 a second idea"
    );
}

#[test]
fn fr_rgn_24_a_bad_candidate_is_invalid_and_the_rest_stay_usable() {
    let lab = Lab::new();
    lab.set_case("bad_candidate");
    let id = lab.run("kitchen");
    assert_eq!(lab.job(id).state, JobState::Fetched);
    let cands = lab.candidates(id);
    assert_eq!(cands[0].status, CandidateStatus::Invalid);
    assert!(cands[1..].iter().all(|c| c.status == CandidateStatus::New));
    let ready = lab.events_named(runner::EVENT_CANDIDATE_READY);
    assert_eq!(ready[0]["status"], "invalid");
}

#[test]
fn fr_rgn_24_aspect_mismatch_gets_a_warning_badge() {
    let lab = Lab::new();
    lab.set_case("aspect_mismatch(2)");
    let id = lab.run("kitchen");
    let cands = lab.candidates(id);
    assert!(
        cands
            .iter()
            .all(|c| c.extra.get("aspect_warning") == Some(&json!(true)))
    );
    lab.set_case("success");
    let id = lab.run("plain");
    assert!(
        lab.candidates(id)
            .iter()
            .all(|c| !c.extra.contains_key("aspect_warning"))
    );
}

#[test]
fn fr_rgn_39_a_kill_between_files_and_manifest_refetches_into_the_same_folder() {
    let lab = Lab::new();
    let id = lab.submit("kitchen");
    runner::tick(&lab.core, true);
    runner::tick(&lab.core, true);
    runner::tick(&lab.core, true);
    // `add_candidates` wrote `iteration` and the files, then the manifest write was lost.
    let dir = {
        let mut state = lab.core.state();
        let install = state.settings.install_id;
        let p = state.project.as_mut().unwrap();
        let before = std::fs::read(p.dir.join(MANIFEST_FILE)).unwrap();
        let cands = (0..3)
            .map(|_| store::NewCandidate {
                bytes: crate::test_support::png_bytes(560, 456, 9),
                ..Default::default()
            })
            .collect();
        store::add_candidates(&lab.core.ctx, install, p, id, cands).unwrap();
        std::fs::write(p.dir.join(MANIFEST_FILE), before).unwrap();
        lab.project_dir
            .join(p.jobs[&id].iteration.as_ref().unwrap().dir.clone())
    };
    // Make job.json say `succeeded` again with the iteration (the follow-up never ran).
    let mut job = lab.job(id);
    job.state = JobState::Succeeded;
    job.candidate_ids.clear();
    std::fs::write(
        lab.project_dir.join(job_dir(id)).join(JOB_FILE),
        serde_json::to_vec_pretty(&job).unwrap(),
    )
    .unwrap();
    let lab = lab.restart();
    lab.open();
    lab.drive();
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Fetched);
    assert_eq!(
        lab.project_dir.join(job.iteration.unwrap().dir),
        dir,
        "the same folder"
    );
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        3,
        "files replaced, none added"
    );
}

// ---------------------------------------------------------------- slices 4–5: helpers

/// The full flatten of the open project's current stack.
fn flatten(lab: &Lab) -> RgbImage {
    let src = super::pixel_source(&lab.core.ctx, &lab.core.state()).unwrap();
    let stack = src.layer_stack().unwrap();
    let (width, height) = src.image_size();
    pf_imaging::compose_rect(
        &stack,
        pf_imaging::PixelRect {
            x: 0,
            y: 0,
            width,
            height,
        },
    )
    .unwrap()
}

fn blend(feather: u32, colour: bool) -> Value {
    json!({"feather_px": feather, "colour_match": colour})
}

fn accept(lab: &Lab, job: Ulid, cand: Ulid, b: Value) -> Result<Value, CoreError> {
    lab.core.dispatch(
        "rgn_candidate_accept",
        json!({"job_id": job, "candidate_id": cand, "blend": b}),
    )
}

fn snapshot(lab: &Lab) -> Value {
    lab.core.dispatch("project_get", json!({})).unwrap()
}

fn restore(lab: &Lab, rev: &Value) -> Value {
    lab.core
        .dispatch("project_restore_revision", json!({"rev_id": rev}))
        .unwrap()
}

fn layer_ids(lab: &Lab) -> Vec<Value> {
    snapshot(lab)["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["id"].clone())
        .collect()
}

fn last_layer(lab: &Lab) -> Value {
    snapshot(lab)["layers"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone()
}

// ---------------------------------------------------------------- slice 4: preview

#[test]
fn fr_rgn_25_preview_is_baked_and_registered_under_a_transient_key() {
    let lab = Lab::new();
    let id = lab.run("kitchen");
    let c = lab.candidates(id)[0].id;
    let seq = snapshot(&lab)["commit_seq"].clone();
    let out = lab
        .core
        .dispatch(
            "rgn_candidate_preview",
            json!({"job_id": id, "candidate_id": c, "blend": blend(48, true)}),
        )
        .unwrap();
    let key = out["key"].as_str().unwrap();
    assert!(key.starts_with("t-"));
    assert!(
        out["url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/i/{key}.png"))
    );
    // The cropped patch sits inside the region (the painted disc).
    let r = &out["rect"];
    assert!(r["x"].as_u64().unwrap() >= INNER.x as u64);
    assert!(r["w"].as_u64().unwrap() < 300 && r["h"].as_u64().unwrap() < 200);
    let served = lab.core.serve_scheme(&format!("/i/{key}.png"));
    assert_eq!(served.status, 200);
    let png = image::load_from_memory(&served.body).unwrap();
    assert_eq!(
        (png.width(), png.height()),
        (
            r["w"].as_u64().unwrap() as u32,
            r["h"].as_u64().unwrap() as u32
        )
    );
    assert_eq!(
        snapshot(&lab)["commit_seq"],
        seq,
        "a preview writes nothing"
    );
    // Invalid candidates and foreign ids.
    lab.core
        .dispatch("rgn_job_dismiss", json!({"job_id": id}))
        .unwrap();
    lab.set_case("bad_candidate");
    let id2 = lab.run("again");
    let bad = lab.candidates(id2)[0].id;
    let preview = |job: Ulid, cand: Ulid, b: Value| {
        lab.core.dispatch(
            "rgn_candidate_preview",
            json!({"job_id": job, "candidate_id": cand, "blend": b}),
        )
    };
    assert_eq!(
        code(preview(id2, bad, blend(48, true))),
        ErrorCode::InvalidCandidate
    );
    assert_eq!(
        code(preview(id2, c, blend(48, true))),
        ErrorCode::InvalidCandidate
    );
    assert_eq!(
        code(preview(id2, bad, blend(300, true))),
        ErrorCode::InvalidInput
    );
}

#[test]
fn fr_rgn_26_blend_controls_change_the_bake() {
    let lab = Lab::new();
    lab.set_case("offset(12)");
    let id = lab.run("kitchen");
    let c = lab.candidates(id)[0].id;
    let preview = |b: Value| {
        lab.core
            .dispatch(
                "rgn_candidate_preview",
                json!({"job_id": id, "candidate_id": c, "blend": b}),
            )
            .unwrap()
    };
    let hard = preview(blend(0, false));
    let soft = preview(blend(96, true));
    assert_ne!(
        hard["rect"], soft["rect"],
        "a wide feather eats the disc edge"
    );
    assert_eq!(hard["notices"], json!([]));
}

// ---------------------------------------------------------------- slice 5: accept

#[test]
fn fr_rgn_27_accept_is_one_commit_and_leaves_the_original_alone() {
    let lab = Lab::new();
    let id = lab.run("turn this room into a kitchen with a long prompt that goes on");
    let before = flatten(&lab);
    let hash = lab.manifest()["original"]["content_hash"].clone();
    let seq = snapshot(&lab)["commit_seq"].as_u64().unwrap();
    let cands = lab.candidates(id);
    let out = accept(&lab, id, cands[0].id, blend(48, true)).unwrap();
    let snap = snapshot(&lab);
    assert_eq!(snap["commit_seq"].as_u64().unwrap(), seq + 1, "one commit");
    let layer = last_layer(&lab);
    assert_eq!(layer["id"], out["layer_id"]);
    assert_eq!(layer["kind"], "patch");
    assert_eq!(layer["scale"], json!(1.0));
    assert_eq!(layer["rotation"], json!(0.0));
    assert_eq!(layer["opacity"], json!(1.0));
    assert_eq!(layer["visible"], json!(true));
    assert_eq!(layer["job_id"], json!(id));
    assert_eq!(layer["candidate_id"], json!(cands[0].id));
    assert_eq!(layer["blend"]["feather_px"], json!(48));
    assert_eq!(layer["blend"]["colour_match"]["on"], json!(true));
    let rev = snap["revisions"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(rev["id"], out["revision_id"]);
    assert_eq!(rev["reason"], "accept");
    assert_eq!(
        rev["label"],
        json!("turn this room into a kitchen with a lon")
    );
    assert_eq!(lab.job(id).state, JobState::Closed);
    assert!(lab.job(id).resolved_at.is_some());
    assert_eq!(lab.manifest()["original"]["content_hash"], hash);
    let original = original();
    let after = flatten(&lab);
    // Outside the region nothing changed; the patch covers the painted disc.
    let mut changed = 0;
    for (x, y, p) in after.enumerate_pixels() {
        let inside = x >= INNER.x as u32
            && x < (INNER.x as u32 + INNER.w)
            && y >= INNER.y as u32
            && y < (INNER.y as u32 + INNER.h);
        if !inside {
            assert_eq!(p, before.get_pixel(x, y), "({x},{y}) outside the region");
        } else if p != original.get_pixel(x, y) {
            changed += 1;
        }
    }
    assert!(changed > 1000, "the patch is visible ({changed} px)");
    // Reopen: pixel-identical.
    lab.close();
    lab.open();
    assert_eq!(flatten(&lab), after);
}

#[test]
fn fr_rgn_28_owner_56_statuses_on_accept_keep_every_result() {
    let lab = Lab::new();
    let id = lab.run("kitchen");
    let c = lab.candidates(id);
    lab.core
        .dispatch(
            "rgn_candidate_star",
            json!({"job_id": id, "candidate_id": c[1].id, "starred": true}),
        )
        .unwrap();
    accept(&lab, id, c[0].id, blend(96, true)).unwrap();
    let c = lab.candidates(id);
    assert_eq!(c[0].status, CandidateStatus::Accepted);
    assert_eq!(c[1].status, CandidateStatus::Dismissed);
    assert!(c[1].starred && c[1].trash_id.is_none(), "starred stays");
    assert_eq!(c[2].status, CandidateStatus::Dismissed);
    assert!(c[2].trash_id.is_none(), "#56: unstarred stays too");
    let dir = lab.project_dir.join(lab.job(id).iteration.unwrap().dir);
    let names = |dir: &std::path::Path| {
        let mut n: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        n.sort();
        n
    };
    assert_eq!(names(&dir), ["01.png", "02.png", "03.png"]);
    // "Discard the rest": one command, accepted and starred stay.
    let out = lab
        .core
        .dispatch("rgn_job_discard_rest", json!({"job_id": id}))
        .unwrap();
    assert_eq!(out, json!({"trashed": 1}));
    assert_eq!(names(&dir), ["01.png", "02.png"]);
    let c = lab.candidates(id);
    assert_eq!(c[2].status, CandidateStatus::Discarded);
    let trashed = lab
        .project_dir
        .join(".pforge/trash")
        .join(c[2].trash_id.unwrap().to_string());
    assert!(trashed.join(&c[2].file).is_file(), "moved, not deleted");
    // Accepted jobs keep their inputs (R9).
    for f in lab.job(id).input_files() {
        assert!(lab.project_dir.join(f).is_file());
    }
}

#[test]
fn fr_rgn_28_discard_and_star_restore() {
    let lab = Lab::new();
    let id = lab.run("kitchen");
    let c = lab.candidates(id);
    lab.core
        .dispatch(
            "rgn_candidate_discard",
            json!({"job_id": id, "candidate_id": c[2].id}),
        )
        .unwrap();
    let c2 = &lab.candidates(id)[2];
    assert_eq!(c2.status, CandidateStatus::Discarded);
    assert!(c2.trash_id.is_some());
    assert!(!lab.project_dir.join(&c2.file).exists(), "left results/");
    lab.core
        .dispatch(
            "rgn_candidate_star",
            json!({"job_id": id, "candidate_id": c[2].id, "starred": true}),
        )
        .unwrap();
    assert!(
        lab.project_dir.join(&c[2].file).is_file(),
        "starring puts it back"
    );
    // Wrong job → invalid_candidate; unknown → unknown_id.
    assert_eq!(
        code(lab.core.dispatch(
            "rgn_candidate_star",
            json!({"job_id": Ulid::from_parts(1, 999), "candidate_id": c[0].id, "starred": true}),
        )),
        ErrorCode::InvalidCandidate
    );
    assert_eq!(
        code(lab.core.dispatch(
            "rgn_candidate_discard",
            json!({"job_id": id, "candidate_id": Ulid::from_parts(1, 999)}),
        )),
        ErrorCode::UnknownId
    );
}

#[test]
fn fr_rgn_28_owner_56_dismiss_keeps_leftovers_until_all_are_discarded() {
    let lab = Lab::new();
    let id = lab.run("kitchen");
    let out = lab
        .core
        .dispatch("rgn_job_dismiss", json!({"job_id": id}))
        .unwrap();
    assert_eq!(out, json!({"trashed": 0}), "#56: nothing trashed");
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Closed);
    assert!(job.resolved_at.is_some());
    for c in lab.candidates(id) {
        assert_eq!((c.status, c.trash_id), (CandidateStatus::Dismissed, None));
        assert!(lab.project_dir.join(&c.file).is_file());
    }
    for f in job.input_files() {
        assert!(lab.project_dir.join(f).is_file(), "inputs kept");
    }
    let out = lab
        .core
        .dispatch("rgn_job_discard_rest", json!({"job_id": id}))
        .unwrap();
    assert_eq!(out, json!({"trashed": 3}));
    for f in job.input_files() {
        assert!(!lab.project_dir.join(f).exists(), "R9: inputs trashed");
    }
    assert!(
        !lab.project_dir.join(job.iteration.unwrap().dir).exists(),
        "an emptied iteration folder is removed"
    );
    // A running job can't be dismissed.
    let id = lab.submit("again");
    assert_eq!(
        code(lab.core.dispatch("rgn_job_dismiss", json!({"job_id": id}))),
        ErrorCode::JobActive
    );
}

#[test]
fn fr_rgn_28_owner_56_the_next_submit_resolves_the_previous_job_and_keeps_it() {
    let lab = Lab::new();
    let first = lab.run("kitchen");
    let c = lab.candidates(first);
    lab.core
        .dispatch(
            "rgn_candidate_star",
            json!({"job_id": first, "candidate_id": c[0].id, "starred": true}),
        )
        .unwrap();
    lab.submit("second");
    let job = lab.job(first);
    assert_eq!(job.state, JobState::Closed);
    assert!(job.resolved_at.is_some());
    let c = lab.candidates(first);
    assert!(c[0].starred && c[0].trash_id.is_none());
    assert!(
        c[1..]
            .iter()
            .all(|c| c.status == CandidateStatus::Dismissed && c.trash_id.is_none()),
        "#56: the rest stays in results/"
    );
}

#[test]
fn fr_rgn_28_owner_67_an_accepted_candidate_cant_be_discarded() {
    let lab = Lab::new();
    let id = lab.run("kitchen");
    let c = lab.candidates(id);
    accept(&lab, id, c[0].id, blend(96, true)).unwrap();
    let before = lab.manifest();
    let err = lab
        .core
        .dispatch(
            "rgn_candidate_discard",
            json!({"job_id": id, "candidate_id": c[0].id}),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(err.field.as_deref(), Some("candidate_id"));
    let c0 = &lab.candidates(id)[0];
    assert_eq!((c0.status, c0.trash_id), (CandidateStatus::Accepted, None));
    assert!(lab.project_dir.join(&c0.file).is_file(), "file stays");
    assert_eq!(lab.manifest(), before, "nothing committed");
}

#[test]
fn fr_rgn_28_explicit_discard_of_a_starred_candidate_clears_its_star() {
    let lab = Lab::new();
    let id = lab.run("kitchen");
    let c = lab.candidates(id);
    lab.core
        .dispatch(
            "rgn_candidate_star",
            json!({"job_id": id, "candidate_id": c[1].id, "starred": true}),
        )
        .unwrap();
    assert!(lab.candidates(id)[1].starred);
    lab.core
        .dispatch(
            "rgn_candidate_discard",
            json!({"job_id": id, "candidate_id": c[1].id}),
        )
        .unwrap();
    let c1 = &lab.candidates(id)[1];
    assert_eq!(c1.status, CandidateStatus::Discarded);
    assert!(!c1.starred, "the star is cleared");
    assert!(c1.trash_id.is_some());
    assert!(!lab.project_dir.join(&c1.file).exists(), "left results/");
    // On disk too.
    let row = lab.manifest()["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == json!(c[1].id))
        .cloned()
        .unwrap();
    assert_eq!(row["starred"], json!(false));
    assert_eq!(row["status"], json!("discarded"));
}

#[test]
fn fr_rgn_28_owner_66_invalid_candidates_count_as_discarded() {
    let lab = Lab::new();
    lab.set_case("bad_candidate");
    let id = lab.run("kitchen");
    let c = lab.candidates(id);
    assert_eq!(c[0].status, CandidateStatus::Invalid);
    lab.core
        .dispatch("rgn_job_dismiss", json!({"job_id": id}))
        .unwrap();
    let inputs = lab.job(id).input_files();
    assert!(!inputs.is_empty());
    lab.core
        .dispatch(
            "rgn_candidate_discard",
            json!({"job_id": id, "candidate_id": c[1].id}),
        )
        .unwrap();
    for f in &inputs {
        assert!(
            lab.project_dir.join(f).is_file(),
            "one candidate still kept"
        );
    }
    lab.core
        .dispatch(
            "rgn_candidate_discard",
            json!({"job_id": id, "candidate_id": c[2].id}),
        )
        .unwrap();
    // Only the invalid one is left; it doesn't keep the inputs (R66).
    assert_eq!(lab.candidates(id)[0].status, CandidateStatus::Invalid);
    for f in &inputs {
        assert!(!lab.project_dir.join(f).exists(), "R66: inputs trashed");
    }
}

#[test]
fn fr_rgn_28_owner_66_a_job_with_no_candidates_trashes_its_inputs_on_resolve() {
    let lab = Lab::new();
    lab.set_case("failure");
    let id = lab.run("kitchen");
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Failed);
    assert!(lab.candidates(id).is_empty());
    let inputs = job.input_files();
    assert!(!inputs.is_empty());
    for f in &inputs {
        assert!(lab.project_dir.join(f).is_file(), "kept until resolved");
    }
    lab.core
        .dispatch("rgn_job_dismiss", json!({"job_id": id}))
        .unwrap();
    assert!(lab.job(id).resolved_at.is_some());
    for f in &inputs {
        assert!(!lab.project_dir.join(f).exists(), "R66: trashed on resolve");
    }
    let trash = lab.manifest()["trash"].clone();
    assert!(
        trash.as_array().unwrap().iter().any(|t| t["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == &json!(inputs[0]))),
        "the inputs are a trash item, not deleted"
    );
}

#[test]
fn fr_rgn_29_30_retune_and_undo_redo() {
    let lab = Lab::new();
    let id = lab.run("kitchen");
    let c = lab.candidates(id);
    let r0 = snapshot(&lab)["head_revision"].clone();
    let acc = accept(&lab, id, c[0].id, blend(96, true)).unwrap();
    let accepted = flatten(&lab);
    let old_file = lab.project_dir.join(format!(
        ".pforge/layers/{}.png",
        acc["layer_id"].as_str().unwrap()
    ));
    let ret = lab
        .core
        .dispatch(
            "rgn_patch_retune",
            json!({"layer_id": acc["layer_id"], "blend": blend(32, true)}),
        )
        .unwrap();
    assert_ne!(ret["layer_id"], acc["layer_id"], "a new layer id");
    let snap = snapshot(&lab);
    let layer = last_layer(&lab);
    assert_eq!(layer["id"], ret["layer_id"]);
    assert_eq!(layer["retune_of"], acc["layer_id"]);
    assert_eq!(layer["blend"]["feather_px"], json!(32));
    assert_eq!(
        snap["layers"].as_array().unwrap().len(),
        2,
        "replaced, not added"
    );
    assert_eq!(
        snap["revisions"].as_array().unwrap().last().unwrap()["reason"],
        "retune"
    );
    assert!(
        old_file.is_file(),
        "the old file stays, referenced by its revision"
    );
    assert_ne!(flatten(&lab), accepted);
    // Undo the re-tune = restore its parent (the accept revision); redo = restore the re-tune.
    restore(&lab, &acc["revision_id"]);
    assert_eq!(flatten(&lab), accepted);
    restore(&lab, &ret["revision_id"]);
    assert_eq!(layer_ids(&lab).last().unwrap(), &ret["layer_id"]);
    // Undo the accept: back to r0's stack. Candidate and job records are unchanged.
    restore(&lab, &r0);
    assert_eq!(layer_ids(&lab).len(), 1);
    assert_eq!(lab.candidates(id)[0].status, CandidateStatus::Accepted);
    assert_eq!(lab.job(id).state, JobState::Closed);
    assert!(old_file.is_file(), "undo never deletes a file");
    let reasons: Vec<String> = snapshot(&lab)["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["reason"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        reasons,
        [
            "create", "accept", "retune", "restore", "restore", "restore"
        ]
    );
    // Close + reopen keeps the blend values (walkthrough 6).
    restore(&lab, &ret["revision_id"]);
    lab.close();
    lab.open();
    assert_eq!(last_layer(&lab)["blend"]["feather_px"], json!(32));
}

#[test]
fn fr_rgn_36_repick_after_undo_including_a_trashed_candidate() {
    let lab = Lab::new();
    let id = lab.run("kitchen");
    let c = lab.candidates(id);
    let r0 = snapshot(&lab)["head_revision"].clone();
    accept(&lab, id, c[0].id, blend(48, true)).unwrap();
    // #56: only an explicit discard sends one to the trash.
    assert!(lab.candidates(id)[2].trash_id.is_none());
    lab.core
        .dispatch(
            "rgn_candidate_discard",
            json!({"job_id": id, "candidate_id": c[2].id}),
        )
        .unwrap();
    assert!(lab.candidates(id)[2].trash_id.is_some());
    let spend = lab.core.dispatch("rgn_spend_get", json!({})).unwrap()["spend"].clone();
    // Not while its patch is in the stack.
    assert_eq!(
        code(accept(&lab, id, c[2].id, blend(48, true))),
        ErrorCode::RepickUnavailable
    );
    restore(&lab, &r0);
    let out = accept(&lab, id, c[2].id, blend(48, true)).unwrap();
    let after = lab.candidates(id);
    assert_eq!(after[2].status, CandidateStatus::Accepted);
    assert!(
        after[2].trash_id.is_none(),
        "restored from the trash in the same commit"
    );
    assert!(lab.project_dir.join(&after[2].file).is_file());
    assert_eq!(
        after[0].status,
        CandidateStatus::Accepted,
        "earlier accept keeps its status"
    );
    assert_eq!(lab.job(id).state, JobState::Closed);
    let layer = last_layer(&lab);
    assert_eq!(layer["id"], out["layer_id"]);
    assert_eq!(layer["candidate_id"], json!(c[2].id));
    assert_eq!(
        lab.core.dispatch("rgn_spend_get", json!({})).unwrap()["spend"],
        spend,
        "no provider call, no spend"
    );
    // A newer job ends re-pick.
    restore(&lab, &r0);
    lab.run("a newer job");
    assert_eq!(
        code(accept(&lab, id, c[1].id, blend(48, true))),
        ErrorCode::RepickUnavailable
    );
}

#[test]
fn fr_rgn_38_read_only_refuses_every_write_fast() {
    let lab = Lab::new();
    let id = lab.run("kitchen");
    let c = lab.candidates(id)[0].id;
    let acc = accept(&lab, id, c, blend(48, true)).unwrap();
    lab.core.state().project.as_mut().unwrap().read_only =
        Some(crate::project::ReadOnlyReason::OriginalChanged);
    let calls = [
        (
            "rgn_candidate_accept",
            json!({"job_id": id, "candidate_id": c, "blend": blend(48, true)}),
        ),
        (
            "rgn_candidate_star",
            json!({"job_id": id, "candidate_id": c, "starred": true}),
        ),
        (
            "rgn_candidate_discard",
            json!({"job_id": id, "candidate_id": c}),
        ),
        (
            "rgn_patch_retune",
            json!({"layer_id": acc["layer_id"], "blend": blend(12, true)}),
        ),
        ("rgn_job_dismiss", json!({"job_id": id})),
        ("rgn_job_estimate", json!({"draft": lab.draft("x")})),
    ];
    for (name, input) in calls {
        assert_eq!(
            code(lab.core.dispatch(name, input)),
            ErrorCode::ReadOnly,
            "{name}"
        );
    }
    // Viewing still works.
    assert!(lab.core.dispatch("rgn_iterations_list", json!({})).is_ok());
}

#[test]
fn fr_rgn_40_iterations_list_newest_first_with_the_active_job() {
    let lab = Lab::new();
    let first = lab.run("kitchen");
    let c = lab.candidates(first);
    accept(&lab, first, c[0].id, blend(48, true)).unwrap();
    let second = lab.run("a second idea");
    let third = lab.submit("third");
    let rows = lab.core.dispatch("rgn_iterations_list", json!({})).unwrap();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["job_id"], json!(third));
    assert_eq!(rows[0]["n"], Value::Null);
    assert_eq!(rows[0]["state"], "submitted");
    assert_eq!(rows[1]["job_id"], json!(second));
    assert_eq!(rows[1]["n"], json!(2));
    assert_eq!(rows[1]["dir"], "results/002 a second idea");
    assert_eq!(rows[1]["slug"], "a second idea");
    assert_eq!(rows[1]["returned"], json!(3));
    assert_eq!(
        rows[1]["kept"],
        json!(3),
        "resolved by the third submit, all kept (#56)"
    );
    assert_eq!(rows[1]["accepted_in_stack"], json!(false));
    assert_eq!(rows[2]["job_id"], json!(first));
    assert_eq!(rows[2]["kept"], json!(3));
    assert_eq!(rows[2]["accepted_in_stack"], json!(true));
    assert_eq!(rows[2]["cost"]["amount"], json!(3.0));
    assert_eq!(rows[2]["references"], json!(0));
    lab.close();
    assert_eq!(
        code(lab.core.dispatch("rgn_iterations_list", json!({}))),
        ErrorCode::NoProject
    );
}

/// The seam fixture: texture noise in mid tones (nothing clamps under the fake's `offset(12)`),
/// tall enough that the per-column means average out the fake's per-pixel edit noise.
const SEAM_H: u32 = 2000;
const SEAM_INNER: Rect = Rect {
    x: 400,
    y: 200,
    w: 300,
    h: 1600,
};

fn seam_fixture() -> RgbImage {
    RgbImage::from_fn(W, SEAM_H, |x, y| {
        let h =
            (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)).wrapping_mul(2_654_435_761);
        let n = (h >> 24) as u8 % 48;
        Rgb([70 + n, 90 + n / 2, 110 + n / 3])
    })
}

#[test]
fn fr_rgn_32_no_visible_seam_with_offset_12() {
    let lab = Lab::with_original(seam_fixture());
    let inner = SEAM_INNER;
    // A hard vertical edge in the middle of the region: left half kept, right half edited.
    let mask: Vec<u8> = (0..inner.w * inner.h)
        .map(|i| if i % inner.w >= inner.w / 2 { 255 } else { 0 })
        .collect();
    let mask_id = lab.put_mask(inner, &mask);
    let mut draft = lab.draft("kitchen");
    draft["mask_id"] = json!(mask_id);
    draft["region"]["inner"] = json!({"x": inner.x, "y": inner.y, "w": inner.w, "h": inner.h});
    lab.set_case("offset(12)");
    let id = lab.submit_draft(draft).unwrap();
    lab.drive();
    let c = lab.candidates(id)[0].clone();
    let src = seam_fixture();
    // Both mask edges: the kept/edited boundary and the edited region's far side (BL-33).
    let edges = [inner.x as u32 + inner.w / 2, inner.x as u32 + inner.w];
    let edge = edges[0];
    let (y0, y1) = (inner.y as u32, inner.y as u32 + inner.h);
    // The metric (R47, R68): mean (out − src) luma per column over ±16 px of an edge, smoothed
    // by a 3-px moving average, then the largest step between neighbouring columns. Unsmoothed,
    // the fake's in-mask noise (0..15) and u8 rounding make the max step a coin flip around 0.5
    // on either edge (BL-33 sweep: both edges mean 0.52, 32/60 seeds over); smoothed, 0.35–0.39.
    let step_at = |img: &RgbImage, edge: u32| {
        let luma = |p: &Rgb<u8>| {
            0.299 * f64::from(p.0[0]) + 0.587 * f64::from(p.0[1]) + 0.114 * f64::from(p.0[2])
        };
        let profile = (edge - 16..edge + 16)
            .map(|x| {
                (y0..y1)
                    .map(|y| luma(img.get_pixel(x, y)) - luma(src.get_pixel(x, y)))
                    .sum::<f64>()
                    / f64::from(y1 - y0)
            })
            .collect::<Vec<f64>>();
        profile
            .windows(3)
            .map(|q| (q[0] + q[1] + q[2]) / 3.0)
            .collect::<Vec<f64>>()
            .windows(2)
            .map(|q| (q[1] - q[0]).abs())
            .fold(0.0, f64::max)
    };

    // A hard edge without colour match shows the seam, smoothing or not.
    let r0 = snapshot(&lab)["head_revision"].clone();
    accept(&lab, id, c.id, blend(0, false)).unwrap();
    let out = flatten(&lab);
    let hard = edges.map(|e| step_at(&out, e));
    assert!(
        hard.iter().all(|&h| h > 5.0),
        "the fixture does produce a seam ({hard:?})"
    );
    restore(&lab, &r0);

    accept(&lab, id, c.id, blend(48, true)).unwrap();
    let out = flatten(&lab);
    let steps = edges.map(|e| step_at(&out, e));
    for (e, step) in edges.iter().zip(steps) {
        assert!(step <= 0.5, "mean edge step {step} levels/px at x = {e}");
    }
    // The residual offset: the colour-matched candidate against the source on the context.
    let layer = last_layer(&lab);
    let g = &layer["blend"]["colour_match"]["gain"];
    let o = &layer["blend"]["colour_match"]["offset"];
    let cand = image::open(lab.project_dir.join(&c.file))
        .unwrap()
        .to_rgb8();
    let outer: super::fit::RectPx =
        serde_json::from_value(lab.job(id).region["outer"].clone()).unwrap();
    let (mut sum, mut n) = (0.0, 0.0);
    for y in 0..outer.h {
        for x in 0..outer.w {
            let (ix, iy) = (outer.x + x, outer.y + y);
            let in_mask = ix >= edge && ix < inner.x as u32 + inner.w && iy >= y0 && iy < y1;
            if in_mask {
                continue;
            }
            for ch in 0..3 {
                let v = f64::from(cand.get_pixel(x, y).0[ch]) * g[ch].as_f64().unwrap()
                    + o[ch].as_f64().unwrap();
                sum += v - f64::from(src.get_pixel(ix, iy).0[ch]);
                n += 1.0;
            }
        }
    }
    let residual = (sum / n).abs();
    eprintln!("FR-RGN-32: hard {hard:.2?}, steps {steps:.3?} levels/px, residual {residual:.3}");
    assert!(residual <= 0.1, "residual offset {residual} levels");
}

#[test]
fn fr_prj_29_r109_a_submit_refused_as_locked_leaves_no_job_file() {
    use crate::context::{FaultAction, Faults};
    let mut lab = Lab::new();
    let set_faults = |lab: &mut Lab, f: Faults| {
        std::sync::Arc::get_mut(&mut lab.core.ctx).unwrap().faults = f;
    };
    set_faults(
        &mut lab,
        Faults::parse("after_temp:project.json", FaultAction::Locked).unwrap(),
    );
    let draft = lab.draft("moss");
    let err = lab.submit_draft(draft).unwrap_err();
    assert_eq!(err.code, ErrorCode::Locked);
    let jobs = lab.project_dir.join(crate::manifest::JOBS_DIR);
    let left: Vec<_> = std::fs::read_dir(&jobs)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert!(left.is_empty(), "nothing left behind: {left:?}");
    // After a restart well past the lost window, no "may still run" notice for it.
    set_faults(&mut lab, Faults::none());
    lab.close();
    let lab = lab.restart();
    lab.clock.advance(std::time::Duration::from_secs(86_400));
    let snap = lab.open();
    let lost = snap["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["code"] == "lost");
    assert!(!lost, "{snap}");
    assert!(lab.core.state().project.as_ref().unwrap().jobs.is_empty());
}

/// R109: a paid job whose fetch keeps failing can be dismissed after at least one failed fetch.
/// It ends `failed` with `fetch_failed`, its handle kept, and is never fetched or resubmitted.
#[test]
fn fr_rgn_28_r109_dismiss_after_a_failed_fetch_fails_the_job_and_keeps_the_handle() {
    let lab = Lab::new();
    let id = lab.submit("kitchen");
    for _ in 0..3 {
        runner::tick(&lab.core, true);
    }
    assert_eq!(lab.job(id).state, JobState::Succeeded);
    // No failed fetch yet: still being fetched.
    assert_eq!(
        code(lab.core.dispatch("rgn_job_dismiss", json!({"job_id": id}))),
        ErrorCode::JobActive
    );
    let handle = lab.job(id).provider.clone();
    lab.set_case("auth");
    runner::tick(&lab.core, true);
    assert_eq!(lab.job(id).error.as_ref().unwrap()["code"], "auth");
    lab.core
        .dispatch("rgn_job_dismiss", json!({"job_id": id}))
        .unwrap();
    let job = lab.job(id);
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.error.as_ref().unwrap()["code"], "fetch_failed");
    assert_eq!(job.provider, handle, "the handle is kept");
    assert!(job.resolved_at.is_some());
    assert!(!lab.core.state().rgn.runner.tracked().contains(&id));
    let shown = lab.events_named(runner::EVENT_JOB_STATE).pop().unwrap();
    assert_eq!(
        (&shown["state"], &shown["error"]["code"]),
        (&json!("failed"), &json!("fetch_failed"))
    );
    // Never fetched again, never resubmitted; the project takes a new submit.
    lab.set_case("success");
    let calls = lab.fake().call_count();
    for _ in 0..3 {
        runner::tick(&lab.core, true);
    }
    assert_eq!(lab.fake().call_count(), calls);
    assert_eq!(
        code(
            lab.core
                .dispatch("rgn_job_fetch_again", json!({"job_id": id}))
        ),
        ErrorCode::InvalidInput
    );
    let next = lab.run("pantry");
    assert_eq!(lab.job(next).state, JobState::Fetched);
    lab.close();
    let lab = lab.restart();
    lab.open();
    assert_eq!(lab.job(id).state, JobState::Failed);
    assert!(!lab.core.state().rgn.runner.tracked().contains(&id));
}

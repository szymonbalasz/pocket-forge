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

fn context(
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
        let dir = tempfile::tempdir().unwrap();
        let events = Arc::new(RecordedEvents::default());
        let clock = Arc::new(ManualClock::new(Timestamp(T0)));
        let core = Core::new(context(dir.path(), events.clone(), clock.clone(), 1)).unwrap();
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
        let provider = std::mem::replace(&mut self.core.ctx.provider, AnyProvider::fake());
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
fn fr_rgn_09_the_provider_receives_the_typed_prompt_byte_for_byte() {
    let lab = Lab::new();
    let prompt = "  Turn this room into a kitchen — ünïcode, trailing  ";
    let id = lab.submit(prompt);
    let job = lab.job(id);
    assert_eq!(job.final_prompt, prompt);
    assert_eq!(job.provider["request"]["prompt"], json!(prompt));
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
    // Resolved: the next tick replays.
    std::fs::write(&path, &ours).unwrap();
    runner::tick(&lab.core, false);
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

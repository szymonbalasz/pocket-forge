//! PRM slice 5: the chat core on RGN's lab project (the rooms-like fixture with a stored mask).
//! Fake chat backend by default (deterministic context: inline turns, a fake clock); the Claude
//! Code adapter runs over a replay spawner. No test starts a process or calls the provider.

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use pf_chat::backend::BackendMode;
use pf_chat::process::{Captured, ChildProcess, CommandSpec, Spawner};
use pf_chat::replay::{ReplaySpawner, Script};
use serde_json::{Value, json};
use ulid::Ulid;

use super::thread::{self, Thread, TurnState};
use crate::error::{CoreError, ErrorCode};
use crate::region_edit::job_tests::{INNER, Lab};

const SUCCESS_IMAGE: &str =
    include_str!("../../../../../fixtures/chat/claude-code/success-image.jsonl");
const TWO_PROPOSALS: &str =
    include_str!("../../../../../fixtures/chat/claude-code/two-proposals.jsonl");

fn pid(lab: &Lab) -> Ulid {
    lab.core
        .state()
        .project
        .as_ref()
        .unwrap()
        .manifest
        .project_id
}

fn consent(lab: &Lab) -> Value {
    lab.core
        .dispatch("prm_chat_consent", json!({"accept": true}))
        .unwrap()
}

fn lab() -> Lab {
    let lab = Lab::new();
    consent(&lab);
    lab
}

fn publish(lab: &Lab, host: &str, input: Value) {
    lab.core
        .dispatch(
            "prm_context_publish",
            json!({"project_id": pid(lab), "host": host, "input": input}),
        )
        .unwrap();
}

fn send(lab: &Lab, text: &str) -> Result<Value, CoreError> {
    lab.core.dispatch(
        "prm_chat_send",
        json!({"project_id": pid(lab), "text": text}),
    )
}

fn thread_view(lab: &Lab) -> Value {
    lab.core
        .dispatch("prm_chat_thread_get", json!({"project_id": pid(lab)}))
        .unwrap()
}

fn events(lab: &Lab, name: &str) -> Vec<Value> {
    lab.events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(n, _)| n == name)
        .map(|(_, v)| v.clone())
        .collect()
}

fn code(r: Result<Value, CoreError>) -> ErrorCode {
    r.unwrap_err().code
}

/// Panics on any use: FR-PRM-20's "no process before consent".
struct PanicSpawner;

impl Spawner for PanicSpawner {
    fn is_file(&self, _: &Path) -> bool {
        panic!("the chat looked for Claude Code before consent")
    }
    fn run(&self, _: &CommandSpec, _: Duration) -> io::Result<Captured> {
        panic!("the chat ran Claude Code before consent")
    }
    fn spawn(&self, _: &CommandSpec) -> io::Result<Box<dyn ChildProcess>> {
        panic!("the chat started Claude Code before consent")
    }
    fn open_console(&self, _: &CommandSpec) -> io::Result<()> {
        panic!("the chat opened a console before consent")
    }
}

#[test]
fn fr_prm_20_nothing_starts_before_consent() {
    let lab = Lab::with_context(|ctx| {
        ctx.chat.mode = BackendMode::ClaudeCode;
        ctx.chat.spawner = Arc::new(PanicSpawner);
    });
    let st = lab
        .core
        .dispatch("prm_chat_status", json!({"refresh": true}))
        .unwrap();
    assert_eq!(st["backend"]["state"], "not_checked");
    assert_eq!(
        (st["consent"].clone(), st["enabled"].clone()),
        (json!(false), json!(false))
    );
    assert_eq!(st["mode"], "claude-code");
    // "Not now" changes nothing and starts nothing.
    let st = lab
        .core
        .dispatch("prm_chat_consent", json!({"accept": false}))
        .unwrap();
    assert_eq!(st["backend"]["state"], "not_checked");
    // Turning the chat on in Settings without consent still starts nothing.
    let st = lab
        .core
        .dispatch("prm_chat_settings_put", json!({"enabled": true}))
        .unwrap();
    assert_eq!(st["backend"]["state"], "not_checked");
    publish(&lab, "rgn", lab.draft("a kitchen"));
    assert_eq!(code(send(&lab, "hi")), ErrorCode::ConsentRequired);
    assert_eq!(
        code(lab.core.dispatch("prm_chat_logout", json!({}))),
        ErrorCode::ConsentRequired
    );
    assert_eq!(
        super::open_login_console(&lab.core).unwrap_err().code,
        ErrorCode::ConsentRequired
    );
    // Settings can't grant consent.
    let s = crate::settings::section_get(&lab.core.state().settings, "prompt_chat").unwrap();
    assert!(s.get("consent").is_none());
}

#[test]
fn fr_prm_20_consent_then_the_off_switch() {
    let lab = Lab::new();
    let st = consent(&lab);
    assert_eq!(
        (st["consent"].clone(), st["enabled"].clone()),
        (json!(true), json!(true))
    );
    assert_eq!(st["backend"]["state"], "ok");
    assert_eq!(st["fake_case"], "success");
    let stored = lab.core.state().settings.prompt_chat.clone();
    assert_eq!(
        stored.consent.unwrap().notice_version,
        super::NOTICE_VERSION
    );

    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "make this a kitchen").unwrap();
    let st = lab
        .core
        .dispatch("prm_chat_settings_put", json!({"enabled": false}))
        .unwrap();
    assert_eq!(st["backend"]["state"], "not_checked");
    assert!(!lab.core.chat.has_session(), "off ends the process");
    assert_eq!(code(send(&lab, "again")), ErrorCode::ChatDisabled);
    // The thread is kept.
    assert_eq!(
        thread_view(&lab)["thread"]["turns"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // A consent for an older notice asks again.
    {
        let mut st = lab.core.state();
        st.settings.prompt_chat.enabled = true;
        st.settings
            .prompt_chat
            .consent
            .as_mut()
            .unwrap()
            .notice_version = 0;
    }
    assert_eq!(code(send(&lab, "again")), ErrorCode::ConsentRequired);
}

#[test]
fn fr_prm_27_send_streams_and_records_the_turns() {
    let lab = lab();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    let out = send(&lab, "  make this a kitchen ").unwrap();
    assert_eq!(out["host"], "rgn");
    assert_eq!(out["fresh_session"], true);
    assert_eq!(out["saved"], true);
    assert!(
        out["context_summary"]
            .as_str()
            .unwrap()
            .starts_with("crop ")
    );
    assert!(
        out["context_summary"]
            .as_str()
            .unwrap()
            .contains("+ mask · prompt · fake-white-edit")
    );

    let turn_id = out["turn_id"].clone();
    let deltas = events(&lab, super::EVENT_DELTA);
    assert!(!deltas.is_empty() && deltas.iter().all(|d| d["turn_id"] == turn_id));
    let ends = events(&lab, super::EVENT_TURN);
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0]["state"], "done");
    assert_eq!(ends[0]["proposals"].as_array().unwrap().len(), 1);
    assert_eq!(
        ends[0]["usage"]["notional"], true,
        "the fake bills nothing (L4)"
    );

    let view = thread_view(&lab);
    assert!(view["running_turn_id"].is_null());
    assert_eq!(view["current_host"], "rgn");
    let t = &view["thread"];
    assert_eq!(t["backend"], "fake");
    let turns = t["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(
        (turns[0]["role"].clone(), turns[0]["state"].clone()),
        (json!("user"), json!("done"))
    );
    assert_eq!(turns[0]["text"], "make this a kitchen");
    assert_eq!(turns[0]["host"], "rgn");
    assert_eq!(turns[0]["fresh_session"], true);
    let c = &turns[0]["context"];
    assert_eq!(c["images"][0]["id"], "rgn:overlay");
    assert_eq!(c["model_id"], "fake-white-edit");
    assert!(c["final_prompt"].as_str().unwrap().starts_with("a kitchen"));
    let reply = &turns[1];
    assert_eq!(
        (reply["role"].clone(), reply["state"].clone()),
        (json!("assistant"), json!("done"))
    );
    let text = reply["text"].as_str().unwrap();
    let streamed: String = deltas.iter().map(|d| d["text"].as_str().unwrap()).collect();
    assert_eq!(streamed, text);
    assert!(text.contains("You wrote: \"make this a kitchen\""));
    assert!(text.contains("Host: Region edit"));
    assert!(
        text.contains("- rgn:overlay "),
        "the fake lists the image it got"
    );
    assert_eq!(reply["proposals"][0]["text"], "make this a kitchen");

    // The thread file holds no image bytes or session ids (§8).
    let id: Ulid = t["id"].as_str().unwrap().parse().unwrap();
    let raw = std::fs::read_to_string(thread::thread_path(&lab.project_dir, id)).unwrap();
    assert!(!raw.contains("/9j/") && !raw.contains("session_id") && !raw.contains("<session"));
    // FR-PRM-34: the turn's images sit in <data-dir>/chat/<thread-id>/.
    let cwd = lab.core.ctx.data_dir.join("chat").join(id.to_string());
    assert_eq!(std::fs::read_dir(&cwd).unwrap().count(), 1);
}

/// FR-PRM-20 (L2, L6): a settings, consent, logout or case change emits the whole status, so
/// Settings and the window follow it without a refocus.
#[test]
fn fr_prm_20_status_changes_are_announced() {
    let lab = lab();
    let last = |lab: &Lab| events(lab, super::EVENT_STATUS).last().cloned().unwrap();
    assert_eq!(last(&lab)["enabled"], true, "consent turns the chat on");
    lab.core
        .dispatch("prm_chat_settings_put", json!({"enabled": false}))
        .unwrap();
    assert_eq!(last(&lab)["enabled"], false);
    assert_eq!(last(&lab)["backend"]["state"], "not_checked");
    lab.core
        .dispatch("prm_chat_settings_put", json!({"enabled": true}))
        .unwrap();
    lab.core.dispatch("prm_chat_logout", json!({})).unwrap();
    assert_eq!(
        last(&lab)["backend"]["code"],
        "not_logged_in",
        "the fake logs out"
    );
    super::open_login_console(&lab.core).unwrap();
    lab.core
        .dispatch("prm_chat_fake_set_case", json!({"case": "offline"}))
        .unwrap();
    assert_eq!(last(&lab)["fake_case"], "offline");
}

/// §7 / R94: on a read-only project the chat works with the turns kept in memory, shown as
/// "not saved"; nothing is written, and the memory goes with the project.
#[test]
fn fr_prm_22_read_only_project_keeps_turns_in_memory() {
    let lab = lab();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "first, saved").unwrap();
    let saved_view = thread_view(&lab);
    let id: Ulid = saved_view["thread"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let path = thread::thread_path(&lab.project_dir, id);
    let before = std::fs::read(&path).unwrap();
    lab.core.state().project.as_mut().unwrap().read_only =
        Some(crate::project::ReadOnlyReason::NewerVersion);

    let out = send(&lab, "second, in memory").unwrap();
    assert_eq!(out["saved"], false);
    assert_eq!(
        events(&lab, super::EVENT_TURN).last().unwrap()["saved"],
        false
    );
    let view = thread_view(&lab);
    let turns = view["thread"]["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 4, "the saved turns, then the new ones");
    assert_eq!(turns[2]["text"], "second, in memory");
    assert_eq!(turns[3]["state"], "done");
    assert_eq!(turns[3]["proposals"].as_array().unwrap().len(), 1);
    assert_eq!(
        view["unsaved_turn_ids"],
        json!([turns[2]["id"], turns[3]["id"]])
    );
    // A second turn continues the same memory thread.
    send(&lab, "third").unwrap();
    assert_eq!(
        thread_view(&lab)["unsaved_turn_ids"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(std::fs::read(&path).unwrap(), before, "nothing is written");

    // Closing the project drops the memory.
    lab.core.chat.on_project_close(pid(&lab));
    let view = thread_view(&lab);
    assert_eq!(view["thread"]["turns"].as_array().unwrap().len(), 2);
    assert_eq!(view["unsaved_turn_ids"], json!([]));
}

#[test]
fn fr_prm_27_send_needs_a_block_and_one_turn_at_a_time() {
    let lab = lab();
    assert_eq!(code(send(&lab, "hi")), ErrorCode::NoPromptBlock);
    publish(&lab, "rgn", lab.draft("a kitchen"));
    lab.core
        .dispatch(
            "prm_context_clear",
            json!({"project_id": pid(&lab), "host": "rgn"}),
        )
        .unwrap();
    assert_eq!(code(send(&lab, "hi")), ErrorCode::NoPromptBlock);
    publish(&lab, "rgn", lab.draft("a kitchen"));
    super::lock(&lab.core.chat.rt).running = Some(super::Running {
        turn_id: Ulid::nil(),
        user_turn_id: Ulid::nil(),
        project_id: pid(&lab),
        cancel: Default::default(),
    });
    assert_eq!(code(send(&lab, "hi")), ErrorCode::TurnActive);
    super::lock(&lab.core.chat.rt).running = None;
    // Inputs are checked at the boundary.
    assert_eq!(code(send(&lab, "   ")), ErrorCode::InvalidInput);
    assert_eq!(code(send(&lab, &"x".repeat(8001))), ErrorCode::InvalidInput);
    assert_eq!(
        code(lab.core.dispatch(
            "prm_context_publish",
            json!({"project_id": pid(&lab), "host": "rgn", "input": 3})
        )),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        code(lab.core.dispatch(
            "prm_chat_send",
            json!({"project_id": Ulid::nil(), "text": "hi"})
        )),
        ErrorCode::NoProject
    );
    assert_eq!(
        code(
            lab.core
                .dispatch("prm_chat_cancel", json!({"turn_id": Ulid::nil()}))
        ),
        ErrorCode::NotFound
    );
}

fn user_turns(lab: &Lab) -> Vec<Value> {
    thread_view(lab)["thread"]["turns"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["role"] == "user")
        .cloned()
        .collect()
}

#[test]
fn fr_prm_25_an_image_is_resent_only_when_new_or_in_a_fresh_session() {
    let lab = lab();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "one").unwrap();
    let out = send(&lab, "two").unwrap();
    assert_eq!(out["fresh_session"], false);
    assert!(
        out["context_summary"]
            .as_str()
            .unwrap()
            .contains("(already sent)")
    );
    let u = user_turns(&lab);
    assert_eq!(u[0]["context"]["images"].as_array().unwrap().len(), 1);
    assert!(u[1]["context"]["images"].as_array().unwrap().is_empty());

    // A changed region changes the overlay's hash: sent again.
    let mut d = lab.draft("a kitchen");
    d["region"]["margin"] = json!(64);
    publish(&lab, "rgn", d.clone());
    send(&lab, "three").unwrap();
    assert_eq!(
        user_turns(&lab)[2]["context"]["images"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // A text-only turn sends none.
    lab.core
        .dispatch(
            "prm_chat_send",
            json!({"project_id": pid(&lab), "text": "four", "include_images": false}),
        )
        .unwrap();
    let u = user_turns(&lab);
    assert!(u[3]["context"]["images"].as_array().unwrap().is_empty());
    assert_eq!(
        u[3]["context"]["summary"]
            .as_str()
            .unwrap()
            .split(" · ")
            .next(),
        Some("text only")
    );

    // A fresh session (the window closed) starts with none received (FR-PRM-35).
    lab.core
        .dispatch("prm_chat_session_end", json!({}))
        .unwrap();
    assert_eq!(thread_view(&lab)["fresh_session_next"], true);
    let out = send(&lab, "five").unwrap();
    assert_eq!(out["fresh_session"], true);
    assert_eq!(
        user_turns(&lab)[4]["context"]["images"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn fr_prm_28_proposals_per_fake_case() {
    let lab = lab();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    for (case, n) in [("two_proposals", 2), ("no_proposal", 0), ("success", 1)] {
        lab.core
            .dispatch("prm_chat_fake_set_case", json!({"case": case}))
            .unwrap();
        send(&lab, "ideas").unwrap();
        let last = events(&lab, super::EVENT_TURN).last().cloned().unwrap();
        let got = last
            .get("proposals")
            .map_or(0, |p| p.as_array().unwrap().len());
        assert_eq!(got, n, "{case}");
    }
    assert_eq!(
        code(
            lab.core
                .dispatch("prm_chat_fake_set_case", json!({"case": "nope"}))
        ),
        ErrorCode::InvalidInput
    );
}

#[test]
fn fr_prm_29_insert_goes_to_the_turns_host_block() {
    let lab = lab();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "make this a kitchen").unwrap();
    let proposal = events(&lab, super::EVENT_TURN)[0]["proposals"][0]["id"].clone();
    let out = lab
        .core
        .dispatch(
            "prm_chat_insert",
            json!({"project_id": pid(&lab), "proposal_id": proposal}),
        )
        .unwrap();
    assert_eq!(out["host"], "rgn");
    let ins = events(&lab, super::EVENT_INSERT);
    assert_eq!(ins.len(), 1);
    assert_eq!(ins[0]["host"], "rgn");
    assert_eq!(ins[0]["text"], "make this a kitchen");
    assert_eq!(ins[0]["project_id"], json!(pid(&lab)));
    let turns = thread_view(&lab)["thread"]["turns"].clone();
    assert!(
        turns[1]["proposals"][0]["inserted_at"].is_string(),
        "Inserted, with the time"
    );

    // The Region edit block isn't attached (New image visible): no_prompt_block.
    lab.core
        .dispatch(
            "prm_context_clear",
            json!({"project_id": pid(&lab), "host": "rgn"}),
        )
        .unwrap();
    let e = lab
        .core
        .dispatch(
            "prm_chat_insert",
            json!({"project_id": pid(&lab), "proposal_id": proposal}),
        )
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NoPromptBlock);
    assert!(e.message.contains("Region edit"));
    assert_eq!(
        code(lab.core.dispatch(
            "prm_chat_insert",
            json!({"project_id": pid(&lab), "proposal_id": Ulid::nil()})
        )),
        ErrorCode::NotFound
    );
}

#[test]
fn fr_prm_37_backend_errors_reach_send_and_the_thread() {
    let lab = lab();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    // Detection errors refuse the send with their own code (Appendix D in the detail).
    lab.core
        .dispatch("prm_chat_fake_set_case", json!({"case": "not_installed"}))
        .unwrap();
    let e = send(&lab, "hi").unwrap_err();
    assert_eq!(e.code, ErrorCode::NotInstalled);
    assert_eq!(
        e.detail["fix"],
        "Install Claude Code, or set its path in Settings → Prompt chat."
    );
    let st = lab.core.dispatch("prm_chat_status", json!({})).unwrap();
    assert_eq!(st["backend"]["state"], "error");
    assert_eq!(st["backend"]["code"], "not_installed");

    // Turn errors end the reply as failed, with the partial text.
    for (case, err, fresh_after, detail) in [
        ("usage_limit", "usage_limit", false, None),
        ("malformed", "stream_malformed", true, Some("weren't JSON")),
        ("crash", "backend_crashed", true, Some("simulated crash")),
    ] {
        lab.core
            .dispatch("prm_chat_fake_set_case", json!({"case": case}))
            .unwrap();
        send(&lab, "hi").unwrap();
        let end = events(&lab, super::EVENT_TURN).last().cloned().unwrap();
        assert_eq!(end["state"], "failed", "{case}");
        assert_eq!(end["error"]["code"], err, "{case}");
        // L3: the event carries the detail (stderr tail / message) for the window only.
        if let Some(d) = detail {
            let got = end["error"]["detail"].as_str().unwrap_or_default();
            assert!(got.contains(d), "{case}: {got}");
        }
        let turns = thread_view(&lab)["thread"]["turns"]
            .as_array()
            .unwrap()
            .clone();
        let reply = turns.last().unwrap();
        assert_eq!(reply["error"]["code"], err);
        assert!(
            reply["error"].get("detail").is_none(),
            "no stderr in the project"
        );
        assert_eq!(
            thread_view(&lab)["fresh_session_next"],
            fresh_after,
            "{case}"
        );
    }
    let turns = thread_view(&lab)["thread"]["turns"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(turns.last().unwrap()["text"], "Here is a first idea");
}

#[test]
fn fr_prm_22_restart_marks_pending_turns_interrupted_and_corrupt_files_are_set_aside() {
    let lab = lab();
    let ctx = &lab.core.ctx;
    let mut t = Thread::new(ctx.new_id(), ctx.now(), "fake");
    for (role, state) in [
        (thread::Role::User, TurnState::Pending),
        (thread::Role::Assistant, TurnState::Pending),
    ] {
        t.turns.push(thread::Turn {
            id: ctx.new_id(),
            at: ctx.now(),
            role,
            host: crate::prompt::context::ChatHost::Rgn,
            text: "x".into(),
            state,
            error: None,
            context: None,
            proposals: vec![],
            usage: None,
            fresh_session: false,
            extra: Default::default(),
        });
    }
    thread::save(ctx, &lab.project_dir, &t).unwrap();
    let view = thread_view(&lab);
    let states: Vec<&str> = view["thread"]["turns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["interrupted", "interrupted"]);

    // A corrupt thread file is kept aside and a new thread starts with the next send.
    std::fs::write(thread::thread_path(&lab.project_dir, t.id), "{not json").unwrap();
    let view = thread_view(&lab);
    assert!(view["thread"].is_null());
    assert_eq!(view["corrupt_set_aside"], true);
    let aside: Vec<String> = std::fs::read_dir(thread::chat_dir(&lab.project_dir))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(aside.len(), 1);
    assert!(aside[0].starts_with(&format!("{}.corrupt-", t.id)));
    publish(&lab, "rgn", lab.draft("a kitchen"));
    let out = send(&lab, "hi").unwrap();
    assert_ne!(out["thread_id"], json!(t.id));
}

#[test]
fn fr_prm_22_fr_prj_34_a_thread_conflict_copy_goes_to_the_trash() {
    let lab = lab();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "mine").unwrap();
    let ours: Thread = serde_json::from_value(thread_view(&lab)["thread"].clone()).unwrap();
    // The other machine's copy is newer.
    let mut theirs = ours.clone();
    theirs.turns[0].text = "theirs".into();
    theirs.updated_at = ours.updated_at.plus(Duration::from_secs(60));
    let copy = thread::chat_dir(&lab.project_dir).join(format!("{}-OTHERPC.json", ours.id));
    std::fs::write(&copy, serde_json::to_vec(&theirs).unwrap()).unwrap();

    lab.open(); // the open sweep settles copies and commits the trash move
    let kept: Thread = serde_json::from_value(thread_view(&lab)["thread"].clone()).unwrap();
    assert_eq!(
        kept.turns[0].text, "theirs",
        "the later updated_at stays the thread"
    );
    let state = lab.core.state();
    let p = state.project.as_ref().unwrap();
    let item = p
        .manifest
        .trash
        .iter()
        .find(|t| t.kind == crate::manifest::TrashKind::ConflictCopy)
        .expect("a conflict_copy trash item");
    let rel = format!(".pforge/chat/{}-OTHERPC.json", ours.id);
    assert!(item.files.contains(&rel));
    let trashed: Thread =
        serde_json::from_slice(&std::fs::read(p.path(&item.trashed_path(&rel))).unwrap()).unwrap();
    assert_eq!(
        trashed.turns[0].text, "mine",
        "the trash keeps the other version"
    );
    assert!(p.warnings.iter().any(|w| w.code == "chat_conflict_copy"));
    // FR-PRJ-20 amendment: the orphan sweep never touches the thread.
    assert!(thread::thread_path(&lab.project_dir, ours.id).is_file());
    assert!(
        !p.manifest
            .trash
            .iter()
            .any(|t| t.kind == crate::manifest::TrashKind::Orphan)
    );
}

#[test]
fn fr_prm_35_closing_the_project_ends_the_session() {
    let lab = lab();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "hi").unwrap();
    assert!(lab.core.chat.has_session());
    lab.core.dispatch("project_close", json!({})).unwrap();
    assert!(!lab.core.chat.has_session());
    assert!(
        super::lock(&lab.core.chat.rt).contexts.is_empty(),
        "its contexts go too"
    );
}

// ---------------------------------------------------------------- the Claude Code adapter

fn logged_in(data: &Path) -> String {
    logged_in_as(data, "claude.ai")
}

fn logged_in_as(data: &Path, auth_method: &str) -> String {
    json!({
        "loggedIn": true, "authMethod": auth_method, "subscriptionType": "max",
        "configDirectory": data.join("claude-config").to_string_lossy(),
        "email": "someone@example.com",
    })
    .to_string()
}

/// A lab whose chat runs the Claude Code adapter over a replay spawner.
fn claude_lab() -> (Lab, Arc<ReplaySpawner>) {
    claude_lab_as("claude.ai")
}

/// [`claude_lab`] logged in with `auth_method`.
fn claude_lab_as(auth_method: &str) -> (Lab, Arc<ReplaySpawner>) {
    let clock = pf_chat::clock::FakeClock::new();
    let mut spawner: Option<Arc<ReplaySpawner>> = None;
    let lab = Lab::with_context(|ctx| {
        // Left over from a crash: emptied at startup (§11).
        std::fs::create_dir_all(ctx.data_dir.join("chat").join("old-thread")).unwrap();
        std::fs::create_dir_all(
            ctx.data_dir
                .join("claude-config")
                .join("projects")
                .join("p"),
        )
        .unwrap();
        let sp = Arc::new(
            ReplaySpawner::new(Arc::new(clock.clone()))
                .with_file("/cc/claude.exe")
                .on_run(&["--version"], Some(0), "2.1.319 (Claude Code)")
                .on_run(
                    &["auth", "status", "--json"],
                    Some(0),
                    &logged_in_as(&ctx.data_dir, auth_method),
                ),
        );
        ctx.chat.mode = BackendMode::ClaudeCode;
        ctx.chat.spawner = sp.clone();
        ctx.chat.clock = Arc::new(clock.clone());
        spawner = Some(sp);
    });
    let data = lab.core.ctx.data_dir.clone();
    assert!(!data.join("chat").join("old-thread").exists());
    assert!(!data.join("claude-config").join("projects").exists());
    lab.core
        .dispatch(
            "prm_chat_settings_put",
            json!({"claude_path": "/cc/claude.exe"}),
        )
        .unwrap();
    (lab, spawner.unwrap())
}

#[test]
fn fr_prm_33_detection_is_cached_for_60_s() {
    let (lab, sp) = claude_lab();
    let st = consent(&lab);
    assert_eq!(st["backend"]["state"], "ok");
    assert_eq!(st["backend"]["notional"], true, "a plan login");
    assert_eq!(st["backend"]["plan"], "max");
    let cmd = st["login_command"].as_str().unwrap();
    assert!(cmd.contains("CLAUDE_CONFIG_DIR") && cmd.ends_with("claude auth login"));
    let raw = serde_json::to_string(&st).unwrap();
    assert!(!raw.contains("example.com"), "only the allowed auth fields");
    let runs = || sp.log().runs.len();
    assert_eq!(runs(), 2, "--version and auth status");
    lab.core.dispatch("prm_chat_status", json!({})).unwrap();
    assert_eq!(runs(), 2, "cached");
    lab.clock.advance(Duration::from_secs(61));
    lab.core.dispatch("prm_chat_status", json!({})).unwrap();
    assert_eq!(runs(), 4, "the cache expired");
    lab.core
        .dispatch("prm_chat_status", json!({"refresh": true}))
        .unwrap();
    assert_eq!(runs(), 6, "window focus / Recheck");
}

#[test]
fn fr_prm_34_fr_prm_45_a_turn_over_the_adapter_and_the_projects_backstop() {
    let (lab, sp) = claude_lab();
    consent(&lab);
    sp.push_script(Script::recordings(&[SUCCESS_IMAGE, TWO_PROPOSALS]));
    publish(&lab, "rgn", lab.draft("a kitchen"));
    let out = send(&lab, "make this a kitchen").unwrap();
    let end = events(&lab, super::EVENT_TURN).last().cloned().unwrap();
    assert_eq!(end["state"], "done");
    assert_eq!(end["usage"]["notional"], true);
    send(&lab, "two more").unwrap();
    let end = events(&lab, super::EVENT_TURN).last().cloned().unwrap();
    assert_eq!(end["proposals"].as_array().unwrap().len(), 2);

    let log = sp.log();
    assert_eq!(log.spawns.len(), 1, "one child across turns");
    let spec = &log.spawns[0].0;
    assert!(spec.args.iter().any(|a| a == "--safe-mode"));
    let thread_id = out["thread_id"].as_str().unwrap();
    assert!(spec.cwd.ends_with(Path::new("chat").join(thread_id)));
    // The stdin lines: the image block first, then the context block and the user text.
    let lines = log.stdin(0);
    let first: Value = serde_json::from_str(&lines[0]).unwrap();
    let content = first["message"]["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "image");
    let text = content[1]["text"].as_str().unwrap();
    assert!(text.starts_with("[Context]\n"));
    assert!(text.contains("Attached images, in order:\n1. region crop with the mask"));
    assert!(text.ends_with("make this a kitchen"));
    let second: Value = serde_json::from_str(&lines[1]).unwrap();
    let content = second["message"]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1, "the unchanged overlay isn't resent");
    assert!(
        content[0]["text"]
            .as_str()
            .unwrap()
            .contains("Unchanged since an earlier message")
    );

    // FR-PRM-45: after the child ends, projects/ is emptied.
    let projects = lab
        .core
        .ctx
        .data_dir
        .join("claude-config")
        .join("projects")
        .join("x");
    std::fs::create_dir_all(&projects).unwrap();
    lab.core
        .dispatch("prm_chat_session_end", json!({}))
        .unwrap();
    assert!(!projects.exists());
}

#[test]
fn fr_prm_27_stop_keeps_the_partial_reply() {
    let clock = pf_chat::clock::SystemClock::new();
    let mut spawner = None;
    let lab = Lab::with_context(|ctx| {
        let sp = Arc::new(
            ReplaySpawner::new(Arc::new(pf_chat::clock::SystemClock::new()))
                .with_file("/cc/claude.exe")
                .on_run(&["--version"], Some(0), "2.1.319")
                .on_run(
                    &["auth", "status", "--json"],
                    Some(0),
                    &logged_in(&ctx.data_dir),
                ),
        );
        ctx.chat.mode = BackendMode::ClaudeCode;
        ctx.chat.spawner = sp.clone();
        ctx.chat.clock = Arc::new(clock);
        ctx.inline_background = false;
        spawner = Some(sp);
    });
    let sp = spawner.unwrap();
    lab.core
        .dispatch(
            "prm_chat_settings_put",
            json!({"claude_path": "/cc/claude.exe"}),
        )
        .unwrap();
    consent(&lab);
    let assistant = |t: &str| {
        pf_chat::replay::Step::Line(
            json!({"type":"assistant","message":{"content":[{"type":"text","text":t}]}})
                .to_string(),
        )
    };
    sp.push_script(Script {
        turns: vec![vec![
            assistant("first part"),
            pf_chat::replay::Step::Delay(Duration::from_secs(20)),
            assistant("never"),
        ]],
        stderr: String::new(),
    });
    publish(&lab, "rgn", lab.draft("a kitchen"));
    let out = send(&lab, "go").unwrap();
    let wait = |name: &str| {
        for _ in 0..200 {
            if !events(&lab, name).is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("no {name}");
    };
    wait(super::EVENT_DELTA);
    assert_eq!(
        lab.core.chat.running_turn(),
        out["turn_id"].as_str().unwrap().parse().ok()
    );
    // M2 (§11): the running turn reads back `pending`, never `interrupted`, and an insert
    // meanwhile doesn't write it as interrupted.
    let mid = thread_view(&lab)["thread"]["turns"].clone();
    assert_eq!(mid[0]["state"], "pending");
    assert_eq!(mid[1]["state"], "pending");
    lab.core
        .dispatch("prm_chat_cancel", json!({"turn_id": out["turn_id"]}))
        .unwrap();
    wait(super::EVENT_TURN);
    let end = events(&lab, super::EVENT_TURN)[0].clone();
    assert_eq!(end["state"], "canceled");
    assert!(end.get("proposals").is_none());
    let turns = thread_view(&lab)["thread"]["turns"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(turns[1]["state"], "canceled");
    assert_eq!(turns[1]["text"], "first part");
    assert!(lab.core.chat.running_turn().is_none());
    assert_eq!(
        thread_view(&lab)["fresh_session_next"],
        true,
        "the next turn starts fresh"
    );
}

#[test]
fn fr_prm_24_the_overlay_golden() {
    use crate::prompt::context::ChatContextSource;
    let lab = Lab::new();
    let ctx = crate::region_edit::chat::RgnChatContext
        .build(&lab.core, pid(&lab), &lab.draft("a kitchen"))
        .unwrap();
    let img = &ctx.images[0];
    assert_eq!(img.id, "rgn:overlay");
    // No capabilities cached yet: the crop at the region's size (inner + 2 × 128 margin).
    assert_eq!((img.w, img.h), (INNER.w + 256, INNER.h + 256));
    let decoded = image::load_from_memory(&img.jpeg).unwrap().to_rgb8();
    assert_eq!(decoded.dimensions(), (img.w, img.h));
    // The mask's middle is tinted towards magenta; the context corner isn't.
    let mid = decoded.get_pixel(128 + INNER.w / 2, 128 + INNER.h / 2);
    assert!(mid[0] > mid[1] && mid[2] > mid[1], "{mid:?}");
    assert_eq!(
        img.content_hash,
        crate::references::prepare::sha256(&img.jpeg)
    );
    assert_eq!(
        img.content_hash, GOLDEN_OVERLAY,
        "the overlay changed; check it, then update the golden"
    );
    assert!(ctx.facts.iter().any(|f| f == "sent unresampled"));
    assert!(
        ctx.facts
            .iter()
            .any(|f| f.contains("size limits aren't loaded"))
    );
    // With the model's capabilities cached by an estimate, the fact goes.
    lab.estimate(&lab.draft("a kitchen"));
    let ctx = crate::region_edit::chat::RgnChatContext
        .build(&lab.core, pid(&lab), &lab.draft("a kitchen"))
        .unwrap();
    assert!(!ctx.facts.iter().any(|f| f.contains("size limits")));
    // No region or no mask: no image.
    let ctx = crate::region_edit::chat::RgnChatContext
        .build(
            &lab.core,
            pid(&lab),
            &json!({"model_id": "fake-white-edit"}),
        )
        .unwrap();
    assert!(ctx.images.is_empty());
    assert_eq!(ctx.facts, ["no region yet"]);
}

/// FR-PRM-24's golden: the overlay JPEG of the lab region (deterministic: no resampling).
const GOLDEN_OVERLAY: &str =
    "sha256:0a0694704726789ce4a2827d59a6786434cbee4094edc3e97b1b3c36ed07565e";

#[test]
fn bl_41_the_window_hears_when_the_project_or_its_host_changes() {
    let lab = lab();
    let ctx_events = |lab: &Lab| events(lab, super::EVENT_CONTEXT);
    let opened = ctx_events(&lab);
    assert_eq!(
        opened.last().unwrap(),
        &json!({"project_id": pid(&lab), "host": null})
    );
    let n = opened.len();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    publish(&lab, "rgn", lab.draft("a kitchen, edited"));
    let e = ctx_events(&lab);
    assert_eq!(
        e.len(),
        n + 1,
        "only a host change is announced, not every edit"
    );
    assert_eq!(e[n], json!({"project_id": pid(&lab), "host": "rgn"}));
    publish(&lab, "gen", json!({}));
    assert_eq!(ctx_events(&lab).last().unwrap()["host"], "gen");
    lab.core
        .dispatch(
            "prm_context_clear",
            json!({"project_id": pid(&lab), "host": "gen"}),
        )
        .unwrap();
    assert_eq!(ctx_events(&lab).last().unwrap()["host"], "rgn");
    lab.core.dispatch("project_close", json!({})).unwrap();
    assert_eq!(
        ctx_events(&lab).last().unwrap(),
        &json!({"project_id": null, "host": null})
    );
}

#[test]
fn bl_42_fr_prm_26_the_preview_shows_what_the_next_turn_sends() {
    let lab = lab();
    let preview = |include: bool| {
        lab.core
            .dispatch(
                "prm_chat_preview",
                json!({"project_id": pid(&lab), "include_images": include}),
            )
            .unwrap()
    };
    assert!(preview(true)["host"].is_null(), "no prompt block yet");
    publish(&lab, "rgn", lab.draft("a kitchen"));
    let p = preview(true);
    assert_eq!(p["host"], "rgn");
    assert!(p["summary"].as_str().unwrap().starts_with("crop "));
    let img = &p["images"][0];
    assert_eq!(img["id"], "rgn:overlay");
    assert_eq!(img["already_sent"], false);
    assert!(
        img["thumb"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,")
    );
    assert!(
        img["thumb"].as_str().unwrap().len() < 20_000,
        "a small thumbnail"
    );
    assert_eq!(
        preview(false)["summary"]
            .as_str()
            .unwrap()
            .split(" · ")
            .next(),
        Some("text only")
    );
    // After a send the same overlay counts as already received; nothing was written by preview.
    let before = thread_view(&lab)["thread"].clone();
    assert!(before.is_null());
    send(&lab, "hi").unwrap();
    let p = preview(true);
    assert_eq!(p["images"][0]["already_sent"], true);
    assert!(p["summary"].as_str().unwrap().contains("(already sent)"));
}

#[test]
fn fr_prm_40_a_failed_command_leaves_the_project_and_settings_unchanged() {
    let lab = lab();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "make this a kitchen").unwrap();
    let proposal = events(&lab, super::EVENT_TURN)[0]["proposals"][0]["id"].clone();
    let before = lab.persisted();
    let fails = |name: &str, input: Value| -> ErrorCode {
        let e = lab.core.dispatch(name, input).unwrap_err();
        assert_eq!(
            lab.persisted(),
            before,
            "{name} failed with {e:?} but changed state"
        );
        e.code
    };
    let p = pid(&lab);
    let other = Ulid::from_parts(1, 999);
    let mut rows = lab
        .core
        .dispatch("prm_helpers_list", json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    rows[0]["label"] = json!("x".repeat(41));
    let cases: Vec<(&str, Value, ErrorCode)> = vec![
        // Settings: helpers and the chat section.
        (
            "prm_helpers_save",
            json!({"helpers": rows}),
            ErrorCode::InvalidInput,
        ),
        (
            "prm_helpers_reset",
            json!({"id": "builtin:nope"}),
            ErrorCode::NotFound,
        ),
        (
            "prm_helpers_reset",
            json!({"id": "builtin:top-down", "all": true}),
            ErrorCode::InvalidInput,
        ),
        (
            "prm_compose_preview",
            json!({"kind": "map", "typed_prompt": "a", "nope": 1}),
            ErrorCode::InvalidInput,
        ),
        (
            "prm_chat_consent",
            json!({"accept": false, "nope": 1}),
            ErrorCode::InvalidInput,
        ),
        (
            "prm_chat_settings_put",
            json!({"enabled": false, "window": {"x": 0, "y": 0, "w": 200, "h": 200}}),
            ErrorCode::InvalidInput,
        ),
        (
            "prm_chat_settings_put",
            json!({"enabled": false, "claude_path": "c".repeat(1025)}),
            ErrorCode::InvalidInput,
        ),
        // The project: its thread and the attached blocks.
        (
            "prm_chat_send",
            json!({"project_id": p, "text": "   "}),
            ErrorCode::InvalidInput,
        ),
        (
            "prm_chat_send",
            json!({"project_id": other, "text": "hi"}),
            ErrorCode::NoProject,
        ),
        (
            "prm_chat_insert",
            json!({"project_id": p, "proposal_id": other}),
            ErrorCode::NotFound,
        ),
        (
            "prm_chat_cancel",
            json!({"turn_id": other}),
            ErrorCode::NotFound,
        ),
        (
            "prm_context_publish",
            json!({"project_id": p, "host": "rgn", "input": 3}),
            ErrorCode::InvalidInput,
        ),
        (
            "prm_chat_thread_get",
            json!({"project_id": other}),
            ErrorCode::NoProject,
        ),
    ];
    for (name, input, want) in cases {
        assert_eq!(fails(name, input.clone()), want, "{name} {input}");
    }
    // No block attached: send and insert fail, nothing written.
    lab.core
        .dispatch("prm_context_clear", json!({"project_id": p, "host": "rgn"}))
        .unwrap();
    assert_eq!(
        fails("prm_chat_send", json!({"project_id": p, "text": "hi"})),
        ErrorCode::NoPromptBlock
    );
    assert_eq!(
        fails(
            "prm_chat_insert",
            json!({"project_id": p, "proposal_id": proposal}),
        ),
        ErrorCode::NoPromptBlock
    );
    // Read-only: insert is refused.
    publish(&lab, "rgn", lab.draft("a kitchen"));
    lab.core.state().project.as_mut().unwrap().read_only =
        Some(crate::project::ReadOnlyReason::NewerVersion);
    assert_eq!(
        fails(
            "prm_chat_insert",
            json!({"project_id": p, "proposal_id": proposal}),
        ),
        ErrorCode::ReadOnly
    );
}

#[test]
fn fr_prm_44_a_host_switched_turn_gets_the_now_on_note() {
    use pf_chat::system_prompt::{HOST_GEN_V1, HOST_RGN_V1, Host, host_note, system_prompt};
    let (lab, sp) = claude_lab();
    consent(&lab);
    sp.push_script(Script::recordings(&[SUCCESS_IMAGE, TWO_PROPOSALS]));
    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "make this a kitchen").unwrap();
    // The tab switch to New image keeps the session (FR-PRM-43).
    lab.core
        .dispatch(
            "prm_context_clear",
            json!({"project_id": pid(&lab), "host": "rgn"}),
        )
        .unwrap();
    let gen_draft = json!({
        "model_id": "fake-gen-wh",
        "controls": {},
        "seed": null,
        "size": {"mode": "grid", "cols": 30, "rows": 40, "pps": 70},
        "structure": null,
        "references": null,
        "typed_prompt": "a crypt",
        "final_prompt": "a crypt",
    });
    lab.core
        .dispatch("provider_list_models", json!({"operation": "generate"}))
        .unwrap();
    lab.core
        .dispatch("gen_job_estimate", json!({"draft": gen_draft}))
        .unwrap();
    publish(&lab, "gen", gen_draft);
    let out = send(&lab, "a flooded crypt").unwrap();
    assert_eq!(out["host"], "gen");
    assert_eq!(out["fresh_session"], false);

    let log = sp.log();
    assert_eq!(log.spawns.len(), 1, "no restart for the host switch");
    // The session's system prompt is the first turn's host.
    let args = &log.spawns[0].0.args;
    let at = args.iter().position(|a| a == "--system-prompt").unwrap();
    assert_eq!(args[at + 1], system_prompt(Host::Rgn));
    assert!(!args[at + 1].contains(HOST_GEN_V1));
    let text_of = |line: &str| -> String {
        let v: Value = serde_json::from_str(line).unwrap();
        v["message"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["type"] == "text")
            .map(|c| c["text"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
            .join("")
    };
    let lines = log.stdin(0);
    assert_eq!(lines.len(), 2);
    let first = text_of(&lines[0]);
    assert!(first.starts_with("[Context]\n"), "same host: no note");
    assert!(!first.contains("[Now on:"));
    let second = text_of(&lines[1]);
    let note = host_note(Host::Gen);
    assert_eq!(note, format!("[Now on: New image] {HOST_GEN_V1}"));
    assert!(
        second.starts_with(&format!("{note}\n\n[Context]\n")),
        "the note tops the message: {second}"
    );
    assert!(second.ends_with("a flooded crypt"));
    assert!(!second.contains(HOST_RGN_V1));
}

#[test]
fn fr_prm_21_closing_the_window_never_cancels_a_job() {
    use crate::job::JobState;
    let lab = lab();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "make this a kitchen").unwrap();
    let id = lab.submit("a kitchen");
    assert_eq!(lab.job(id).state, JobState::Submitted);
    let calls = lab.fake().call_count();
    // What the window's `Destroyed` handler runs (app/src-tauri chat_window.rs).
    lab.core.chat.end_session();
    assert_eq!(lab.job(id).state, JobState::Submitted, "still running");
    assert_eq!(
        lab.fake().call_count(),
        calls,
        "no provider call (no cancel)"
    );
    lab.drive();
    assert_eq!(lab.job(id).state, JobState::Fetched);
    assert_eq!(lab.candidates(id).len(), 3);
    // The thread is kept.
    assert_eq!(
        thread_view(&lab)["thread"]["turns"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn fr_prm_38_each_turn_records_usage_notional_only_on_a_plan_login() {
    for (auth, notional) in [("claude.ai", true), ("api_key", false)] {
        let (lab, sp) = claude_lab_as(auth);
        let st = consent(&lab);
        assert_eq!(st["backend"]["auth_method"], auth);
        assert_eq!(st["backend"]["notional"], notional, "{auth}");
        sp.push_script(Script::recordings(&[SUCCESS_IMAGE]));
        publish(&lab, "rgn", lab.draft("a kitchen"));
        send(&lab, "make this a kitchen").unwrap();
        let end = events(&lab, super::EVENT_TURN).last().cloned().unwrap();
        assert_eq!(end["state"], "done");
        let want = json!({"input_tokens": 2, "output_tokens": 246, "cost_usd": 0.016376,
            "notional": notional});
        assert_eq!(end["usage"], want, "{auth}");
        // Recorded on the reply turn in the thread file.
        let turns = thread_view(&lab)["thread"]["turns"].clone();
        assert_eq!(turns[1]["usage"], want, "{auth}");
        assert!(turns[0]["usage"].is_null(), "the user turn has none");
    }
    // The fake bills nothing: notional (L4).
    let lab = lab();
    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "hi").unwrap();
    let end = events(&lab, super::EVENT_TURN).last().cloned().unwrap();
    assert_eq!(end["usage"]["notional"], true);
}

#[test]
fn fr_prm_32_the_mode_picks_the_backend_over_injected_seams() {
    // Fake mode: the shared fake; the process spawner is never touched.
    let lab = Lab::with_context(|ctx| {
        ctx.chat.mode = BackendMode::Fake;
        ctx.chat.spawner = Arc::new(PanicSpawner);
    });
    let st = consent(&lab);
    assert_eq!(st["mode"], "fake");
    assert_eq!(st["backend"]["auth_method"], "fake");
    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "hi").unwrap();
    let end = events(&lab, super::EVENT_TURN).last().cloned().unwrap();
    assert_eq!(end["state"], "done");
    let view = thread_view(&lab);
    assert_eq!(view["thread"]["backend"], "fake");

    // Claude Code mode: the adapter, over the injected spawner (replay) and clock.
    let (lab, sp) = claude_lab();
    let st = consent(&lab);
    assert_eq!(st["mode"], "claude-code");
    assert_eq!(st["backend"]["version"], "2.1.319");
    assert_eq!(
        sp.log().runs.len(),
        2,
        "detection ran on the injected spawner"
    );
    sp.push_script(Script::recordings(&[SUCCESS_IMAGE]));
    publish(&lab, "rgn", lab.draft("a kitchen"));
    send(&lab, "hi").unwrap();
    assert_eq!(
        sp.log().spawns.len(),
        1,
        "the turn's child came from the seam"
    );
    assert_eq!(thread_view(&lab)["thread"]["backend"], "claude-code");
}

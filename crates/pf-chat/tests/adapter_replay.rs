//! The Claude Code adapter over the process seam (PRM §13): the real invocation, detection,
//! parser and lifecycle, replaying the owner's recordings (`fixtures/chat/claude-code/`)
//! and hand-made lines. No test starts the CLI.
#![allow(clippy::unwrap_used)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pf_chat::backend::{
    CancelHandle, ChatBackend, ChatSession, SessionOptions, TurnEvent, TurnImage, TurnInput,
    TurnOutcome,
};
use pf_chat::claude_code::{ClaudeCodeBackend, ClaudeCodeConfig};
use pf_chat::clock::{Clock, FakeClock};
use pf_chat::error::ChatErrorKind;
use pf_chat::invocation::{self, ExeKind};
use pf_chat::process::CommandSpec;
use pf_chat::replay::{ReplaySpawner, Script, Step};
use pf_chat::system_prompt::{Host, system_prompt};
use serde_json::Value;

const SUCCESS_IMAGE: &str = include_str!("../../../fixtures/chat/claude-code/success-image.jsonl");
const TWO_PROPOSALS: &str = include_str!("../../../fixtures/chat/claude-code/two-proposals.jsonl");
const NOT_LOGGED_IN: &str = include_str!("../../../fixtures/chat/claude-code/not-logged-in.jsonl");
const USAGE_LIMIT: &str = include_str!("../../../fixtures/chat/handmade/usage-limit.jsonl");
const MALFORMED: &str = include_str!("../../../fixtures/chat/handmade/malformed.jsonl");

const S: fn(u64) -> Duration = Duration::from_secs;

/// A Windows home path built at run time (never written in a tracked file).
fn home() -> String {
    ["C:", "Users", "alice"].join("\\")
}

fn parent_env() -> Vec<(OsString, OsString)> {
    [
        ("PATH", "/bin".to_owned()),
        ("USERPROFILE", home()),
        ("HOME", home()),
        ("HOMEDRIVE", "C:".to_owned()),
        ("HOMEPATH", "\\Users\\alice".to_owned()),
        ("PF_DATA_DIR", "/x".to_owned()),
        ("PF_CHAT_REAL", "1".to_owned()),
        ("SystemRoot", "C:\\Windows".to_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (OsString::from(k), OsString::from(v)))
    .collect()
}

struct Rig {
    _tmp: tempfile::TempDir,
    data: PathBuf,
    clock: FakeClock,
    spawner: Arc<ReplaySpawner>,
    backend: ClaudeCodeBackend,
}

fn rig_with(exe: &str, setup: impl FnOnce(ReplaySpawner) -> ReplaySpawner) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let clock = FakeClock::new();
    let spawner = Arc::new(setup(
        ReplaySpawner::new(Arc::new(clock.clone())).with_file(exe),
    ));
    let cfg = ClaudeCodeConfig {
        data_dir: data.clone(),
        claude_path: None,
        parent_env: parent_env(),
        windows: true,
    };
    let backend = ClaudeCodeBackend::new(cfg, spawner.clone(), Arc::new(clock.clone()));
    Rig {
        _tmp: tmp,
        data,
        clock,
        spawner,
        backend,
    }
}

fn rig() -> Rig {
    rig_with("/bin/claude.exe", |s| s)
}

fn env_of(spec: &CommandSpec, name: &str) -> Option<String> {
    spec.env_var(name).map(|v| v.to_string_lossy().into_owned())
}

fn input(text: &str) -> TurnInput {
    TurnInput {
        host: Host::Rgn,
        message_text: format!("Context: model fake-model\n\n{text}"),
        user_text: text.into(),
        model_id: Some("fake-model".into()),
        final_prompt: Some("a kitchen".into()),
        images: vec![TurnImage {
            id: "rgn:overlay".into(),
            content_hash: "sha256:0011".into(),
            width: 512,
            height: 640,
            media_type: "image/jpeg".into(),
            bytes: vec![0xff, 0xd8, 0xff, 0xe0],
        }],
    }
}

impl Rig {
    fn thread(&self) -> PathBuf {
        self.data.join("chat").join("t1")
    }

    fn open(&self, script: Script) -> Box<dyn ChatSession> {
        self.spawner.push_script(script);
        self.backend
            .open_session(SessionOptions {
                first_host: Host::Rgn,
                cwd: self.thread(),
            })
            .unwrap()
    }
}

fn turn(s: &mut dyn ChatSession, input: &TurnInput) -> (TurnOutcome, Vec<String>) {
    let mut deltas = Vec::new();
    let out = s.run_turn(input, &CancelHandle::new(), &mut |e| {
        let TurnEvent::Delta(d) = e;
        deltas.push(d);
    });
    (out, deltas)
}

fn same_path(a: &str, b: &Path) -> bool {
    a.replace('\\', "/") == b.to_string_lossy().replace('\\', "/")
}

// ---------------------------------------------------------------- invocation

#[test]
fn fr_prm_34_session_spawn_is_exactly_s10_with_the_isolated_env() {
    let r = rig();
    let junk = r.data.join("chat-home").join(".claude").join("CLAUDE.md");
    std::fs::create_dir_all(junk.parent().unwrap()).unwrap();
    std::fs::write(&junk, "leak").unwrap();

    let _s = r.open(Script::recordings(&[SUCCESS_IMAGE]));
    let log = r.spawner.log();
    let spec = &log.spawns[0].0;
    assert_eq!(spec.program, PathBuf::from("/bin").join("claude.exe"));
    assert_eq!(spec.kind, ExeKind::Native);
    assert_eq!(spec.args, invocation::chat_args(&system_prompt(Host::Rgn)));
    assert!(spec.args.iter().any(|a| a == "--safe-mode"));
    assert!(spec.hidden);
    assert_eq!(spec.cwd, r.thread());
    assert!(r.thread().is_dir());
    // FR-PRM-45 / R87: the isolated config, the empty chat home, no PF_* variables.
    assert!(same_path(
        &env_of(spec, "CLAUDE_CONFIG_DIR").unwrap(),
        &r.data.join("claude-config")
    ));
    let chat_home = r.data.join("chat-home");
    assert!(same_path(&env_of(spec, "USERPROFILE").unwrap(), &chat_home));
    assert!(same_path(&env_of(spec, "HOME").unwrap(), &chat_home));
    assert_eq!(env_of(spec, "HOMEDRIVE"), None);
    assert_eq!(env_of(spec, "HOMEPATH"), None);
    assert!(
        spec.env
            .iter()
            .all(|(k, _)| !k.to_string_lossy().to_uppercase().starts_with("PF_"))
    );
    assert_eq!(
        env_of(spec, "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC").as_deref(),
        Some("1")
    );
    assert_eq!(env_of(spec, "DISABLE_AUTOUPDATER").as_deref(), Some("1"));
    // The chat home was emptied before the start.
    assert!(chat_home.is_dir());
    assert_eq!(std::fs::read_dir(&chat_home).unwrap().count(), 0);
}

#[test]
fn fr_prm_34_npm_shim_gets_the_system_prompt_as_a_file() {
    let r = rig_with("/bin/claude.cmd", |s| s);
    r.spawner.push_script(Script::default());
    let _s = r
        .backend
        .open_session(SessionOptions {
            first_host: Host::Gen,
            cwd: r.thread(),
        })
        .unwrap();
    let spec = r.spawner.log().spawns[0].0.clone();
    assert_eq!(spec.kind, ExeKind::CmdShim);
    assert!(!spec.args.iter().any(|a| a == "--system-prompt"));
    assert!(spec.args.iter().all(|a| !a.contains('\n')));
    let i = spec
        .args
        .iter()
        .position(|a| a == "--system-prompt-file")
        .unwrap();
    let text = std::fs::read_to_string(&spec.args[i + 1]).unwrap();
    assert_eq!(text, system_prompt(Host::Gen));
}

// ---------------------------------------------------------------- detection

fn logged_in(dir: &Path) -> String {
    serde_json::json!({
        "loggedIn": true, "authMethod": "claude.ai", "subscriptionType": "max",
        "configDirectory": dir.to_string_lossy().replace('/', "\\"),
        "email": "someone@example.com", "orgId": "org-1",
    })
    .to_string()
}

#[test]
fn fr_prm_33_detection_results() {
    // not_installed
    let r = rig_with("/elsewhere/claude.exe", |s| s);
    assert_eq!(r.backend.detect().unwrap_err().code(), "not_installed");

    // version_unsupported
    let r = rig_with("/bin/claude.exe", |s| {
        s.on_run(&["--version"], Some(0), "2.1.300 (Claude Code)\n")
    });
    let e = r.backend.detect().unwrap_err();
    assert_eq!(
        e.kind,
        ChatErrorKind::VersionUnsupported {
            found: "2.1.300".into(),
            min: "2.1.319".into()
        }
    );

    // not_logged_in (exit 1)
    let r = rig_with("/bin/claude.exe", |s| {
        s.on_run(&["--version"], Some(0), "2.1.319 (Claude Code)")
            .on_run(
                &["auth", "status", "--json"],
                Some(1),
                r#"{"loggedIn":false,"authMethod":"none"}"#,
            )
    });
    assert_eq!(r.backend.detect().unwrap_err().code(), "not_logged_in");

    // No version in the output; a timed-out command.
    let r = rig_with("/bin/claude.exe", |s| {
        s.on_run(&["--version"], Some(0), "hello")
    });
    assert_eq!(r.backend.detect().unwrap_err().code(), "backend_error");
    let r = rig_with("/bin/claude.exe", |s| s.on_run(&["--version"], None, ""));
    assert_eq!(r.backend.detect().unwrap_err().code(), "timeout");
}

#[test]
fn fr_prm_33_detection_ok_and_its_environment() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let clock = FakeClock::new();
    let spawner = Arc::new(
        ReplaySpawner::new(Arc::new(clock.clone()))
            .with_file("/bin/claude.exe")
            .on_run(&["--version"], Some(0), "2.1.319 (Claude Code)")
            .on_run(
                &["auth", "status", "--json"],
                Some(0),
                &logged_in(&data.join("claude-config")),
            ),
    );
    let cfg = ClaudeCodeConfig {
        data_dir: data.clone(),
        claude_path: None,
        parent_env: parent_env(),
        windows: true,
    };
    let backend = ClaudeCodeBackend::new(cfg, spawner.clone(), Arc::new(clock));
    let ok = backend.detect().unwrap();
    assert_eq!(ok.version, "2.1.319");
    assert_eq!(ok.auth_method.as_deref(), Some("claude.ai"));
    assert_eq!(ok.plan.as_deref(), Some("max"));
    assert!(ok.notional());
    let dbg = format!("{ok:?}");
    assert!(!dbg.contains("example") && !dbg.contains("org-1"));
    // Detection keeps the real home and sets the isolated config (FR-PRM-45, R87).
    let runs = spawner.log().runs;
    assert_eq!(runs.len(), 2);
    for spec in &runs {
        assert!(spec.hidden);
        assert_eq!(env_of(spec, "USERPROFILE"), Some(home()));
        assert!(same_path(
            &env_of(spec, "CLAUDE_CONFIG_DIR").unwrap(),
            &data.join("claude-config")
        ));
        assert_eq!(env_of(spec, "PF_DATA_DIR"), None);
    }
    assert!(data.join("claude-config").is_dir());
}

#[test]
fn fr_prm_45_missing_or_foreign_config_directory_fails_closed() {
    let user_claude = PathBuf::from(home()).join(".claude");
    for stdout in [
        r#"{"loggedIn":true,"authMethod":"claude.ai"}"#.to_owned(),
        logged_in(&user_claude),
    ] {
        let r = rig_with("/bin/claude.exe", |s| {
            s.on_run(&["--version"], Some(0), "2.1.319").on_run(
                &["auth", "status", "--json"],
                Some(0),
                &stdout,
            )
        });
        assert_eq!(
            r.backend.detect().unwrap_err().code(),
            "isolation_unavailable"
        );
    }
    // Not logged in, but in a foreign folder: isolation first.
    let foreign = serde_json::json!({
        "loggedIn": false,
        "configDirectory": user_claude.to_string_lossy(),
    })
    .to_string();
    let r = rig_with("/bin/claude.exe", |s| {
        s.on_run(&["--version"], Some(0), "2.1.319").on_run(
            &["auth", "status", "--json"],
            Some(1),
            &foreign,
        )
    });
    assert_eq!(
        r.backend.detect().unwrap_err().code(),
        "isolation_unavailable"
    );
}

#[test]
fn fr_prm_45_login_console_and_logout() {
    let r = rig_with("/bin/claude.exe", |s| {
        s.on_run(&["auth", "logout"], Some(0), "")
    });
    let cmd = r.backend.login_command().unwrap().unwrap();
    assert_eq!(cmd.spec.args, ["auth", "login"]);
    assert!(!cmd.spec.hidden, "the login runs in a visible console");
    assert_eq!(env_of(&cmd.spec, "USERPROFILE"), Some(home()));
    assert!(same_path(
        &env_of(&cmd.spec, "CLAUDE_CONFIG_DIR").unwrap(),
        &r.data.join("claude-config")
    ));
    assert!(
        cmd.copy_text.contains("CLAUDE_CONFIG_DIR") && cmd.copy_text.ends_with("claude auth login")
    );
    r.backend.open_login_console().unwrap();
    assert_eq!(r.spawner.log().consoles.len(), 1);
    r.backend.logout().unwrap();

    let r = rig_with("/bin/claude.exe", |s| {
        s.on_run(&["auth", "logout"], Some(1), "")
    });
    assert_eq!(r.backend.logout().unwrap_err().code(), "backend_error");
}

// ---------------------------------------------------------------- turns

fn result_of(recording: &str) -> Value {
    recording
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v["type"] == "result")
        .unwrap()
}

#[test]
fn fr_prm_37_replays_the_success_image_recording() {
    let r = rig();
    let mut s = r.open(Script::recordings(&[SUCCESS_IMAGE]));
    let inp = input("make this a kitchen");
    let (out, deltas) = turn(s.as_mut(), &inp);
    let TurnOutcome::Completed { text, usage } = out else {
        panic!("{out:?}")
    };
    assert!(text.starts_with("Magenta."));
    assert!(text.contains("```prompt\n"));
    assert_eq!(deltas.concat(), text);
    let rec = result_of(SUCCESS_IMAGE);
    let usage = usage.unwrap();
    assert_eq!(
        usage.input_tokens,
        rec["usage"]["input_tokens"].as_u64().unwrap()
    );
    assert_eq!(
        usage.output_tokens,
        rec["usage"]["output_tokens"].as_u64().unwrap()
    );
    assert_eq!(usage.cost_usd, rec["total_cost_usd"].as_f64());
    // The stdin line: the image block, then the composed text (FR-PRM-34).
    let line: Value = serde_json::from_str(&r.spawner.log().stdin(0)[0]).unwrap();
    let content = &line["message"]["content"];
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["source"]["data"], "/9j/4A==");
    assert_eq!(content[1]["text"], inp.message_text);
    assert!(s.is_alive());
}

#[test]
fn fr_prm_35_two_turns_in_one_process() {
    let r = rig();
    let mut s = r.open(Script::recordings(&[SUCCESS_IMAGE, TWO_PROPOSALS]));
    let (a, _) = turn(s.as_mut(), &input("make this a kitchen"));
    assert!(matches!(a, TurnOutcome::Completed { .. }));
    let (b, _) = turn(s.as_mut(), &input("two more, please"));
    let TurnOutcome::Completed { text, .. } = b else {
        panic!("{b:?}")
    };
    assert_eq!(text.matches("```prompt").count(), 2);
    let log = r.spawner.log();
    assert_eq!(log.spawns.len(), 1);
    assert_eq!(log.stdin(0).len(), 2);
    assert!(s.is_alive());
}

#[test]
fn fr_prm_37_error_recordings_map_to_codes() {
    let r = rig();
    let mut s = r.open(Script::recordings(&[NOT_LOGGED_IN]));
    let (out, deltas) = turn(s.as_mut(), &input("hi"));
    let TurnOutcome::Failed { error, partial } = out else {
        panic!("{out:?}")
    };
    assert_eq!(error.code(), "not_logged_in");
    assert!(
        partial.is_empty() && deltas.is_empty(),
        "the CLI's error text isn't a reply"
    );
    assert!(s.is_alive(), "a result ends the turn, not the process");

    let r = rig();
    let mut s = r.open(Script::recordings(&[USAGE_LIMIT]));
    let (out, _) = turn(s.as_mut(), &input("hi"));
    let TurnOutcome::Failed { error, .. } = out else {
        panic!("{out:?}")
    };
    assert_eq!(error.code(), "usage_limit");
    assert_eq!(error.fix, "You've hit your limit · resets 3pm");

    let r = rig();
    let mut s = r.open(Script::recordings(&[MALFORMED]));
    let (out, _) = turn(s.as_mut(), &input("hi"));
    let TurnOutcome::Failed { error, .. } = out else {
        panic!("{out:?}")
    };
    assert_eq!(error.code(), "stream_malformed");
    assert!(!s.is_alive());
}

fn assistant(text: &str) -> Step {
    Step::Line(
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":text}]}})
            .to_string(),
    )
}

fn init() -> Step {
    Step::Line(r#"{"type":"system","subtype":"init","tools":[]}"#.into())
}

#[test]
fn fr_prm_37_crash_offline_and_other_errors() {
    let r = rig();
    let projects = r.data.join("claude-config").join("projects").join("p");
    let stderr = format!("boom in {}\\AppData\\x", home());
    let mut s = r.open(
        Script {
            turns: vec![vec![init(), assistant("partial"), Step::Exit(Some(3))]],
            stderr: String::new(),
        }
        .with_stderr(&stderr),
    );
    std::fs::create_dir_all(&projects).unwrap();
    let (out, _) = turn(s.as_mut(), &input("hi"));
    let TurnOutcome::Failed { error, partial } = out else {
        panic!("{out:?}")
    };
    assert_eq!(
        error.kind,
        ChatErrorKind::BackendCrashed { exit_code: Some(3) }
    );
    assert_eq!(partial, "partial");
    let detail = error.detail.unwrap();
    assert!(
        detail.contains("<home>") && !detail.contains("alice"),
        "{detail}"
    );
    assert!(!s.is_alive());
    assert!(
        !projects.exists(),
        "projects/ is emptied after the child ends (FR-PRM-45)"
    );

    for (result, code) in [
        ("API Error: Connection error.", "offline"),
        ("Something else broke", "backend_error"),
    ] {
        let r = rig();
        let line = serde_json::json!({"type":"result","subtype":"success","is_error":true,"result":result});
        let mut s = r.open(Script {
            turns: vec![vec![init(), Step::Line(line.to_string())]],
            stderr: String::new(),
        });
        let (out, _) = turn(s.as_mut(), &input("hi"));
        let TurnOutcome::Failed { error, .. } = out else {
            panic!("{out:?}")
        };
        assert_eq!(error.code(), code);
        assert_eq!(error.detail.as_deref(), Some(result));
    }
}

#[test]
fn fr_prm_36_timeouts_under_a_fake_clock() {
    // No event within 60 s of the send.
    let r = rig();
    let mut s = r.open(Script {
        turns: vec![vec![Step::Stall]],
        stderr: String::new(),
    });
    let (out, _) = turn(s.as_mut(), &input("hi"));
    assert!(matches!(&out, TurnOutcome::Failed { error, .. } if error.code() == "timeout"));
    assert_eq!(r.clock.now(), S(60));
    assert!(!s.is_alive(), "the child is killed on timeout");

    // More than 90 s between events; the partial text is kept.
    let r = rig();
    let mut s = r.open(Script {
        turns: vec![vec![
            init(),
            Step::Delay(S(30)),
            assistant("so far"),
            Step::Stall,
        ]],
        stderr: String::new(),
    });
    let (out, _) = turn(s.as_mut(), &input("hi"));
    let TurnOutcome::Failed { error, partial } = out else {
        panic!("{out:?}")
    };
    assert_eq!((error.code(), partial.as_str()), ("timeout", "so far"));
    assert_eq!(r.clock.now(), S(120));

    // Events every 80 s: the 300 s total cap.
    let r = rig();
    let mut steps = vec![init()];
    for i in 0..10 {
        steps.push(Step::Delay(S(80)));
        steps.push(assistant(&format!("part {i}")));
    }
    let mut s = r.open(Script {
        turns: vec![steps],
        stderr: String::new(),
    });
    let (out, _) = turn(s.as_mut(), &input("hi"));
    let TurnOutcome::Failed { error, partial } = out else {
        panic!("{out:?}")
    };
    assert_eq!(error.code(), "timeout");
    assert_eq!(r.clock.now(), S(300));
    assert!(partial.starts_with("part 0\n\npart 1\n\npart 2"));
}

#[test]
fn fr_prm_27_cancel_keeps_the_partial_text_and_ends_the_child() {
    let r = rig();
    let projects = r.data.join("claude-config").join("projects");
    let mut s = r.open(Script {
        turns: vec![vec![
            init(),
            assistant("first"),
            Step::Delay(S(10)),
            assistant("second"),
            Step::Line(
                r#"{"type":"result","subtype":"success","is_error":false,"result":"x"}"#.into(),
            ),
        ]],
        stderr: String::new(),
    });
    std::fs::create_dir_all(&projects).unwrap();
    let cancel = CancelHandle::new();
    let stop = cancel.clone();
    let out = s.run_turn(&input("hi"), &cancel, &mut |_| stop.cancel());
    assert_eq!(
        out,
        TurnOutcome::Canceled {
            partial: "first".into()
        }
    );
    assert!(!s.is_alive());
    assert!(!projects.exists());

    // A stop that arrives before the turn starts: nothing is sent.
    let r = rig();
    let mut s = r.open(Script::recordings(&[SUCCESS_IMAGE]));
    let cancel = CancelHandle::new();
    cancel.cancel();
    let out = s.run_turn(&input("hi"), &cancel, &mut |_| {});
    assert!(matches!(out, TurnOutcome::Canceled { .. }));
    assert!(r.spawner.log().stdin(0).is_empty());
}

#[test]
fn fr_prm_35_close_ends_the_session_and_sweeps_projects() {
    let r = rig();
    let projects = r.data.join("claude-config").join("projects").join("p");
    let mut s = r.open(Script::recordings(&[SUCCESS_IMAGE]));
    std::fs::create_dir_all(&projects).unwrap();
    s.close();
    assert!(!s.is_alive());
    assert!(!projects.exists());
    // A turn on a closed session fails without a process.
    let (out, _) = turn(s.as_mut(), &input("hi"));
    assert!(matches!(out, TurnOutcome::Failed { .. }));
}

// ---------------------------------------------------------------- error heuristics

fn result_line(is_error: bool, result: &str) -> Step {
    Step::Line(
        serde_json::json!({"type":"result","subtype":"success","is_error":is_error,"result":result})
            .to_string(),
    )
}

fn assistant_error(error: &str, text: &str) -> Step {
    Step::Line(
        serde_json::json!({"type":"assistant","error":error,"message":{"content":[{"type":"text","text":text}]}})
            .to_string(),
    )
}

fn outcome_of(steps: Vec<Step>, stderr: &str) -> (TurnOutcome, bool) {
    let r = rig();
    let mut s = r.open(
        Script {
            turns: vec![steps],
            stderr: String::new(),
        }
        .with_stderr(stderr),
    );
    let (out, _) = turn(s.as_mut(), &input("hi"));
    (out, s.is_alive())
}

fn code_of(out: &TurnOutcome) -> &'static str {
    match out {
        TurnOutcome::Failed { error, .. } => error.code(),
        TurnOutcome::Completed { .. } => "completed",
        TurnOutcome::Canceled { .. } => "canceled",
    }
}

/// FR-PRM-37 / Appendix D: the committed recordings plus synthetic variants of what the CLI
/// prints for each state (R89: heuristics until more real recordings exist).
#[test]
fn fr_prm_37_error_heuristics_map_cli_messages_to_states() {
    let not_logged_in = Script::recordings(&[NOT_LOGGED_IN]).turns.remove(0);
    let usage_limit = Script::recordings(&[USAGE_LIMIT]).turns.remove(0);
    let expired = r#"API Error: 401 {"type":"error","error":{"type":"authentication_error","message":"OAuth token has expired. Please obtain a new token or refresh your existing token."}}"#;
    let cases: Vec<(&str, Vec<Step>, &str)> = vec![
        // Not logged in.
        ("recording: not logged in", not_logged_in, "not_logged_in"),
        ("invalid api key", vec![init(), result_line(true, "Invalid API key · Please run /login")], "not_logged_in"),
        // Auth expired (the 401 body, with and without the CLI's "/login" hint).
        ("auth expired", vec![init(), result_line(true, expired)], "not_logged_in"),
        ("auth expired + hint", vec![init(), result_line(true, &format!("{expired} · Please run /login"))], "not_logged_in"),
        ("auth failed, empty result", vec![init(), assistant_error("authentication_failed", "OAuth token revoked"), result_line(true, "")], "not_logged_in"),
        // Usage limit.
        ("recording: usage limit", usage_limit, "usage_limit"),
        ("old limit format", vec![init(), result_line(true, "Claude AI usage limit reached|1762700000")], "usage_limit"),
        ("5-hour limit", vec![init(), result_line(true, "5-hour limit reached ∙ resets 3pm")], "usage_limit"),
        ("weekly limit", vec![init(), result_line(true, "Weekly limit reached · resets Mon 9am")], "usage_limit"),
        ("rate_limit assistant error", vec![init(), assistant_error("rate_limit", "Rate limited"), result_line(true, "")], "usage_limit"),
        ("api 429", vec![init(), result_line(true, r#"API Error: 429 {"type":"error","error":{"type":"rate_limit_error","message":"Rate limited"}}"#)], "usage_limit"),
        // Offline.
        ("connection error", vec![init(), result_line(true, "API Error: Connection error.")], "offline"),
        ("retrying", vec![init(), result_line(true, "API Error (Connection error.) · Retrying in 8 seconds… (attempt 10/10)")], "offline"),
        ("dns", vec![init(), result_line(true, "getaddrinfo ENOTFOUND api.anthropic.com")], "offline"),
        ("reset", vec![init(), result_line(true, "read ECONNRESET")], "offline"),
        ("errors[] only", vec![init(), Step::Line(r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["fetch failed"]}"#.into())], "offline"),
        // Anything else.
        ("other", vec![init(), result_line(true, "Something else broke")], "backend_error"),
        ("context limit", vec![init(), result_line(true, "Context limit reached · /compact or /clear to continue")], "backend_error"),
        // CLI crash mid-stream: no result, stdout closes.
        ("crash with code", vec![init(), assistant("half"), Step::Exit(Some(1))], "backend_crashed"),
        ("crash, killed", vec![init(), assistant("half"), Step::Exit(None)], "backend_crashed"),
        // An auth error and then an exit without a result is still the auth state.
        ("auth error then exit", vec![init(), assistant_error("authentication_failed", "Not logged in · Please run /login"), Step::Exit(Some(1))], "not_logged_in"),
        // Truncated JSON lines.
        ("truncated then exit", vec![init(), assistant("half"), Step::Line(r#"{"type":"assistant","message":{"content":[{"type":"te"#.into()), Step::Exit(Some(1))], "backend_crashed"),
        ("truncated then result", vec![init(), Step::Line(r#"{"type":"assistant","mess"#.into()), result_line(false, "fine")], "completed"),
        ("three truncated", vec![init(), Step::Line("{\"a".into()), Step::Line("{\"b".into()), Step::Line("{\"c".into())], "stream_malformed"),
    ];
    for (name, steps, want) in cases {
        let (out, alive) = outcome_of(steps, "boom");
        assert_eq!(code_of(&out), want, "{name}: {out:?}");
        // Only a `result` keeps the process for the next turn.
        let has_result =
            !matches!(want, "backend_crashed" | "stream_malformed") && !name.contains("then exit");
        assert_eq!(alive, has_result, "{name}");
        if want == "usage_limit" {
            let TurnOutcome::Failed { error, .. } = &out else {
                unreachable!()
            };
            assert!(!error.fix.is_empty(), "{name}");
        }
        if name.starts_with("crash") || name == "truncated then exit" {
            let TurnOutcome::Failed { error, partial } = &out else {
                unreachable!()
            };
            assert_eq!(partial, "half", "{name}: the partial reply is kept");
            assert_eq!(
                error.detail.as_deref(),
                Some("boom"),
                "{name}: the stderr tail"
            );
        }
    }
}

// ---------------------------------------------------------------- login copy text

/// review-sec 5 (R96): the folder is single-quoted, so `$`, backticks and `"` stay literal.
#[test]
fn fr_prm_45_login_copy_text_quotes_the_folder() {
    use pf_chat::claude_code::login_copy_text;
    assert_eq!(
        login_copy_text(r"C:\pf $HOME `n o'brien\claude-config", true),
        r"$env:CLAUDE_CONFIG_DIR = 'C:\pf $HOME `n o''brien\claude-config'; claude auth login"
    );
    assert_eq!(
        login_copy_text("/pf $HOME `x` o'brien/claude-config", false),
        r"CLAUDE_CONFIG_DIR='/pf $HOME `x` o'\''brien/claude-config' claude auth login"
    );
}

/// The Windows copy text, run by real PowerShell (no CLI: `claude auth login` is replaced by
/// printing the variable), sets exactly the folder.
#[cfg(windows)]
#[test]
fn fr_prm_45_login_copy_text_round_trips_through_powershell() {
    use pf_chat::claude_code::login_copy_text;
    let dir = r"C:\pf $HOME $(whoami) `t o'brien & a;b ünï\claude-config";
    let script = login_copy_text(dir, true).replace(
        "claude auth login",
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8; [Console]::Out.Write($env:CLAUDE_CONFIG_DIR)",
    );
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand"])
        .arg(pf_chat::base64::encode(&utf16))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_start_matches('\u{feff}'),
        dir
    );
}

// ---------------------------------------------------------------- size caps

/// review-sec 7 (R96): a stdout line over 1 MiB is `stream_malformed`; the child is killed.
#[test]
fn fr_prm_37_a_stdout_line_over_1_mib_is_malformed() {
    let big = "x".repeat(pf_chat::stream::MAX_LINE);
    let line =
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":big}]}})
            .to_string();
    assert!(line.len() > pf_chat::stream::MAX_LINE);
    let (out, alive) = outcome_of(vec![init(), assistant("before"), Step::Line(line)], "");
    let TurnOutcome::Failed { error, partial } = out else {
        panic!("{out:?}")
    };
    assert_eq!(error.code(), "stream_malformed");
    assert_eq!(partial, "before");
    assert!(!alive);
}

/// review-sec 7 (R96): the reply is capped at 1 MiB; what fits is kept and streamed, cut at
/// a character boundary, then the turn ends with `stream_malformed`.
#[test]
fn fr_prm_37_the_reply_is_capped() {
    let cap = pf_chat::stream::MAX_REPLY;
    // Each message is under the line cap; together they pass the reply cap.
    let chunk = "é".repeat(cap / 6);
    let steps: Vec<Step> = std::iter::once(init())
        .chain((0..4).map(|_| assistant(&chunk)))
        .chain(std::iter::once(result_line(false, "x")))
        .collect();
    let r = rig();
    let mut s = r.open(Script {
        turns: vec![steps],
        stderr: String::new(),
    });
    let (out, deltas) = turn(s.as_mut(), &input("hi"));
    let TurnOutcome::Failed { error, partial } = out else {
        panic!("{out:?}")
    };
    assert_eq!(error.code(), "stream_malformed");
    assert!(
        partial.len() <= cap && partial.len() > cap - 4,
        "{}",
        partial.len()
    );
    assert_eq!(deltas.concat(), partial, "the UI saw exactly what is kept");
    assert!(!s.is_alive());
}

// ---------------------------------------------------------------- blocked stdin

/// review-code 5 (FR-PRM-36): a child that never reads its stdin can't hold the turn past
/// the 60 s first-event budget; the write is given up on, the child killed.
#[test]
fn fr_prm_36_a_blocked_stdin_write_times_out() {
    let r = rig();
    let mut s = r.open(Script {
        turns: vec![vec![Step::StdinStall]],
        stderr: String::new(),
    });
    let (out, deltas) = turn(s.as_mut(), &input("hi"));
    assert!(
        matches!(&out, TurnOutcome::Failed { error, .. } if error.code() == "timeout"),
        "{out:?}"
    );
    assert!(deltas.is_empty());
    assert_eq!(r.clock.now(), S(60));
    assert!(!s.is_alive(), "the child is killed");
}

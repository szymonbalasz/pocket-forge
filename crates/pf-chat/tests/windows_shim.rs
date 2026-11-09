//! The npm `claude.cmd` shim on the real spawner (R89, §10), Windows only. A fake npm
//! install is generated in a temp folder: the real `cmd-shim` template forwarding to a
//! fake `cli.js` run by `node`, which reports its argv, environment, cwd, the system prompt
//! file and the stdin message. No Claude Code CLI and no network: the fake home and `PATH`
//! hold only the fake shim, `node` and the system folder, so no real CLI can be resolved.
#![cfg(windows)]
#![allow(clippy::unwrap_used)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pf_chat::backend::{
    CancelHandle, ChatBackend, ChatSession, SessionOptions, TurnEvent, TurnInput, TurnOutcome,
};
use pf_chat::claude_code::{ClaudeCodeBackend, ClaudeCodeConfig};
use pf_chat::clock::SystemClock;
use pf_chat::invocation::{self, ExeKind, ExeSource};
use pf_chat::process::OsSpawner;
use pf_chat::system_prompt::{Host, system_prompt};
use serde_json::Value;

/// npm's `cmd-shim` template (npm 7+), as `npm install -g` writes `claude.cmd`.
const NPM_SHIM: &str = r#"@ECHO off
GOTO start
:find_dp0
SET dp0=%~dp0
EXIT /b
:start
SETLOCAL
CALL :find_dp0

IF EXIST "%dp0%\node.exe" (
  SET "_prog=%dp0%\node.exe"
) ELSE (
  SET "_prog=node"
  SET PATHEXT=%PATHEXT:;.JS;=;%
)

endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%"  "%dp0%\node_modules\@anthropic-ai\claude-code\cli.js" %*
"#;

/// The fake CLI. `--version` and `auth status --json` answer like 2.1.319 (with CRLF and
/// stderr noise); otherwise it speaks stream-json and answers each user message with one
/// assistant message holding a JSON report, then a `result`. A message containing `SPAWN`
/// or `HANG` starts a detached grandchild that never exits (outside node's own job, so
/// only ours can end it) and writes both pids to `FAKE_CLAUDE_PIDFILE`; `HANG` then never
/// sends a `result` and ignores stdin closing.
const FAKE_CLI: &str = r#"
const fs = require('fs');
const args = process.argv.slice(2);
const out = (o) => process.stdout.write(JSON.stringify(o) + '\n');
const KEYS = ['CLAUDE_CONFIG_DIR', 'HOME', 'USERPROFILE', 'HOMEDRIVE', 'HOMEPATH',
  'ANTHROPIC_API_KEY', 'PF_DATA_DIR', 'PF_LIBRARY_DIR', 'PF_CHAT_REAL',
  'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC', 'DISABLE_AUTOUPDATER'];
const env = {};
for (const k of KEYS) if (process.env[k] !== undefined) env[k] = process.env[k];
if (args[0] === '--version') {
  process.stderr.write('(node) some warning on stderr 9.9.9\n');
  process.stdout.write('2.1.319 (Claude Code)\r\n');
} else if (args[0] === 'auth' && args[1] === 'status') {
  out({ loggedIn: true, authMethod: 'claude.ai', subscriptionType: 'max',
        configDirectory: process.env.CLAUDE_CONFIG_DIR });
} else {
  const i = args.indexOf('--system-prompt-file');
  let prompt;
  try { prompt = fs.readFileSync(args[i + 1], 'utf8'); } catch (e) { prompt = 'ERR ' + e.message; }
  out({ type: 'system', subtype: 'init', tools: [] });
  let hanging = false;
  const rl = require('readline').createInterface({ input: process.stdin });
  rl.on('line', (line) => {
    const msg = JSON.parse(line);
    const text = msg.message.content.filter((b) => b.type === 'text').map((b) => b.text).join('');
    if (text.includes('HANG') || text.includes('SPAWN')) {
      const g = require('child_process').spawn(process.execPath,
        ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore', detached: true });
      fs.writeFileSync(process.env.FAKE_CLAUDE_PIDFILE, process.pid + '\n' + g.pid + '\n');
    }
    if (text.includes('HANG')) {
      out({ type: 'assistant', message: { content: [{ type: 'text', text: 'hanging' }] } });
      hanging = true;
      setInterval(() => {}, 1000);
      return;
    }
    const report = { argv: args, env, cwd: process.cwd(), prompt, text };
    out({ type: 'assistant', message: { content: [{ type: 'text', text: JSON.stringify(report) }] } });
    out({ type: 'result', subtype: 'success', is_error: false, result: '',
          usage: { input_tokens: 1, output_tokens: 1 } });
  });
  rl.on('close', () => { if (!hanging) process.exit(0); });
}
"#;

/// Characters `cmd` treats specially, plus an apostrophe and non-ASCII.
const NASTY: &str = "pf data 'q' & a^b %PATH% %PF_NOPE% !bang! ünï 東";

/// The folder that holds `node.exe` on the parent `PATH`.
fn node_dir() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .find(|d| d.join("node.exe").is_file())
        .expect("node.exe on PATH (the Windows CI job sets up node)")
}

struct Rig {
    _tmp: tempfile::TempDir,
    data: PathBuf,
    shim: PathBuf,
    pidfile: PathBuf,
    backend: ClaudeCodeBackend,
}

fn rig() -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(NASTY);
    // npm's global prefix under a user name with a space, an apostrophe and non-ASCII.
    let prefix = tmp.path().join("npm o'brien ünï");
    let cli = prefix
        .join("node_modules")
        .join("@anthropic-ai")
        .join("claude-code");
    std::fs::create_dir_all(&cli).unwrap();
    std::fs::write(cli.join("cli.js"), FAKE_CLI).unwrap();
    let shim = prefix.join("claude.cmd");
    std::fs::write(&shim, NPM_SHIM.replace('\n', "\r\n")).unwrap();
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let data = root.join("data");
    let pidfile = tmp.path().join("pids.txt");

    let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    let path = std::env::join_paths([prefix.clone(), node_dir(), system]).unwrap();
    let mut parent_env: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(k, _)| {
            let k = k.to_string_lossy().to_uppercase();
            !["PATH", "USERPROFILE", "HOME", "HOMEDRIVE", "HOMEPATH"].contains(&k.as_str())
                && !k.starts_with("ANTHROPIC_")
                && !k.starts_with("CLAUDE")
        })
        .collect();
    for (k, v) in [
        ("PATH", path),
        ("USERPROFILE", home.clone().into_os_string()),
        ("HOME", home.into_os_string()),
        ("HOMEDRIVE", "C:".into()),
        ("HOMEPATH", "\\nowhere".into()),
        ("PF_DATA_DIR", "leak".into()),
        ("ANTHROPIC_API_KEY", "dummy-not-a-key".into()),
        ("FAKE_CLAUDE_PIDFILE", pidfile.clone().into_os_string()),
    ] {
        parent_env.push((k.into(), v));
    }
    let cfg = ClaudeCodeConfig {
        data_dir: data.clone(),
        claude_path: None,
        parent_env,
        windows: true,
    };
    let backend = ClaudeCodeBackend::new(cfg, Arc::new(OsSpawner), Arc::new(SystemClock::new()));
    Rig {
        _tmp: tmp,
        data,
        shim,
        pidfile,
        backend,
    }
}

fn input(text: &str) -> TurnInput {
    TurnInput {
        host: Host::Gen,
        message_text: text.into(),
        user_text: text.into(),
        model_id: None,
        final_prompt: None,
        images: Vec::new(),
    }
}

impl Rig {
    fn thread(&self) -> PathBuf {
        self.data.join("chat").join("t1")
    }

    fn open(&self) -> Box<dyn ChatSession> {
        let exe = self.backend.resolve().unwrap();
        // Never a real CLI: the fake shim, found as `claude.cmd` on PATH (§10).
        assert_eq!(exe.path, self.shim);
        assert_eq!(
            (exe.kind, exe.source),
            (ExeKind::CmdShim, ExeSource::NpmShim)
        );
        self.backend
            .open_session(SessionOptions {
                first_host: Host::Gen,
                cwd: self.thread(),
            })
            .unwrap()
    }
}

fn same_path(a: &str, b: &Path) -> bool {
    a.eq_ignore_ascii_case(&b.to_string_lossy())
}

#[test]
fn fr_prm_33_detection_through_the_npm_shim() {
    let r = rig();
    let ok = r.backend.detect().unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(ok.version, "2.1.319");
    assert_eq!(ok.auth_method.as_deref(), Some("claude.ai"));
}

#[test]
fn fr_prm_34_npm_shim_survives_special_characters_and_text_goes_over_stdin() {
    let r = rig();
    let mut s = r.open();
    let text = "Draw \"it\" & more | <b> ^caret^ 100% %PATH% !bang! 'q' ünï 東\nline two \\ end";
    let mut deltas = String::new();
    let out = s.run_turn(&input(text), &CancelHandle::new(), &mut |e| {
        let TurnEvent::Delta(d) = e;
        deltas.push_str(&d);
    });
    let TurnOutcome::Completed { text: reply, .. } = out else {
        panic!("{out:?}")
    };
    let report: Value = serde_json::from_str(&reply).unwrap_or_else(|e| panic!("{e}: {reply}"));

    // The arguments arrive exactly as built, the prompt file path included.
    let prompt_file = r
        .data
        .join(pf_chat::claude_code::PROMPT_DIR_NAME)
        .join(format!(
            "system-gen-v{}.txt",
            pf_chat::system_prompt::PROMPT_CHAT_SYSTEM_VERSION
        ));
    let want = invocation::chat_args_with_prompt_file(&prompt_file.to_string_lossy());
    let argv: Vec<String> = report["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(argv, want);
    assert_eq!(
        report["prompt"].as_str(),
        Some(system_prompt(Host::Gen).as_str())
    );
    // The message goes over stdin, byte for byte, and never into argv (FR-PRM-34).
    assert_eq!(report["text"].as_str(), Some(text));
    assert!(argv.iter().all(|a| !a.contains("caret")));
    assert!(same_path(report["cwd"].as_str().unwrap(), &r.thread()));

    // R87 / FR-PRM-45 through the shim: the isolated config, the empty chat home.
    let env = &report["env"];
    let config = r.data.join("claude-config");
    let chat_home = r.data.join("chat-home");
    assert!(same_path(
        env["CLAUDE_CONFIG_DIR"].as_str().unwrap(),
        &config
    ));
    assert!(same_path(env["USERPROFILE"].as_str().unwrap(), &chat_home));
    assert!(same_path(env["HOME"].as_str().unwrap(), &chat_home));
    assert!(env.get("HOMEDRIVE").is_none() && env.get("HOMEPATH").is_none());
    assert!(env.get("PF_DATA_DIR").is_none());
    // §10's env is "user env − PF_* − HOMEDRIVE/HOMEPATH + …": the API key passes through
    // (it overrides the plan login; detection reports `api_key`, FR-PRM-38 warns). Whether
    // to allow-list the environment instead is an open owner question (review-sec 3).
    assert_eq!(env["ANTHROPIC_API_KEY"], "dummy-not-a-key");
    assert_eq!(env["DISABLE_AUTOUPDATER"], "1");
    assert_eq!(env["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"], "1");
    assert!(s.is_alive());
    s.close();
    assert!(!s.is_alive());
}

/// Whether a process with `pid` runs (`tasklist`, a system tool; no new dependency).
fn running(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
}

/// Waits up to 10 s for the fake CLI's pid file: `[node, grandchild]`.
fn tree_pids(r: &Rig) -> Vec<u32> {
    let start = Instant::now();
    loop {
        let pids: Vec<u32> = std::fs::read_to_string(&r.pidfile)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect();
        if pids.len() == 2 {
            return pids;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "no pid file");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn assert_tree_gone(pids: &[u32]) {
    let start = Instant::now();
    while pids.iter().any(|&p| running(p)) {
        if start.elapsed() > Duration::from_secs(10) {
            // Don't leak them: they hold the test's output pipe open.
            for p in pids {
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/PID", &p.to_string()])
                    .output();
            }
            panic!("a process of the cmd → node → node tree outlived the job: {pids:?}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Stop (FR-PRM-27, §10): cancel kills the shim's whole tree, `cmd.exe` → `node` (the
/// CLI) → its own child, through the Job Object.
#[test]
fn fr_prm_27_cancel_kills_the_shim_process_tree() {
    let r = rig();
    let mut s = r.open();
    let cancel = CancelHandle::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let pids = std::thread::scope(|scope| {
        let stop = cancel.clone();
        scope.spawn(move || {
            // Stop once the CLI and its grandchild run.
            let _: Result<(), _> = rx.recv();
            stop.cancel();
        });
        let mut tx = Some(tx);
        let mut pids = Vec::new();
        let out = s.run_turn(&input("HANG please"), &cancel, &mut |_| {
            if let Some(tx) = tx.take() {
                pids = tree_pids(&r);
                assert!(pids.iter().all(|&p| running(p)), "{pids:?}");
                let _ = tx.send(());
            }
        });
        assert_eq!(
            out,
            TurnOutcome::Canceled {
                partial: "hanging".into()
            }
        );
        pids
    });
    assert!(!s.is_alive());
    assert_tree_gone(&pids);
}

/// App exit (§10, R75): dropping the session (the app closing) closes stdin; the CLI exits
/// but leaves a grandchild running, which ends with the job
/// (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: the same handle close an app crash causes).
#[test]
fn prm_s10_session_drop_kills_what_the_cli_leaves_behind() {
    let r = rig();
    let mut s = r.open();
    let out = s.run_turn(&input("SPAWN"), &CancelHandle::new(), &mut |_| {});
    assert!(matches!(out, TurnOutcome::Completed { .. }), "{out:?}");
    let pids = tree_pids(&r);
    assert!(pids.iter().all(|&p| running(p)), "{pids:?}");
    drop(s);
    assert_tree_gone(&pids);
}

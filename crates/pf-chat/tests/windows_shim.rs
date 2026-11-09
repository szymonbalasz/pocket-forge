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
/// assistant message holding a JSON report, then a `result`. A message containing `HANG`
/// starts a grandchild, writes both pids to `FAKE_CLAUDE_PIDFILE` and never answers.
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
  const rl = require('readline').createInterface({ input: process.stdin });
  rl.on('line', (line) => {
    const msg = JSON.parse(line);
    const text = msg.message.content.filter((b) => b.type === 'text').map((b) => b.text).join('');
    if (text.includes('HANG')) {
      const g = require('child_process').spawn(process.execPath,
        ['-e', 'setInterval(() => {}, 1000)'], { stdio: 'ignore' });
      fs.writeFileSync(process.env.FAKE_CLAUDE_PIDFILE, process.pid + '\n' + g.pid + '\n');
      out({ type: 'assistant', message: { content: [{ type: 'text', text: 'hanging' }] } });
      setInterval(() => {}, 1000);
      return;
    }
    const report = { argv: args, env, cwd: process.cwd(), prompt, text };
    out({ type: 'assistant', message: { content: [{ type: 'text', text: JSON.stringify(report) }] } });
    out({ type: 'result', subtype: 'success', is_error: false, result: '',
          usage: { input_tokens: 1, output_tokens: 1 } });
  });
  rl.on('close', () => process.exit(0));
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
    assert_eq!(env["DISABLE_AUTOUPDATER"], "1");
    assert_eq!(env["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"], "1");
    assert!(s.is_alive());
    s.close();
    assert!(!s.is_alive());
}

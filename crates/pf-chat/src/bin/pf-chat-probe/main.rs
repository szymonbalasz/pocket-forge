//! `pf-chat-probe`: PRM slice 0. Runs §10's checks against the installed Claude Code CLI,
//! records scrubbed fixtures into `fixtures/chat/claude-code/`, and prints the claims table
//! and the minimum version. Built only with `--features real-call`; refuses to run in CI.
//! Owner runbook: `docs/guides/prompt-chat-probe.md`.
//!
//! ```text
//! cargo run -p pf-chat --features real-call --bin pf-chat-probe -- <command> [options]
//!   status   version + auth status of the isolated config (free)
//!   login    `claude auth login` into <data-dir>/claude-config/ (interactive)
//!   logout   `claude auth logout` for that folder
//!   run      every check; real turns use the plan login of <data-dir>/claude-config/
//! options: --data-dir <dir> (default $PF_DATA_DIR, else .pf-dev/data)
//!          --claude <path>  (executable override)  --out <dir> (default fixtures/chat/claude-code)
//!          --no-record      --max-turn (adds one 6-image, ~4 MiB turn)
//! ```

mod images;
mod session;
mod standin;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pf_chat::auth::{AuthState, AuthStatus, parse_auth_status, same_config_dir};
use pf_chat::claims::{self, Claim, Status};
use pf_chat::invocation::{
    self, ExeKind, ExeSource, Executable, chat_child_env, child_env, resolve_executable,
};
use pf_chat::message::{ImageBlock, user_message_line};
use pf_chat::scrub::Scrubber;
use pf_chat::stream::{InitInfo, LineOutcome, LineParser, StreamEvent, TurnResult};
use pf_chat::system_prompt::{Host, system_prompt};
use pf_chat::version::{MIN_VERSION_FLOOR, Version, parse_version};

use session::{KillReport, Next, Session, capture, command};

/// FR-PRM-36 budgets.
const FIRST_EVENT: Duration = Duration::from_secs(60);
const MAX_GAP: Duration = Duration::from_secs(90);
const TURN_TOTAL: Duration = Duration::from_secs(300);
const CLI_TIMEOUT: Duration = Duration::from_secs(30);
const CODE_WORD: &str = "TEAPOT-31";
const THREAD: &str = "probe-thread";

fn usage() -> String {
    "usage: pf-chat-probe <status|login|logout|run> [--data-dir <dir>] [--claude <path>] [--out <dir>] [--no-record] [--max-turn]".into()
}

struct Opts {
    cmd: String,
    claude: Option<PathBuf>,
    data_dir: PathBuf,
    out: PathBuf,
    record: bool,
    max_turn: bool,
}

fn parse_opts() -> Result<Opts, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| -> Result<Option<String>, String> {
        match args.iter().position(|a| a == name) {
            Some(i) => args
                .get(i + 1)
                .cloned()
                .map(Some)
                .ok_or_else(|| format!("{name} needs a value")),
            None => Ok(None),
        }
    };
    let cmd = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .ok_or_else(usage)?;
    let data_dir = value("--data-dir")?
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("PF_DATA_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(".pf-dev/data"));
    let data_dir = std::path::absolute(&data_dir).map_err(|e| format!("data dir: {e}"))?;
    Ok(Opts {
        cmd,
        claude: value("--claude")?.map(PathBuf::from),
        data_dir,
        out: value("--out")?
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("fixtures/chat/claude-code")),
        record: !args.iter().any(|a| a == "--no-record"),
        max_turn: args.iter().any(|a| a == "--max-turn"),
    })
}

fn main() {
    let ci = std::env::var("CI").ok();
    if let Err(e) = pf_chat::ci_guard(ci.as_deref(), cfg!(feature = "real-call")) {
        eprintln!("pf-chat-probe: {e}");
        std::process::exit(1);
    }
    let result = parse_opts().and_then(|o| match o.cmd.as_str() {
        "status" => status(&o),
        "login" => auth_console(&o, invocation::auth_login_args()),
        "logout" => auth_console(&o, invocation::auth_logout_args()),
        "run" => run(&o),
        _ => Err(usage()),
    });
    match result {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(e) => {
            eprintln!("pf-chat-probe: {e}");
            std::process::exit(1);
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Everything the commands share: the executable and the isolated environment.
struct Base {
    home: Option<PathBuf>,
    data_dir: PathBuf,
    config_dir: PathBuf,
    /// `<data-dir>/chat-home/`, the chat child's empty home (FR-PRM-45).
    chat_home: PathBuf,
    exe: Option<Executable>,
    /// Detection and login commands.
    env: Vec<(OsString, OsString)>,
    /// Chat children: `env` with the home pointed at `chat_home`.
    chat_env: Vec<(OsString, OsString)>,
}

impl Base {
    fn new(o: &Opts) -> Result<Self, String> {
        let config_dir = invocation::config_dir(&o.data_dir);
        fs::create_dir_all(&config_dir).map_err(|e| format!("create claude-config: {e}"))?;
        let chat_home = invocation::chat_home_dir(&o.data_dir);
        fs::create_dir_all(&chat_home).map_err(|e| format!("create chat-home: {e}"))?;
        let home = home_dir();
        let exe = resolve_executable(
            o.claude.as_deref(),
            std::env::var_os("PATH").as_deref(),
            home.as_deref(),
            cfg!(windows),
            &|p: &Path| p.is_file(),
        );
        let env = child_env(std::env::vars_os(), &config_dir);
        let chat_env = chat_child_env(std::env::vars_os(), &config_dir, &chat_home);
        Ok(Self {
            home,
            data_dir: o.data_dir.clone(),
            config_dir,
            chat_home,
            exe,
            env,
            chat_env,
        })
    }

    fn exe(&self) -> Result<&Executable, String> {
        self.exe.as_ref().ok_or_else(|| {
            "Claude Code not found (claude.exe on PATH, <home>/.local/bin, claude.cmd); pass --claude <path>".into()
        })
    }

    fn version(&self) -> Result<Option<Version>, String> {
        let c = command(
            self.exe()?,
            &invocation::version_args(),
            &self.env,
            &self.data_dir,
            true,
        );
        let out = capture(c, CLI_TIMEOUT).map_err(|e| format!("--version: {e}"))?;
        Ok(parse_version(&out.stdout))
    }

    /// `auth status --json` with `config_dir` as `CLAUDE_CONFIG_DIR`. Only the four allowed
    /// fields survive; the raw output is never printed.
    fn auth_status(&self, config_dir: &Path) -> Result<AuthState, String> {
        let env = child_env(std::env::vars_os(), config_dir);
        let c = command(
            self.exe()?,
            &invocation::auth_status_args(),
            &env,
            &self.data_dir,
            true,
        );
        let out = capture(c, CLI_TIMEOUT).map_err(|e| format!("auth status: {e}"))?;
        if out.timed_out {
            return Ok(AuthState::Unreadable { exit_code: None });
        }
        Ok(parse_auth_status(out.code, &out.stdout))
    }
}

fn describe(s: &AuthStatus, config_dir: &Path) -> String {
    let dir = match &s.config_directory {
        Some(d) if same_config_dir(d, config_dir) => "the isolated folder",
        Some(_) => "ANOTHER folder (isolation_unavailable)",
        None => "not reported",
    };
    format!(
        "method {}, plan {}, config folder: {dir}",
        s.auth_method.as_deref().unwrap_or("?"),
        s.subscription_type.as_deref().unwrap_or("?"),
    )
}

fn status(o: &Opts) -> Result<bool, String> {
    let b = Base::new(o)?;
    let exe = b.exe()?;
    println!("executable: {}", exe_text(exe));
    let v = b.version()?;
    println!(
        "version: {}",
        v.map_or_else(|| "unreadable".into(), |v| v.to_string())
    );
    Ok(match b.auth_status(&b.config_dir)? {
        AuthState::LoggedIn(s) => {
            println!("logged in: yes, {}", describe(&s, &b.config_dir));
            true
        }
        AuthState::NotLoggedIn(_) => {
            println!(
                "logged in: no. Run: cargo run -p pf-chat --features real-call --bin pf-chat-probe -- login"
            );
            false
        }
        AuthState::Unreadable { exit_code } => {
            println!("auth status unreadable (exit {exit_code:?})");
            false
        }
    })
}

/// `claude auth login` / `logout` in this console, with the isolated environment.
fn auth_console(o: &Opts, args: Vec<String>) -> Result<bool, String> {
    let b = Base::new(o)?;
    println!(
        "Running `claude {}` with CLAUDE_CONFIG_DIR = <data-dir>/claude-config ...",
        args.join(" ")
    );
    let st = command(b.exe()?, &args, &b.env, &b.data_dir, false)
        .status()
        .map_err(|e| format!("claude {}: {e}", args.join(" ")))?;
    println!("exit: {:?}", st.code());
    status(o)
}

fn exe_text(e: &Executable) -> &'static str {
    match (e.source, e.kind) {
        (ExeSource::Override, ExeKind::CmdShim) => "--claude override (.cmd, via cmd /d /c)",
        (ExeSource::Override, ExeKind::Native) => "--claude override (native)",
        (ExeSource::Path, _) => "claude.exe on PATH",
        (ExeSource::NativeInstaller, _) => "<home>/.local/bin (native installer)",
        (ExeSource::NpmShim, _) => "claude.cmd on PATH (npm, via cmd /d /c)",
    }
}

// ---- turns ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    Result,
    Eof,
    Timeout(&'static str),
    Malformed,
    Killed,
    SendFailed,
}

struct Turn {
    raw: Vec<String>,
    init: Option<InitInfo>,
    text: String,
    n_assistant: usize,
    n_deltas: usize,
    result: Option<TurnResult>,
    first: Option<Duration>,
    max_gap: Duration,
    total: Duration,
    end: End,
    kill: Option<KillReport>,
}

impl Turn {
    fn ok(&self) -> bool {
        self.end == End::Result && self.result.as_ref().is_some_and(|r| !r.is_error)
    }

    fn reply(&self) -> String {
        self.result
            .as_ref()
            .and_then(|r| r.result.clone())
            .unwrap_or_else(|| self.text.clone())
    }

    fn end_text(&self) -> String {
        match (self.end, &self.result) {
            (End::Result, Some(r)) if r.is_error => format!("error result ({})", r.subtype),
            (End::Result, _) => "result".into(),
            (End::Eof, _) => "stdout closed without a result".into(),
            (End::Timeout(w), _) => format!("timeout: {w}"),
            (End::Malformed, _) => "stream_malformed".into(),
            (End::Killed, _) => "killed".into(),
            (End::SendFailed, _) => "stdin write failed".into(),
        }
    }
}

/// Sends one line and reads to the `result` (or a budget, EOF, malformed, or the kill rule).
fn run_turn(s: &mut Session, line: &str, kill_after_deltas: Option<usize>) -> Turn {
    let mut t = Turn {
        raw: Vec::new(),
        init: None,
        text: String::new(),
        n_assistant: 0,
        n_deltas: 0,
        result: None,
        first: None,
        max_gap: Duration::ZERO,
        total: Duration::ZERO,
        end: End::Eof,
        kill: None,
    };
    if s.send(line).is_err() {
        t.end = End::SendFailed;
        return t;
    }
    let start = std::time::Instant::now();
    let mut last = start;
    let mut parser = LineParser::new();
    loop {
        let elapsed = start.elapsed();
        let left = TURN_TOTAL.saturating_sub(elapsed);
        let wait = match t.first {
            None => FIRST_EVENT.saturating_sub(elapsed),
            Some(_) => MAX_GAP.saturating_sub(last.elapsed()),
        }
        .min(left);
        match s.next(wait) {
            Next::Line(l) => {
                let now = std::time::Instant::now();
                match t.first {
                    None => t.first = Some(now - start),
                    Some(_) => t.max_gap = t.max_gap.max(now - last),
                }
                last = now;
                let outcome = parser.push(&l);
                t.raw.push(l);
                match outcome {
                    LineOutcome::Event(StreamEvent::Init(i)) => t.init = Some(*i),
                    LineOutcome::Event(StreamEvent::AssistantText(x)) => {
                        t.n_assistant += 1;
                        t.text.push_str(&x);
                    }
                    LineOutcome::Event(StreamEvent::TextDelta(_)) => {
                        t.n_deltas += 1;
                        if kill_after_deltas.is_some_and(|k| t.n_deltas >= k) {
                            t.kill = Some(s.kill());
                            t.end = End::Killed;
                            break;
                        }
                    }
                    LineOutcome::Event(StreamEvent::Result(r)) => {
                        t.result = Some(r);
                        t.end = End::Result;
                        break;
                    }
                    LineOutcome::Malformed => {
                        t.end = End::Malformed;
                        break;
                    }
                    _ => {}
                }
            }
            Next::Eof => {
                t.end = End::Eof;
                break;
            }
            Next::Timeout => {
                t.end = End::Timeout(if t.first.is_none() {
                    "no event within 60 s"
                } else if start.elapsed() >= TURN_TOTAL {
                    "turn over 300 s"
                } else {
                    "over 90 s between events"
                });
                t.kill = Some(s.kill());
                break;
            }
        }
    }
    t.total = start.elapsed();
    t
}

fn context_text(user: &str, facts: &str) -> String {
    format!(
        "[Context: Region edit]\nModel: Ideogram 3.0 (probe stand-in), 4 controls\nTyped prompt: (empty)\nFinal prompt: Top-down battlemap view.\nFacts: {facts}\n\n{user}"
    )
}

fn count_prompt_fences(text: &str) -> usize {
    text.lines()
        .filter(|l| l.trim_start().starts_with("```prompt"))
        .count()
}

fn secs(d: Duration) -> String {
    format!("{:.1} s", d.as_secs_f64())
}

fn files_under(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

fn truncate(s: &str, n: usize) -> String {
    let one_line = s.replace(['\r', '\n'], " ");
    match one_line.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &one_line[..i]),
        None => one_line,
    }
}

// ---- the run ----

struct Probe<'a> {
    o: &'a Opts,
    b: Base,
    root: PathBuf,
    main_cwd: PathBuf,
    standin: Option<standin::StandIn>,
    claims: Vec<Claim>,
    version: Option<Version>,
    iso_init: Option<InitInfo>,
    timings: Vec<(String, Option<Duration>, Duration, Duration)>,
    projects: Vec<(String, usize)>,
    recorded: Vec<String>,
    leaky: Vec<String>,
}

const CLAIMS: [(&str, &str, bool); 25] = [
    ("C01", "Executable resolved (§10 order)", true),
    ("C02", "`--version` parses and is ≥ the floor", true),
    (
        "C03",
        "`auth status` reports not logged in for a fresh, empty config folder (exit 1 / loggedIn false)",
        false,
    ),
    (
        "C04",
        "A turn without a login ends with an error result, no usage (recording not-logged-in)",
        false,
    ),
    (
        "C05",
        "Logged in to <data-dir>/claude-config/ (`pf-chat-probe login`)",
        true,
    ),
    (
        "C06",
        "`auth status` configDirectory = <data-dir>/claude-config/ (FR-PRM-45)",
        true,
    ),
    (
        "C07",
        "`auth status --json` has loggedIn and subscriptionType (observed, undocumented)",
        false,
    ),
    (
        "C08",
        "Isolation: §10's invocation (--safe-mode, empty chat home) uses no stand-in marker from the cwd's parent folders (CLAUDE.md, .claude/CLAUDE.md, hook, skill, agent, command, plugin, MCP)",
        true,
    ),
    ("C09", "Isolation: the debug log shows no marker read", true),
    (
        "C10",
        "Debug log mentions of the stand-in ~/.claude paths (review the samples)",
        false,
    ),
    (
        "C11",
        "Isolation: your real home adds nothing to system/init (plugins, skills, agents, MCP, commands) over the stand-in",
        true,
    ),
    (
        "C12",
        "Base64 image block on stream-json stdin reaches the model (it names the tint colour)",
        true,
    ),
    (
        "C13",
        "--system-prompt applies (the reply uses ```prompt fences)",
        false,
    ),
    (
        "C14",
        "Two turns in one process with --no-session-persistence (turn 2 recalls turn 1)",
        true,
    ),
    (
        "C15",
        "No transcript: <data-dir>/claude-config/projects/ stays empty (after exit and after a kill)",
        true,
    ),
    ("C16", "The process exits 0 when stdin closes", false),
    (
        "C17",
        "result has total_cost_usd and usage tokens (FR-PRM-38)",
        false,
    ),
    (
        "C18",
        "Streaming: §10's list gives whole messages; --include-partial-messages gives text deltas",
        false,
    ),
    (
        "C19",
        "Kill mid-reply: stdout closes within 5 s and the process exits (no orphan holds the pipe)",
        true,
    ),
    (
        "C20",
        "Every turn within FR-PRM-36 budgets (first event 60 s, gap 90 s, total 300 s)",
        false,
    ),
    (
        "C21",
        "A 6-image turn of ~4 MiB on stdin works (FR-PRM-25 worst case; --max-turn)",
        false,
    ),
    (
        "C22",
        "apiKeySource and ANTHROPIC_API_KEY (§10 privacy)",
        false,
    ),
    (
        "C23",
        "Recordings written to fixtures/chat/claude-code/ and scrubbed",
        false,
    ),
    (
        "C24",
        "Isolation layer: --safe-mode alone, with the stand-in as the home folder and AGENTS.md above the cwd, uses no marker (user CLAUDE.md, AGENTS.md, hook, skill, agent, command, plugin, MCP)",
        true,
    ),
    (
        "C25",
        "Control: the same session without --safe-mode does show the user CLAUDE.md marker (so C08 and C24 can fail)",
        true,
    ),
];

impl<'a> Probe<'a> {
    fn new(o: &'a Opts) -> Result<Self, String> {
        if o.record && !o.out.parent().is_some_and(Path::is_dir) {
            return Err(format!(
                "{} not found: run from the repo root, or pass --out / --no-record",
                o.out.display()
            ));
        }
        let b = Base::new(o)?;
        let root = b.data_dir.join("chat-probe");
        let main_cwd = b.data_dir.join("chat").join(THREAD);
        for d in [&root, &main_cwd] {
            if d.exists() {
                fs::remove_dir_all(d).map_err(|e| format!("clear probe folder: {e}"))?;
            }
            fs::create_dir_all(d).map_err(|e| format!("create probe folder: {e}"))?;
        }
        let claims = CLAIMS
            .iter()
            .map(|(id, text, gate)| Claim::new(id, *text, *gate))
            .collect();
        Ok(Self {
            o,
            b,
            root,
            main_cwd,
            standin: None,
            claims,
            version: None,
            iso_init: None,
            timings: Vec::new(),
            projects: Vec::new(),
            recorded: Vec::new(),
            leaky: Vec::new(),
        })
    }

    fn c(&mut self, id: &str) -> &mut Claim {
        let i = self
            .claims
            .iter()
            .position(|c| c.id == id)
            .unwrap_or_default();
        &mut self.claims[i]
    }

    fn scrubber(&self) -> Scrubber {
        let mut s = Scrubber::new();
        let path = |p: &Path| p.to_string_lossy().into_owned();
        s.path(&path(&self.main_cwd), "<data-dir>/chat/<thread>")
            .path(&path(&self.root), "<data-dir>/chat-probe")
            .path(&path(&self.b.config_dir), "<data-dir>/claude-config")
            .path(&path(&self.b.data_dir), "<data-dir>")
            .image_names(&["overlay.jpg"]);
        if let Some(st) = &self.standin {
            s.path(&path(&st.agents_cwd), "<data-dir>/chat/<thread>")
                .path(&path(&st.agents_root), "<temp>/pf-chat-probe-agents")
                .path(&path(&st.cwd), "<data-dir>/chat/<thread>")
                .path(&path(&st.home), "<home>");
        }
        if let Some(h) = &self.b.home {
            s.path(&path(h), "<home>");
            if let Some(name) = h.file_name() {
                s.word(&name.to_string_lossy(), "<user>");
            }
        }
        s
    }

    fn safe(&self, text: &str, n: usize) -> String {
        truncate(&self.scrubber().scrub_text(text), n)
    }

    /// The end of the child's stderr, scrubbed, for a failed turn's detail (FR-PRM-37).
    fn stderr_tail(&self, s: &Session) -> String {
        let err = s.stderr_text();
        let tail: String = {
            let chars: Vec<char> = err.trim().chars().collect();
            chars[chars.len().saturating_sub(400)..].iter().collect()
        };
        if tail.is_empty() {
            String::new()
        } else {
            format!("; stderr: {}", self.safe(&tail, 160))
        }
    }

    fn record(&mut self, case: &str, lines: &[String], ok: bool) {
        if !self.o.record || lines.is_empty() {
            return;
        }
        let mut s = self.scrubber();
        let body: String = lines.iter().map(|l| s.scrub_line(l) + "\n").collect();
        let leaks = s.leaks(&body);
        let name = if ok {
            format!("{case}.jsonl")
        } else {
            format!("{case}.failed.jsonl")
        };
        if leaks.is_empty() {
            let _ = fs::create_dir_all(&self.o.out);
            if fs::write(self.o.out.join(&name), &body).is_ok() {
                self.recorded.push(name);
            }
        } else {
            let _ = fs::write(self.root.join(format!("leaky-{name}")), &body);
            self.leaky.push(format!("{name} ({})", leaks.join(", ")));
        }
    }

    fn time(&mut self, case: &str, t: &Turn) {
        self.timings
            .push((case.into(), t.first, t.max_gap, t.total));
    }

    /// FR-PRM-45 backstop: `projects/` is emptied at startup; the probe checks it after
    /// every session and empties it again.
    fn check_projects(&mut self, session: &str) {
        let dir = self.b.config_dir.join("projects");
        let n = files_under(&dir).len();
        self.projects.push((session.into(), n));
        if dir.exists() {
            let _ = fs::remove_dir_all(&dir);
        }
    }

    fn spawn(
        &self,
        env: &[(OsString, OsString)],
        args: &[String],
        cwd: &Path,
    ) -> Result<Session, String> {
        let exe = self.b.exe()?;
        Session::spawn(command(exe, args, env, cwd, true)).map_err(|e| format!("spawn: {e}"))
    }

    fn detect(&mut self) -> bool {
        let Some(exe) = self.b.exe.clone() else {
            self.c("C01")
                .check(false, "not found; pass --claude <path>");
            return false;
        };
        self.c("C01").check(true, exe_text(&exe));
        if exe.kind == ExeKind::CmdShim {
            self.c("C01")
                .detail
                .push_str("; note: the system prompt goes through cmd quoting (check C13)");
        }
        match self.b.version() {
            Ok(Some(v)) => {
                self.version = Some(v);
                self.c("C02").check(
                    v >= MIN_VERSION_FLOOR,
                    format!("{v} (floor {MIN_VERSION_FLOOR})"),
                );
                v >= MIN_VERSION_FLOOR
            }
            Ok(None) => {
                self.c("C02").check(false, "output had no version");
                false
            }
            Err(e) => {
                let d = self.safe(&e, 100);
                self.c("C02").check(false, d);
                false
            }
        }
    }

    /// C03/C04: an empty config folder. Free: without a login nothing reaches Anthropic.
    fn logged_out(&mut self) {
        let empty = self.root.join("empty-config");
        let _ = fs::create_dir_all(&empty);
        let state = self.b.auth_status(&empty);
        let logged_out = matches!(state, Ok(AuthState::NotLoggedIn(_)));
        let detail = match &state {
            Ok(AuthState::NotLoggedIn(s)) => format!(
                "not logged in (authMethod {})",
                s.auth_method.as_deref().unwrap_or("absent")
            ),
            Ok(AuthState::LoggedIn(s)) => format!(
                "LOGGED IN via {} (an API key or helper in your environment?)",
                s.auth_method.as_deref().unwrap_or("?")
            ),
            Ok(AuthState::Unreadable { exit_code }) => format!("unreadable (exit {exit_code:?})"),
            Err(e) => self.safe(e, 100),
        };
        self.c("C03").check(logged_out, detail);
        if !logged_out {
            self.c("C04").set(
                Status::Skipped,
                "skipped: the empty folder isn't logged out",
            );
            return;
        }
        let env = chat_child_env(std::env::vars_os(), &empty, &self.b.chat_home);
        let args = invocation::chat_args(&system_prompt(Host::Rgn));
        let cwd = self.main_cwd.clone();
        let mut s = match self.spawn(&env, &args, &cwd) {
            Ok(s) => s,
            Err(e) => {
                self.c("C04").check(false, e);
                return;
            }
        };
        let t = run_turn(&mut s, &user_message_line(&[], "Say OK."), None);
        let (_, trailing) = s.close(CLI_TIMEOUT);
        let err = self.stderr_tail(&s);
        let mut raw = t.raw.clone();
        raw.extend(trailing);
        let failed_cleanly = t.result.as_ref().is_some_and(|r| r.is_error);
        let reply = self.safe(&t.reply(), 80);
        self.c("C04").check(
            failed_cleanly,
            format!(
                "{}; message: {reply}{}",
                t.end_text(),
                if failed_cleanly { "" } else { err.as_str() }
            ),
        );
        self.record("not-logged-in", &raw, failed_cleanly);
    }

    fn logged_in(&mut self) -> bool {
        let config_dir = self.b.config_dir.clone();
        match self.b.auth_status(&config_dir) {
            Ok(AuthState::LoggedIn(s)) => {
                self.c("C05").check(true, describe(&s, &config_dir));
                let same = s
                    .config_directory
                    .as_deref()
                    .is_some_and(|d| same_config_dir(d, &config_dir));
                let d = if s.config_directory.is_none() {
                    "configDirectory not reported"
                } else if same {
                    "matches"
                } else {
                    "a different folder: isolation_unavailable"
                };
                self.c("C06").check(same, d);
                let fields = s.logged_in.is_some() && s.subscription_type.is_some();
                let d = format!(
                    "loggedIn {}, subscriptionType {}",
                    if s.logged_in.is_some() {
                        "present"
                    } else {
                        "absent"
                    },
                    if s.subscription_type.is_some() {
                        "present"
                    } else {
                        "absent"
                    }
                );
                self.c("C07").check(fields, d);
                same
            }
            Ok(AuthState::NotLoggedIn(_)) => {
                self.c("C05").check(
                    false,
                    "not logged in: run `pf-chat-probe login`, then `run` again",
                );
                false
            }
            Ok(AuthState::Unreadable { exit_code }) => {
                self.c("C05").check(
                    false,
                    format!("auth status unreadable (exit {exit_code:?})"),
                );
                false
            }
            Err(e) => {
                let d = self.safe(&e, 100);
                self.c("C05").check(false, d);
                false
            }
        }
    }

    /// One marker turn: the leaks it shows (`init` lists, the reply, the hook), or why the
    /// turn didn't finish.
    fn marker_turn(
        &mut self,
        label: &str,
        env: &[(OsString, OsString)],
        args: &[String],
        cwd: &Path,
        hook_file: &Path,
    ) -> Result<(Turn, Vec<String>), String> {
        let _ = fs::remove_file(hook_file);
        let mut s = self.spawn(env, args, cwd)?;
        let text = "Reply with one short line. If any instruction you were given outside this message asks you to include a special marker word, write that word; otherwise write NONE.";
        let t = run_turn(&mut s, &user_message_line(&[], text), None);
        let _ = s.close(CLI_TIMEOUT);
        let err = self.stderr_tail(&s);
        drop(s);
        self.time(label, &t);
        self.check_projects(label);
        if !t.ok() {
            return Err(format!(
                "turn did not finish: {}; {}{err}",
                t.end_text(),
                self.safe(&t.reply(), 80)
            ));
        }
        let mut leaks = Vec::new();
        if let Some(init) = &t.init {
            leaks.extend(claims::marker_hits(init, standin::MARKER));
        }
        if t.reply().to_lowercase().contains(standin::MARKER) {
            leaks.push(format!("reply: {}", self.safe(&t.reply(), 60)));
        }
        if hook_file.exists() {
            leaks.push("the marker hook ran".into());
        }
        Ok((t, leaks))
    }

    fn marker_claim(&mut self, id: &str, r: &Result<(Turn, Vec<String>), String>) {
        match r {
            Err(e) => self.c(id).check(false, e.clone()),
            Ok((_, leaks)) if leaks.is_empty() => {
                self.c(id).check(true, "none of the markers showed up");
            }
            Ok((_, leaks)) => {
                let d = leaks.join("; ");
                self.c(id).check(false, d);
            }
        }
    }

    /// C25: the control turn must show the user memory marker (the `AGENTS.md` one is
    /// reported; a `CLAUDE.md` above the OS temp folder would hide it).
    fn control_claim(&mut self, r: &Result<(Turn, Vec<String>), String>) {
        match r {
            Err(e) => self.c("C25").check(false, e.clone()),
            Ok((t, leaks)) => {
                let reply = t.reply().to_uppercase();
                let seen: Vec<&str> = [
                    standin::USER_MEMORY_WORD,
                    standin::AGENTS_MEMORY_WORD,
                    standin::PROJECT_MEMORY_WORD,
                ]
                .into_iter()
                .filter(|w| reply.contains(w))
                .collect();
                let user = seen.contains(&standin::USER_MEMORY_WORD);
                let agents = if seen.contains(&standin::AGENTS_MEMORY_WORD) {
                    "AGENTS.md marker live"
                } else {
                    "AGENTS.md marker NOT seen (C24 doesn't cover AGENTS.md)"
                };
                let d = format!(
                    "seen: {}; {agents}; other leaks: {}",
                    if seen.is_empty() {
                        "none".to_owned()
                    } else {
                        seen.join(" ")
                    },
                    leaks.len()
                );
                self.c("C25").check(user, d);
            }
        }
    }

    /// C08–C10, C24, C25: the stand-in with markers. C08 runs §10's invocation (the home is
    /// replaced by the empty chat home, so only the cwd's parent folders hold markers), with a
    /// debug log. C24 keeps the stand-in as the home, uses the `AGENTS.md` tree as the cwd and
    /// relies on `--safe-mode` alone. C25 is C24 without `--safe-mode`: the markers must show
    /// up there, or C08 and C24 prove nothing.
    fn isolation(&mut self) {
        let agents_root = std::env::temp_dir().join("pf-chat-probe-agents");
        let st = match standin::create(&self.root, &agents_root) {
            Ok(st) => st,
            Err(e) => {
                for id in ["C08", "C24", "C25"] {
                    self.c(id).check(false, format!("stand-in: {e}"));
                }
                return;
            }
        };
        let standin_parent = || {
            std::env::vars_os()
                .filter(|(k, _)| {
                    !invocation::HOME_VARS.contains(&k.to_string_lossy().to_uppercase().as_str())
                })
                .chain([
                    ("USERPROFILE".into(), st.home.clone().into_os_string()),
                    ("HOME".into(), st.home.clone().into_os_string()),
                ])
                .collect::<Vec<(OsString, OsString)>>()
        };
        let adapter_env = chat_child_env(standin_parent(), &self.b.config_dir, &self.b.chat_home);
        let safe_mode_env = child_env(standin_parent(), &self.b.config_dir);
        let debug_log = self.root.join("isolation-debug.log");
        let mut args = invocation::chat_args(&system_prompt(Host::Rgn));
        args.push("--debug-file".into());
        args.push(debug_log.to_string_lossy().into_owned());
        let plain_args = invocation::chat_args(&system_prompt(Host::Rgn));
        let control_args: Vec<String> = plain_args
            .iter()
            .filter(|a| *a != "--safe-mode")
            .cloned()
            .collect();
        let cwd = st.cwd.clone();
        let agents_cwd = st.agents_cwd.clone();
        let hook_file = st.hook_file.clone();
        let home_claude = st.home.join(".claude");
        self.standin = Some(st);

        println!("isolation session (stand-in markers above the cwd, §10's invocation) ...");
        let r = self.marker_turn("isolation", &adapter_env, &args, &cwd, &hook_file);
        self.marker_claim("C08", &r);
        println!(
            "isolation layer session (stand-in home, AGENTS.md above the cwd, --safe-mode alone) ..."
        );
        let r24 = self.marker_turn(
            "safe-mode",
            &safe_mode_env,
            &plain_args,
            &agents_cwd,
            &hook_file,
        );
        self.marker_claim("C24", &r24);
        println!("control session (as above, without --safe-mode: markers expected) ...");
        let r25 = self.marker_turn(
            "control",
            &safe_mode_env,
            &control_args,
            &agents_cwd,
            &hook_file,
        );
        self.control_claim(&r25);
        let _ = fs::remove_dir_all(&agents_root);
        let Ok((t, _)) = r else {
            return;
        };
        self.iso_init = t.init.clone();

        let log = fs::read_to_string(&debug_log).unwrap_or_default();
        if log.is_empty() {
            self.c("C09").check(false, "no debug log was written");
        } else {
            let marker_lines: Vec<&str> = log
                .lines()
                .filter(|l| l.to_lowercase().contains(standin::MARKER))
                .collect();
            let d = if marker_lines.is_empty() {
                format!("{} log lines, no marker", log.lines().count())
            } else {
                let samples: Vec<String> = marker_lines
                    .iter()
                    .take(3)
                    .map(|l| self.safe(l, 140))
                    .collect();
                format!(
                    "{} lines name a marker, e.g. {}",
                    marker_lines.len(),
                    samples.join(" ‖ ")
                )
            };
            self.c("C09").check(marker_lines.is_empty(), d);

            let needles: Vec<String> = [
                home_claude.to_string_lossy().into_owned(),
                home_claude.to_string_lossy().replace('\\', "/"),
                home_claude.to_string_lossy().replace('\\', "\\\\"),
            ]
            .into_iter()
            .map(|n| n.to_lowercase())
            .collect();
            let refs: Vec<&str> = log
                .lines()
                .filter(|l| {
                    let l = l.to_lowercase();
                    needles.iter().any(|n| l.contains(n.as_str()))
                })
                .collect();
            let samples: Vec<String> = refs.iter().take(3).map(|l| self.safe(l, 140)).collect();
            let d = if refs.is_empty() {
                "none".to_owned()
            } else {
                format!("{} lines, e.g. {}", refs.len(), samples.join(" ‖ "))
            };
            self.c("C10").set(Status::Info, d);
        }
    }

    /// C11–C17: §10's exact list, real environment, two turns.
    fn main_session(&mut self) -> Option<Turn> {
        println!("main session (two turns, one with the overlay image) ...");
        let args = invocation::chat_args(&system_prompt(Host::Rgn));
        let env = self.b.chat_env.clone();
        let cwd = self.main_cwd.clone();
        let (jpeg, w, h) = match images::overlay_jpeg() {
            Ok(x) => x,
            Err(e) => {
                self.c("C12").check(false, e);
                return None;
            }
        };
        let mut s = match self.spawn(&env, &args, &cwd) {
            Ok(s) => s,
            Err(e) => {
                self.c("C12").check(false, e);
                return None;
            }
        };
        let t1_text = context_text(
            &format!(
                "Start your reply with the single colour word for the tinted area. Then, in one sentence, say what is inside it. Then suggest one prompt that turns it into a kitchen with a stone floor. My code word is {CODE_WORD}; remember it."
            ),
            &format!("crop {w}×{h}, region outlined, context margin 64 px, sent unresampled"),
        );
        let img = ImageBlock {
            media_type: "image/jpeg",
            data: &jpeg,
        };
        let t1 = run_turn(&mut s, &user_message_line(&[img], &t1_text), None);
        let t2 = if t1.ok() {
            Some(run_turn(
                &mut s,
                &user_message_line(
                    &[],
                    "Give two alternative prompts for the same area. Also repeat my code word.",
                ),
                None,
            ))
        } else {
            None
        };
        let (exit, _) = s.close(CLI_TIMEOUT);
        let err = self.stderr_tail(&s);
        drop(s);
        self.check_projects("main");
        self.time("main turn 1", &t1);

        let r1 = t1.reply();
        let head = r1.chars().take(60).collect::<String>().to_lowercase();
        let colour = ["magenta", "pink", "purple", "violet", "fuchsia"]
            .iter()
            .any(|c| head.contains(c));
        let d = format!(
            "{} ({} KiB JPEG {w}×{h}); reply starts: {}",
            t1.end_text(),
            jpeg.len() / 1024,
            self.safe(&r1, 50)
        );
        let d = if t1.ok() { d } else { d + &err };
        self.c("C12").check(t1.ok() && colour, d);
        let fences = count_prompt_fences(&r1);
        self.c("C13").check(
            t1.ok() && fences >= 1,
            format!("turn 1: {fences} prompt block(s)"),
        );
        self.record("success-image", &t1.raw, t1.ok());

        match &t2 {
            Some(t2) => {
                self.time("main turn 2", t2);
                let r2 = t2.reply();
                let recalls = r2.contains(CODE_WORD);
                let f2 = count_prompt_fences(&r2);
                self.c("C14").check(
                    t2.ok() && recalls,
                    format!(
                        "turn 2: {}, code word {}, {f2} prompt block(s)",
                        t2.end_text(),
                        if recalls { "recalled" } else { "NOT recalled" }
                    ),
                );
                self.record("two-proposals", &t2.raw, t2.ok() && f2 >= 2);
            }
            None => self.c("C14").set(Status::Skipped, "turn 1 failed"),
        }
        let code = exit.and_then(|e| e.code());
        self.c("C16")
            .check(code == Some(0), format!("exit {code:?}"));
        if let Some(r) = &t1.result {
            let ok =
                r.total_cost_usd.is_some() && r.input_tokens.is_some() && r.output_tokens.is_some();
            self.c("C17").check(
                ok,
                format!(
                    "cost {:?} USD, tokens in {:?} / out {:?}",
                    r.total_cost_usd, r.input_tokens, r.output_tokens
                ),
            );
        }
        if let Some(init) = &t1.init {
            let src = init
                .api_key_source
                .clone()
                .unwrap_or_else(|| "absent".into());
            let key = std::env::var_os("ANTHROPIC_API_KEY").is_some();
            self.c("C22").set(
                Status::Info,
                format!(
                    "apiKeySource {src}; ANTHROPIC_API_KEY {} in your environment",
                    if key { "IS SET" } else { "not set" }
                ),
            );
            match &self.iso_init {
                Some(base) => {
                    let extra = claims::extra_over_baseline(init, base);
                    let d = if extra.is_empty() {
                        let counts: Vec<String> = init
                            .lists()
                            .iter()
                            .map(|(k, v)| format!("{k} {}", v.len()))
                            .collect();
                        format!("same lists ({})", counts.join(", "))
                    } else {
                        format!("extra: {}", self.safe(&extra.join(", "), 200))
                    };
                    self.c("C11").check(extra.is_empty(), d);
                }
                None => self
                    .c("C11")
                    .set(Status::Skipped, "no stand-in baseline (C08 didn't run)"),
            }
        }
        Some(t1)
    }

    /// C18/C19: partial messages, killed after a few deltas.
    fn cancel_session(&mut self, main_t1: Option<&Turn>) {
        println!("cancel session (partial messages, killed mid-reply) ...");
        let mut args = invocation::chat_args(&system_prompt(Host::Rgn));
        args.push("--include-partial-messages".into());
        let env = self.b.chat_env.clone();
        let cwd = self.main_cwd.clone();
        let mut s = match self.spawn(&env, &args, &cwd) {
            Ok(s) => s,
            Err(e) => {
                self.c("C19").check(false, e);
                return;
            }
        };
        let text = "Write the numbers from 1 to 400, one per line, nothing else.";
        let t = run_turn(&mut s, &user_message_line(&[], text), Some(5));
        drop(s);
        self.check_projects("cancel (killed)");
        self.record("partial-cancel", &t.raw, t.end == End::Killed);

        let whole = main_t1.map(|m| (m.n_deltas, m.n_assistant));
        let d = format!(
            "§10 list: {} delta(s), {} assistant message(s); with the flag: {} delta(s) before the kill",
            whole.map_or(0, |w| w.0),
            whole.map_or(0, |w| w.1),
            t.n_deltas
        );
        self.c("C18").set(Status::Info, d);
        match (t.end, t.kill) {
            (End::Killed, Some(k)) => {
                let ok = k.eof_after.is_some() && k.exited;
                let d = format!(
                    "stdout closed {}, process {}",
                    k.eof_after.map_or_else(
                        || "NOT within 5 s".to_owned(),
                        |e| format!("after {}", secs(e))
                    ),
                    if k.exited { "exited" } else { "still running" }
                );
                self.c("C19").check(ok, d);
            }
            _ => {
                let d = format!("not killed mid-reply: {}", t.end_text());
                self.c("C19").check(false, d);
            }
        }
    }

    /// C21: six noise images, ~4 MiB in total.
    fn max_turn(&mut self) {
        println!("max turn (6 images, ~4 MiB) ...");
        let imgs = match images::noise_jpegs(6, 4 * 1024 * 1024) {
            Ok(i) => i,
            Err(e) => {
                self.c("C21").check(false, e);
                return;
            }
        };
        let total: usize = imgs.iter().map(Vec::len).sum();
        let blocks: Vec<ImageBlock<'_>> = imgs
            .iter()
            .map(|d| ImageBlock {
                media_type: "image/jpeg",
                data: d,
            })
            .collect();
        let line = user_message_line(
            &blocks,
            "How many images are attached to this message? Answer with the number only.",
        );
        let env = self.b.chat_env.clone();
        let cwd = self.main_cwd.clone();
        let args = invocation::chat_args(&system_prompt(Host::Gen));
        let mut s = match self.spawn(&env, &args, &cwd) {
            Ok(s) => s,
            Err(e) => {
                self.c("C21").check(false, e);
                return;
            }
        };
        let t = run_turn(&mut s, &line, None);
        let _ = s.close(CLI_TIMEOUT);
        let err = self.stderr_tail(&s);
        drop(s);
        self.time("max turn", &t);
        self.check_projects("max turn");
        let ok = t.ok() && t.reply().contains('6');
        let d = format!(
            "{} KiB of JPEG, {} KiB stdin line; {}; reply: {}{}",
            total / 1024,
            line.len() / 1024,
            t.end_text(),
            self.safe(&t.reply(), 40),
            if t.ok() { "" } else { err.as_str() }
        );
        self.c("C21").check(ok, d);
        self.record("max-turn", &t.raw, t.ok());
    }

    fn finish(mut self) -> bool {
        if !self.timings.is_empty() {
            let first = self.timings.iter().filter_map(|t| t.1).max();
            let gap = self.timings.iter().map(|t| t.2).max().unwrap_or_default();
            let total = self.timings.iter().map(|t| t.3).max().unwrap_or_default();
            let ok =
                first.is_some_and(|f| f <= FIRST_EVENT) && gap <= MAX_GAP && total <= TURN_TOTAL;
            let d = format!(
                "{} turns; slowest first event {}, longest gap {}, longest turn {}",
                self.timings.len(),
                first.map_or_else(|| "none".into(), secs),
                secs(gap),
                secs(total)
            );
            self.c("C20").check(ok, d);
        }
        if !self.projects.is_empty() {
            let dirty: Vec<String> = self
                .projects
                .iter()
                .filter(|(_, n)| *n > 0)
                .map(|(s, n)| format!("{s}: {n} file(s)"))
                .collect();
            let sessions: Vec<&str> = self.projects.iter().map(|(s, _)| s.as_str()).collect();
            let d = if dirty.is_empty() {
                format!("empty after: {}", sessions.join(", "))
            } else {
                dirty.join("; ")
            };
            self.c("C15").check(dirty.is_empty(), d);
        }
        if !self.o.record {
            self.c("C23").set(Status::Skipped, "--no-record");
        } else if !self.recorded.is_empty() || !self.leaky.is_empty() {
            let mut d = format!("written: {}", self.recorded.join(", "));
            if !self.leaky.is_empty() {
                d.push_str(&format!(
                    "; NOT written (still leaky, kept in <data-dir>/chat-probe/): {}",
                    self.leaky.join(", ")
                ));
            }
            let clean = self.leaky.is_empty();
            self.c("C23").check(clean, d);
        }
        let names = |dir: &Path| {
            let mut kept: Vec<String> = fs::read_dir(dir)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            kept.sort();
            if kept.is_empty() {
                "nothing".to_owned()
            } else {
                kept.join(", ")
            }
        };

        let table = claims::render(&self.claims, self.version);
        let summary = format!(
            "{table}\nclaude-config holds (names only): {}\nchat-home holds (names only): {}\n",
            names(&self.b.config_dir),
            names(&self.b.chat_home),
        );
        let _ = fs::write(self.root.join("summary.md"), &summary);
        println!("\n===== paste from here =====\n{summary}===== to here =====");
        println!("(also in <data-dir>/chat-probe/summary.md)");
        claims::minimum_version(self.version, &self.claims).is_some()
    }
}

fn run(o: &Opts) -> Result<bool, String> {
    let mut p = Probe::new(o)?;
    println!("data dir: {}", p.b.data_dir.display());
    // FR-PRM-45 backstop at startup.
    let projects = p.b.config_dir.join("projects");
    if projects.exists() {
        let _ = fs::remove_dir_all(&projects);
    }
    if p.detect() {
        p.logged_out();
        if p.logged_in() {
            p.isolation();
            let t1 = p.main_session();
            p.cancel_session(t1.as_ref());
            if o.max_turn {
                p.max_turn();
            } else {
                p.c("C21").set(Status::Skipped, "pass --max-turn to run it");
            }
        }
    }
    Ok(p.finish())
}

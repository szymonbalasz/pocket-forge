//! The Claude Code adapter (§10): executable resolution, the isolated config and the empty
//! chat home (FR-PRM-45, R87), detection with the allowed `auth status` fields only
//! (FR-PRM-33), the login console and logout, §10's invocation (FR-PRM-34), turns over the
//! stream-json parser with FR-PRM-36's budgets, cancel, and the `projects/` backstop.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::auth::{AuthState, parse_auth_status, same_config_dir};
use crate::backend::{
    BackendMode, BackendOk, CancelHandle, ChatBackend, ChatSession, ConsoleCommand, SessionOptions,
    TurnEvent, TurnInput, TurnOutcome,
};
use crate::clock::{Clock, TurnBudget};
use crate::error::{ChatError, ChatErrorKind};
use crate::invocation::{self, ExeKind, Executable, chat_child_env, child_env, resolve_executable};
use crate::message::{ImageBlock, user_message_line};
use crate::process::{ChildProcess, CommandSpec, Spawner};
use crate::scrub::Scrubber;
use crate::system_prompt::{Host, PROMPT_CHAT_SYSTEM_VERSION, system_prompt};
use crate::turn;
use crate::version::{MIN_VERSION_FLOOR, parse_version};

/// `--version`, `auth status` and `auth logout` are killed after this.
pub const CLI_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a closing child gets after stdin closes.
pub const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// The folder for the npm shim's system prompt files, inside `<data-dir>`.
pub const PROMPT_DIR_NAME: &str = "chat-prompt";

/// Where the adapter works. `parent_env` is the app's environment (minus nothing: the
/// child environments drop `PF_*` themselves).
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    pub data_dir: PathBuf,
    /// `prompt_chat.claude_path` (Settings).
    pub claude_path: Option<PathBuf>,
    pub parent_env: Vec<(OsString, OsString)>,
    pub windows: bool,
}

impl ClaudeCodeConfig {
    /// The running app's environment.
    pub fn from_process(data_dir: PathBuf, claude_path: Option<PathBuf>) -> Self {
        Self {
            data_dir,
            claude_path,
            parent_env: std::env::vars_os().collect(),
            windows: cfg!(windows),
        }
    }

    fn var(&self, name: &str) -> Option<&OsStr> {
        self.parent_env
            .iter()
            .find(|(k, _)| {
                let k = k.to_string_lossy();
                if self.windows {
                    k.eq_ignore_ascii_case(name)
                } else {
                    k == name
                }
            })
            .map(|(_, v)| v.as_os_str())
    }

    /// The user's real home (`USERPROFILE` on Windows, else `HOME`).
    pub fn home(&self) -> Option<PathBuf> {
        self.var(if self.windows { "USERPROFILE" } else { "HOME" })
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    }

    pub fn config_dir(&self) -> PathBuf {
        invocation::config_dir(&self.data_dir)
    }

    pub fn chat_home(&self) -> PathBuf {
        invocation::chat_home_dir(&self.data_dir)
    }
}

/// The login command shown for copying (FR-PRM-45). The folder is single-quoted so no `$`,
/// backtick or `"` in it expands (review-sec 5): PowerShell on Windows (`'` doubled),
/// POSIX sh elsewhere (`'` → `'\''`).
pub fn login_copy_text(config_dir: &str, windows: bool) -> String {
    if windows {
        let dir = config_dir.replace('\'', "''");
        format!("$env:CLAUDE_CONFIG_DIR = '{dir}'; claude auth login")
    } else {
        let dir = config_dir.replace('\'', r"'\''");
        format!("CLAUDE_CONFIG_DIR='{dir}' claude auth login")
    }
}

/// The Claude Code backend.
pub struct ClaudeCodeBackend {
    cfg: ClaudeCodeConfig,
    spawner: Arc<dyn Spawner>,
    clock: Arc<dyn Clock>,
    budget: TurnBudget,
}

impl ClaudeCodeBackend {
    pub fn new(cfg: ClaudeCodeConfig, spawner: Arc<dyn Spawner>, clock: Arc<dyn Clock>) -> Self {
        Self {
            cfg,
            spawner,
            clock,
            budget: TurnBudget::default(),
        }
    }

    pub fn with_budget(mut self, budget: TurnBudget) -> Self {
        self.budget = budget;
        self
    }

    pub fn config(&self) -> &ClaudeCodeConfig {
        &self.cfg
    }

    /// §10's order; `not_installed` when nothing is found.
    pub fn resolve(&self) -> Result<Executable, ChatError> {
        let home = self.cfg.home();
        let spawner = Arc::clone(&self.spawner);
        resolve_executable(
            self.cfg.claude_path.as_deref(),
            self.cfg.var("PATH"),
            home.as_deref(),
            self.cfg.windows,
            &move |p: &Path| spawner.is_file(p),
        )
        .ok_or_else(|| ChatError::new(ChatErrorKind::NotInstalled))
    }

    /// Detection and login commands: the real home, `CLAUDE_CONFIG_DIR` set (FR-PRM-45).
    fn detect_env(&self) -> Vec<(OsString, OsString)> {
        child_env(self.cfg.parent_env.iter().cloned(), &self.cfg.config_dir())
    }

    fn one_shot(
        &self,
        exe: &Executable,
        args: Vec<String>,
    ) -> Result<crate::process::Captured, ChatError> {
        let dir = self.cfg.config_dir();
        fs::create_dir_all(&dir).map_err(|e| {
            ChatError::backend_error(format!("create the chat's config folder: {e}"))
        })?;
        let spec = CommandSpec::new(exe, args, self.detect_env(), &self.cfg.data_dir, true);
        let out = self.spawner.run(&spec, CLI_TIMEOUT).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ChatError::new(ChatErrorKind::NotInstalled).with_detail(e.to_string())
            } else {
                ChatError::backend_error(e.to_string())
            }
        })?;
        if out.timed_out {
            return Err(ChatError::new(ChatErrorKind::Timeout));
        }
        Ok(out)
    }

    fn scrubber(&self) -> impl Fn(&str) -> String + use<> {
        let home = self.cfg.home().map(|h| h.to_string_lossy().into_owned());
        move |text: &str| {
            let mut s = Scrubber::new();
            if let Some(h) = &home {
                s.path(h, "<home>");
            }
            s.scrub_text(text)
        }
    }

    /// The npm shim gets the system prompt as a file (`--system-prompt-file`), written once
    /// per host and prompt version.
    fn prompt_file(&self, host: Host) -> Result<PathBuf, ChatError> {
        let dir = self.cfg.data_dir.join(PROMPT_DIR_NAME);
        let name = match host {
            Host::Rgn => "rgn",
            Host::Gen => "gen",
        };
        let path = dir.join(format!("system-{name}-v{PROMPT_CHAT_SYSTEM_VERSION}.txt"));
        let text = system_prompt(host);
        if fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            fs::create_dir_all(&dir)
                .and_then(|()| fs::write(&path, text))
                .map_err(|e| {
                    ChatError::backend_error(format!("write the system prompt file: {e}"))
                })?;
        }
        Ok(path)
    }

    /// The exact spawn for a session (FR-PRM-34): §10's arguments, the chat child
    /// environment (`CLAUDE_CONFIG_DIR`, home → the empty chat home, no `PF_*`), the
    /// thread folder as cwd, no window.
    pub fn session_spec(
        &self,
        exe: &Executable,
        options: &SessionOptions,
    ) -> Result<CommandSpec, ChatError> {
        let args = match exe.kind {
            ExeKind::Native => invocation::chat_args(&system_prompt(options.first_host)),
            ExeKind::CmdShim => {
                let file = self.prompt_file(options.first_host)?;
                invocation::chat_args_with_prompt_file(&file.to_string_lossy())
            }
        };
        let env = chat_child_env(
            self.cfg.parent_env.iter().cloned(),
            &self.cfg.config_dir(),
            &self.cfg.chat_home(),
        );
        Ok(CommandSpec::new(exe, args, env, &options.cwd, true))
    }

    /// The chat home must exist and be empty (R87): anything in it is removed.
    fn prepare_chat_home(&self) -> Result<(), ChatError> {
        let home = self.cfg.chat_home();
        if home.exists() {
            fs::remove_dir_all(&home)
                .map_err(|e| ChatError::backend_error(format!("empty the chat home: {e}")))?;
        }
        fs::create_dir_all(&home)
            .and_then(|()| fs::create_dir_all(self.cfg.config_dir()))
            .map_err(|e| ChatError::backend_error(format!("create the chat folders: {e}")))
    }
}

impl ChatBackend for ClaudeCodeBackend {
    fn mode(&self) -> BackendMode {
        BackendMode::ClaudeCode
    }

    fn detect(&self) -> Result<BackendOk, ChatError> {
        let exe = self.resolve()?;
        let out = self.one_shot(&exe, invocation::version_args())?;
        let version = parse_version(&out.stdout)
            .ok_or_else(|| ChatError::backend_error("`claude --version` printed no version"))?;
        if version < MIN_VERSION_FLOOR {
            return Err(ChatError::version_unsupported(version));
        }
        let out = self.one_shot(&exe, invocation::auth_status_args())?;
        let config_dir = self.cfg.config_dir();
        let isolated = |dir: &Option<String>| {
            dir.as_deref()
                .is_some_and(|d| same_config_dir(d, &config_dir))
        };
        match parse_auth_status(out.code, &out.stdout) {
            // Fail closed first: a foreign folder means the isolation doesn't hold.
            AuthState::NotLoggedIn(s)
                if s.config_directory.is_some() && !isolated(&s.config_directory) =>
            {
                Err(ChatError::new(ChatErrorKind::IsolationUnavailable))
            }
            AuthState::NotLoggedIn(_) => Err(ChatError::new(ChatErrorKind::NotLoggedIn)),
            AuthState::LoggedIn(s) if !isolated(&s.config_directory) => {
                Err(ChatError::new(ChatErrorKind::IsolationUnavailable))
            }
            AuthState::LoggedIn(s) => Ok(BackendOk {
                version: version.to_string(),
                auth_method: s.auth_method,
                plan: s.subscription_type,
            }),
            AuthState::Unreadable { exit_code } => Err(ChatError::backend_error(format!(
                "`claude auth status` gave unexpected output (exit {})",
                exit_code.map_or_else(|| "none".to_owned(), |c| c.to_string())
            ))),
        }
    }

    fn login_command(&self) -> Result<Option<ConsoleCommand>, ChatError> {
        let exe = self.resolve()?;
        let config_dir = self.cfg.config_dir();
        fs::create_dir_all(&config_dir).map_err(|e| {
            ChatError::backend_error(format!("create the chat's config folder: {e}"))
        })?;
        let spec = CommandSpec::new(
            &exe,
            invocation::auth_login_args(),
            self.detect_env(),
            &self.cfg.data_dir,
            false,
        );
        let copy_text = login_copy_text(&config_dir.to_string_lossy(), self.cfg.windows);
        Ok(Some(ConsoleCommand { spec, copy_text }))
    }

    fn open_login_console(&self) -> Result<(), ChatError> {
        let Some(cmd) = self.login_command()? else {
            return Ok(());
        };
        self.spawner
            .open_console(&cmd.spec)
            .map_err(|e| ChatError::backend_error(format!("open the login console: {e}")))
    }

    fn logout(&self) -> Result<(), ChatError> {
        let exe = self.resolve()?;
        let out = self.one_shot(&exe, invocation::auth_logout_args())?;
        match out.code {
            Some(0) => Ok(()),
            code => Err(ChatError::backend_error(format!(
                "`claude auth logout` failed (exit {})",
                code.map_or_else(|| "none".to_owned(), |c| c.to_string())
            ))),
        }
    }

    fn open_session(&self, options: SessionOptions) -> Result<Box<dyn ChatSession>, ChatError> {
        let exe = self.resolve()?;
        self.prepare_chat_home()?;
        fs::create_dir_all(&options.cwd)
            .map_err(|e| ChatError::backend_error(format!("create the thread folder: {e}")))?;
        let spec = self.session_spec(&exe, &options)?;
        let child = self.spawner.spawn(&spec).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ChatError::new(ChatErrorKind::NotInstalled).with_detail(e.to_string())
            } else {
                ChatError::backend_error(format!("start Claude Code: {e}"))
            }
        })?;
        Ok(Box::new(ClaudeCodeSession {
            child: Some(child),
            clock: Arc::clone(&self.clock),
            budget: self.budget,
            data_dir: self.cfg.data_dir.clone(),
            scrub: Box::new(self.scrubber()),
        }))
    }
}

/// One Claude Code child (FR-PRM-35).
pub struct ClaudeCodeSession {
    child: Option<Box<dyn ChildProcess>>,
    clock: Arc<dyn Clock>,
    budget: TurnBudget,
    data_dir: PathBuf,
    scrub: Box<dyn Fn(&str) -> String + Send>,
}

impl ClaudeCodeSession {
    /// The child is gone or unusable: make sure it's dead, then the backstop.
    fn end(&mut self) {
        if let Some(mut c) = self.child.take()
            && c.wait(Duration::ZERO).is_none()
        {
            c.killer().kill();
            let _ = c.wait(CLOSE_WAIT);
        }
        let _ = invocation::sweep_projects(&self.data_dir);
    }
}

impl ChatSession for ClaudeCodeSession {
    fn run_turn(
        &mut self,
        input: &TurnInput,
        cancel: &CancelHandle,
        on_event: &mut dyn FnMut(TurnEvent),
    ) -> TurnOutcome {
        let Some(child) = self.child.as_mut() else {
            return TurnOutcome::Failed {
                error: ChatError::new(ChatErrorKind::BackendCrashed { exit_code: None })
                    .with_detail("the session has ended; start a new one"),
                partial: String::new(),
            };
        };
        let blocks: Vec<ImageBlock<'_>> = input
            .images
            .iter()
            .map(|i| ImageBlock {
                media_type: &i.media_type,
                data: &i.bytes,
            })
            .collect();
        let line = user_message_line(&blocks, &input.message_text);
        let (outcome, alive) = turn::run_turn(
            child.as_mut(),
            &line,
            self.clock.as_ref(),
            self.budget,
            cancel,
            on_event,
            &self.scrub,
        );
        if !alive {
            self.end();
        }
        outcome
    }

    fn is_alive(&self) -> bool {
        self.child.is_some()
    }

    fn close(&mut self) {
        if let Some(c) = self.child.as_mut() {
            c.close_stdin();
            if c.wait(CLOSE_WAIT).is_some() {
                self.child = None;
            }
        }
        self.end();
    }
}

impl Drop for ClaudeCodeSession {
    fn drop(&mut self) {
        self.close();
    }
}

//! The chat backend trait (§10, FR-PRM-33..39) that pf-core's chat (slice 5) drives, and
//! the backend mode switch (§13).
//!
//! The API is synchronous, like pf-core's commands: `run_turn` blocks on its own thread and
//! reports text through a callback (`prm:chat-delta`); Stop goes through a
//! [`CancelHandle`] from another thread.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::error::ChatError;
use crate::process::{CommandSpec, Kill};
use crate::system_prompt::Host;

/// `prm_chat_status.mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendMode {
    Fake,
    ClaudeCode,
}

/// Selects the real backend (§13): `PF_CHAT_BACKEND=claude-code`.
pub const ENV_BACKEND: &str = "PF_CHAT_BACKEND";
/// …together with `PF_CHAT_REAL=1` in a non-release build.
pub const ENV_REAL: &str = "PF_CHAT_REAL";
/// The fake backend's case at startup (FR-PRM-39).
pub const ENV_FAKE_CASE: &str = "PF_FAKE_CHAT_CASE";

/// The backend for this run (§13, as FR-PRV-28): release builds use Claude Code; dev and
/// test builds use the fake unless both `PF_CHAT_BACKEND=claude-code` and `PF_CHAT_REAL=1`
/// are set. Under CI (`CI` set and not empty) either variable is an error.
pub fn backend_mode(
    get: &dyn Fn(&str) -> Option<String>,
    release: bool,
) -> Result<BackendMode, String> {
    let set = |k: &str| get(k).filter(|v| !v.trim().is_empty());
    if set("CI").is_some() && (set(ENV_BACKEND).is_some() || set(ENV_REAL).is_some()) {
        return Err(format!(
            "{ENV_BACKEND} / {ENV_REAL} are set in CI: real chat calls never run there (PRM §13)"
        ));
    }
    if release {
        return Ok(BackendMode::ClaudeCode);
    }
    let real =
        set(ENV_BACKEND).as_deref() == Some("claude-code") && set(ENV_REAL).as_deref() == Some("1");
    Ok(if real {
        BackendMode::ClaudeCode
    } else {
        BackendMode::Fake
    })
}

/// FR-PRM-33 `ok{version, auth_method, plan?}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackendOk {
    pub version: String,
    pub auth_method: Option<String>,
    pub plan: Option<String>,
}

impl BackendOk {
    /// FR-PRM-38: plan usage (`claude.ai`) is notional, not billed per call.
    pub fn notional(&self) -> bool {
        self.auth_method.as_deref() == Some("claude.ai")
    }
}

/// The login console (FR-PRM-45): what to start, and the command shown for copying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleCommand {
    pub spec: CommandSpec,
    pub copy_text: String,
}

/// How a backend session starts (FR-PRM-34, FR-PRM-44).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOptions {
    /// The host of the session's first turn: picks the system prompt's host paragraph.
    pub first_host: Host,
    /// `<data-dir>/chat/<thread-id>/` (the core creates and fills it).
    pub cwd: PathBuf,
}

/// One image of a turn, already downscaled and encoded (FR-PRM-25).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnImage {
    pub id: String,
    pub content_hash: String,
    pub width: u32,
    pub height: u32,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// One turn. The real backend sends `message_text` (host note + context block + user text,
/// composed by the core) and the images; the fake lists the structured fields (FR-PRM-39).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnInput {
    pub host: Host,
    pub message_text: String,
    pub user_text: String,
    pub model_id: Option<String>,
    pub final_prompt: Option<String>,
    pub images: Vec<TurnImage>,
}

/// Streamed while a turn runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEvent {
    /// More reply text (`prm:chat-delta`). The reply is the concatenation of the deltas.
    Delta(String),
}

/// FR-PRM-38 usage from the `result` message (`notional` comes from detection).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
}

/// How a turn ended.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnOutcome {
    Completed {
        text: String,
        usage: Option<Usage>,
    },
    /// The partial text is kept (FR-PRM-36, FR-PRM-37).
    Failed {
        error: ChatError,
        partial: String,
    },
    /// Stop (FR-PRM-27): the partial reply is kept; no proposals come from it.
    Canceled {
        partial: String,
    },
}

#[derive(Default)]
struct CancelInner {
    requested: AtomicBool,
    killer: Mutex<Option<Arc<dyn Kill>>>,
}

/// Stop for one turn. Create one per turn before starting it and keep a clone for
/// `prm_chat_cancel`; a cancel that arrives before the turn starts still counts.
#[derive(Clone, Default)]
pub struct CancelHandle(Arc<CancelInner>);

impl std::fmt::Debug for CancelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelHandle")
            .field("requested", &self.is_cancelled())
            .finish()
    }
}

impl CancelHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests the stop and kills the turn's child (its whole job on Windows).
    pub fn cancel(&self) {
        self.0.requested.store(true, Ordering::SeqCst);
        let killer = self.0.killer.lock().ok().and_then(|k| k.clone());
        if let Some(k) = killer {
            k.kill();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.requested.load(Ordering::SeqCst)
    }

    /// Binds the running child; kills it at once if the stop came first.
    pub(crate) fn arm(&self, killer: Arc<dyn Kill>) {
        if let Ok(mut k) = self.0.killer.lock() {
            *k = Some(Arc::clone(&killer));
        }
        if self.is_cancelled() {
            killer.kill();
        }
    }
}

/// A chat backend: the Claude Code adapter or the fake.
pub trait ChatBackend: Send + Sync {
    fn mode(&self) -> BackendMode;

    /// FR-PRM-33 detection (no caching here: the core caches for 60 s).
    fn detect(&self) -> Result<BackendOk, ChatError>;

    /// The login console (FR-PRM-45); `None` when there is nothing to log in to (fake).
    fn login_command(&self) -> Result<Option<ConsoleCommand>, ChatError>;

    /// Opens the login console (the Tauri layer's `prm_chat_login`).
    fn open_login_console(&self) -> Result<(), ChatError>;

    /// `claude auth logout` for the chat's own folder (FR-PRM-45).
    fn logout(&self) -> Result<(), ChatError>;

    /// Starts one backend session (one child, FR-PRM-35).
    fn open_session(&self, options: SessionOptions) -> Result<Box<dyn ChatSession>, ChatError>;
}

/// One backend session: one child process for as long as it lives (FR-PRM-35).
pub trait ChatSession: Send {
    /// Runs one turn to its end. After a cancel, timeout, crash or malformed stream the
    /// session is over ([`ChatSession::is_alive`] is false) and the next turn needs a fresh
    /// session.
    fn run_turn(
        &mut self,
        input: &TurnInput,
        cancel: &CancelHandle,
        on_event: &mut dyn FnMut(TurnEvent),
    ) -> TurnOutcome;

    fn is_alive(&self) -> bool;

    /// Ends the session: stdin closes, the child gets 5 s to exit, then it is killed;
    /// `<data-dir>/claude-config/projects/` is emptied (FR-PRM-45).
    fn close(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k: &str| m.get(k).cloned()
    }

    #[test]
    fn prm_s13_backend_mode_needs_both_variables_and_never_runs_in_ci() {
        assert_eq!(backend_mode(&env(&[]), false), Ok(BackendMode::Fake));
        assert_eq!(
            backend_mode(&env(&[(ENV_BACKEND, "claude-code")]), false),
            Ok(BackendMode::Fake)
        );
        assert_eq!(
            backend_mode(
                &env(&[(ENV_BACKEND, "claude-code"), (ENV_REAL, "1")]),
                false
            ),
            Ok(BackendMode::ClaudeCode)
        );
        assert_eq!(backend_mode(&env(&[]), true), Ok(BackendMode::ClaudeCode));
        assert!(backend_mode(&env(&[("CI", "true"), (ENV_REAL, "1")]), false).is_err());
        assert!(backend_mode(&env(&[("CI", "true"), (ENV_BACKEND, "fake")]), true).is_err());
        assert_eq!(
            backend_mode(&env(&[("CI", "true")]), false),
            Ok(BackendMode::Fake)
        );
    }

    #[test]
    fn fr_prm_27_cancel_before_arm_kills_on_arm() {
        #[derive(Default)]
        struct K(AtomicBool);
        impl Kill for K {
            fn kill(&self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let h = CancelHandle::new();
        h.clone().cancel();
        let k = Arc::new(K::default());
        h.arm(k.clone());
        assert!(k.0.load(Ordering::SeqCst));
    }
}

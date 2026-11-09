//! The prompt chat core (PRM slice 5, spec Part B): consent (FR-PRM-20), detection with a 60 s
//! cache (FR-PRM-33), the live context published by the visible tab (FR-PRM-23/43), the thread
//! (FR-PRM-22), send / cancel / insert (FR-PRM-27..29), image limits and re-sending
//! (FR-PRM-25), sessions (FR-PRM-35) and the events. Nothing here calls the provider
//! (FR-PRM-31).
//!
//! **Locks.** Chat state lives in [`ChatRuntime`], beside the core's state lock: `rt` (status
//! cache, published contexts, the running turn), `session` (the backend session; a turn holds it
//! while it runs) and `thread_io` (read-modify-write of the thread file). A turn runs on its own
//! thread and never takes the core's state lock. Nothing waits for `session` while holding `rt`:
//! ending a session cancels the running turn first, so the wait is short.

pub mod message;
pub mod thread;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use pf_chat::backend::{
    BackendMode, CancelHandle, ChatBackend, ChatSession, SessionOptions, TurnEvent, TurnImage,
    TurnInput, TurnOutcome,
};
use pf_chat::claude_code::{ClaudeCodeBackend, ClaudeCodeConfig};
use pf_chat::error::{ChatError, ChatErrorKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ulid::Ulid;

use crate::context::Context;
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::prompt::context::{ChatContextSource, ChatHost, ChatImage, limit_images};
use crate::settings::{self, ChatConsent, ChatWindowRect, PromptChatSettings};
use crate::time::Timestamp;
use thread::{
    Proposal, Role, SentImageRecord, Thread, Turn, TurnContext, TurnError, TurnState, TurnUsage,
};

/// Appendix C's version. A higher one asks for consent again (FR-PRM-20).
pub const NOTICE_VERSION: u32 = 1;
/// FR-PRM-33: detection is cached this long.
pub const STATUS_TTL_MS: i64 = 60_000;
/// `prm_chat_send`'s text limit.
pub const MAX_TEXT_CHARS: usize = 8000;

pub const EVENT_DELTA: &str = "prm:chat-delta";
pub const EVENT_TURN: &str = "prm:chat-turn";
pub const EVENT_STATUS: &str = "prm:chat-status";
pub const EVENT_INSERT: &str = "prm:insert";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------- backend status

/// `prm_chat_status.backend`: `not_checked`, `ok{…}` or an FR-PRM-37 error.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BackendStatus {
    NotChecked,
    Ok {
        version: String,
        auth_method: Option<String>,
        plan: Option<String>,
        /// FR-PRM-38: plan usage, not billed per call.
        notional: bool,
    },
    Error {
        #[serde(flatten)]
        error: ChatError,
    },
}

/// A backend error as a core error with the same code (FR-PRM-37); `detail` holds the whole
/// error (message, fix, the scrubbed detail). Never written to a project.
pub fn core_error(e: &ChatError) -> CoreError {
    let code = match e.kind {
        ChatErrorKind::NotInstalled => ErrorCode::NotInstalled,
        ChatErrorKind::VersionUnsupported { .. } => ErrorCode::VersionUnsupported,
        ChatErrorKind::IsolationUnavailable => ErrorCode::IsolationUnavailable,
        ChatErrorKind::NotLoggedIn => ErrorCode::NotLoggedIn,
        ChatErrorKind::UsageLimit => ErrorCode::UsageLimit,
        ChatErrorKind::Offline => ErrorCode::Offline,
        ChatErrorKind::Timeout => ErrorCode::Timeout,
        ChatErrorKind::BackendCrashed { .. } => ErrorCode::BackendCrashed,
        ChatErrorKind::StreamMalformed => ErrorCode::StreamMalformed,
        ChatErrorKind::BackendError => ErrorCode::BackendError,
    };
    CoreError::new(code, e.message.clone())
        .with_detail(serde_json::to_value(e).unwrap_or(Value::Null))
}

fn turn_error(e: &ChatError) -> TurnError {
    TurnError {
        code: e.code().to_owned(),
        message: e.message.clone(),
        fix: Some(e.fix.clone()),
    }
}

// ---------------------------------------------------------------- runtime

struct Running {
    turn_id: Ulid,
    project_id: Ulid,
    cancel: CancelHandle,
}

#[derive(Default)]
struct Rt {
    status: Option<(Timestamp, BackendStatus)>,
    /// `(project, host)` → (publish order, input) (FR-PRM-23).
    contexts: HashMap<(Ulid, ChatHost), (u64, Value)>,
    seq: u64,
    running: Option<Running>,
}

struct SessionEntry {
    project_id: Ulid,
    first_host: ChatHost,
    session: Box<dyn ChatSession>,
    /// FR-PRM-25: `(image id, content hash)` pairs this session has received.
    received: HashSet<(String, String)>,
}

/// The chat's memory: one per core (see the module docs for the locks).
#[derive(Default)]
pub struct ChatRuntime {
    rt: Arc<Mutex<Rt>>,
    session: Arc<Mutex<Option<SessionEntry>>>,
    thread_io: Arc<Mutex<()>>,
}

impl std::fmt::Debug for ChatRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let rt = lock(&self.rt);
        f.debug_struct("ChatRuntime")
            .field("contexts", &rt.contexts.len())
            .field("running", &rt.running.as_ref().map(|r| r.turn_id))
            .finish_non_exhaustive()
    }
}

impl ChatRuntime {
    /// Ends the backend session (FR-PRM-35): the running turn is canceled first, then the
    /// child closes. The next turn starts a fresh session.
    pub fn end_session(&self) {
        let cancel = lock(&self.rt).running.as_ref().map(|r| r.cancel.clone());
        if let Some(c) = cancel {
            c.cancel();
        }
        let entry = lock(&self.session).take();
        if let Some(mut e) = entry {
            e.session.close();
        }
    }

    /// The asset project closed or another one opened: its contexts go, and its session ends
    /// (FR-PRM-35). A running turn finishes into its own thread file (§11).
    pub fn on_project_close(&self, project_id: Ulid) {
        let running = {
            let mut rt = lock(&self.rt);
            rt.contexts.retain(|(p, _), _| *p != project_id);
            rt.running.as_ref().map(|r| r.project_id)
        };
        match running {
            Some(p) if p == project_id => self.end_session(),
            Some(_) => {}
            None => {
                let mut slot = lock(&self.session);
                if slot.as_ref().is_some_and(|e| e.project_id == project_id)
                    && let Some(mut e) = slot.take()
                {
                    e.session.close();
                }
            }
        }
    }

    fn clear_status(&self) {
        lock(&self.rt).status = None;
    }

    /// True while a session is open (tests, the window's "New session" note).
    pub fn has_session(&self) -> bool {
        self.session.try_lock().map(|s| s.is_some()).unwrap_or(true)
    }

    /// The running turn, if any.
    pub fn running_turn(&self) -> Option<Ulid> {
        lock(&self.rt).running.as_ref().map(|r| r.turn_id)
    }
}

/// At startup (§11): turn image folders and Claude Code's `projects/` are emptied.
pub fn sweep_startup(ctx: &Context) {
    let chat = ctx.data_dir.join("chat");
    for e in std::fs::read_dir(&chat).into_iter().flatten().flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    let _ = pf_chat::invocation::sweep_projects(&ctx.data_dir);
}

fn consented(c: &PromptChatSettings) -> bool {
    c.consent
        .as_ref()
        .is_some_and(|c| c.notice_version >= NOTICE_VERSION)
}

/// The backend for this use: the shared fake, or a Claude Code adapter with the current
/// `claude_path` (FR-PRM-32).
fn backend(core: &Core, chat: &PromptChatSettings) -> Arc<dyn ChatBackend> {
    let seams = &core.ctx.chat;
    match seams.mode {
        BackendMode::Fake => seams.fake.clone(),
        BackendMode::ClaudeCode => Arc::new(
            ClaudeCodeBackend::new(
                ClaudeCodeConfig {
                    data_dir: core.ctx.data_dir.clone(),
                    claude_path: chat.claude_path.as_ref().map(PathBuf::from),
                    parent_env: seams.parent_env.clone(),
                    windows: cfg!(windows),
                },
                seams.spawner.clone(),
                seams.clock.clone(),
            )
            .with_budget(seams.budget),
        ),
    }
}

fn backend_name(mode: BackendMode) -> &'static str {
    match mode {
        BackendMode::Fake => "fake",
        BackendMode::ClaudeCode => "claude-code",
    }
}

/// FR-PRM-33: the cached detection, or a fresh one (only called after consent).
fn detection(core: &Core, chat: &PromptChatSettings, refresh: bool) -> BackendStatus {
    let now = core.ctx.now();
    if !refresh
        && let Some((at, st)) = &lock(&core.chat.rt).status
        && now.millis() - at.millis() < STATUS_TTL_MS
    {
        return st.clone();
    }
    let st = match backend(core, chat).detect() {
        Ok(ok) => BackendStatus::Ok {
            notional: ok.notional(),
            version: ok.version,
            auth_method: ok.auth_method,
            plan: ok.plan,
        },
        Err(error) => BackendStatus::Error { error },
    };
    let old = lock(&core.chat.rt).status.replace((now, st.clone()));
    if old.map(|(_, o)| o) != Some(st.clone()) {
        core.ctx.emit(EVENT_STATUS, json!({ "backend": &st }));
    }
    st
}

/// `prm_chat_status`'s output.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatStatus {
    pub enabled: bool,
    /// The current notice is accepted (FR-PRM-20).
    pub consent: bool,
    pub notice_version: u32,
    pub mode: BackendMode,
    pub backend: BackendStatus,
    /// The fake backend's case (fake mode only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fake_case: Option<String>,
    /// The login command shown for copying in Settings (FR-PRM-45); after consent only, and
    /// `null` in fake mode or when Claude Code isn't found.
    pub login_command: Option<String>,
    /// The Claude Code path override (Settings).
    pub claude_path: Option<String>,
}

/// FR-PRM-20/33: no process before consent or while off, not even detection.
pub fn status(core: &Core, refresh: bool) -> ChatStatus {
    let chat = core.state().settings.prompt_chat.clone();
    let mode = core.ctx.chat.mode;
    let backend = if consented(&chat) && chat.enabled {
        detection(core, &chat, refresh)
    } else {
        BackendStatus::NotChecked
    };
    ChatStatus {
        enabled: chat.enabled,
        consent: consented(&chat),
        notice_version: NOTICE_VERSION,
        mode,
        backend,
        fake_case: (mode == BackendMode::Fake).then(|| core.ctx.chat.fake.case().to_string()),
        login_command: if consented(&chat) {
            login_copy_text(core).ok().flatten()
        } else {
            None
        },
        claude_path: chat.claude_path.clone(),
    }
}

/// What a send needs from detection.
struct Ready {
    notional: bool,
}

fn require_ready(core: &Core) -> CoreResult<(PromptChatSettings, Ready)> {
    let chat = core.state().settings.prompt_chat.clone();
    if !consented(&chat) {
        return Err(CoreError::new(
            ErrorCode::ConsentRequired,
            "accept the prompt chat's privacy notice first",
        ));
    }
    if !chat.enabled {
        return Err(CoreError::new(
            ErrorCode::ChatDisabled,
            "the prompt chat is off in Settings",
        ));
    }
    match detection(core, &chat, false) {
        BackendStatus::Ok { notional, .. } => Ok((chat, Ready { notional })),
        BackendStatus::Error { error } => Err(core_error(&error)),
        BackendStatus::NotChecked => Err(CoreError::new(
            ErrorCode::ChatDisabled,
            "the prompt chat is off in Settings",
        )),
    }
}

/// The Tauri layer's `prm_chat_login` (FR-PRM-40/45): opens the login console.
pub fn open_login_console(core: &Core) -> CoreResult<()> {
    let chat = core.state().settings.prompt_chat.clone();
    if !consented(&chat) {
        return Err(CoreError::new(
            ErrorCode::ConsentRequired,
            "accept the prompt chat's privacy notice first",
        ));
    }
    core.chat.clear_status();
    backend(core, &chat)
        .open_login_console()
        .map_err(|e| core_error(&e))
}

/// The login command shown for copying (FR-PRM-45); `None` in fake mode.
pub fn login_copy_text(core: &Core) -> CoreResult<Option<String>> {
    let chat = core.state().settings.prompt_chat.clone();
    backend(core, &chat)
        .login_command()
        .map(|c| c.map(|c| c.copy_text))
        .map_err(|e| core_error(&e))
}

fn source(host: ChatHost) -> &'static dyn ChatContextSource {
    match host {
        ChatHost::Rgn => &crate::region_edit::chat::RgnChatContext,
        ChatHost::Gen => &crate::generate::chat::GenContextSource,
    }
}

/// The open project's folder and read-only flag, or `no_project`.
fn open_project(core: &Core, project_id: Ulid) -> CoreResult<(PathBuf, bool)> {
    let state = core.state();
    let p = state
        .project
        .as_ref()
        .filter(|p| p.manifest.project_id == project_id)
        .ok_or_else(|| CoreError::new(ErrorCode::NoProject, "that asset project is not open"))?;
    Ok((p.dir.clone(), p.read_only.is_some()))
}

fn no_prompt_block(host: Option<ChatHost>) -> CoreError {
    let message = match host {
        None => "open the Generate panel to chat about this project".to_owned(),
        Some(h) => format!("switch to the {} tab to insert", message::host_label(h)),
    };
    CoreError::new(ErrorCode::NoPromptBlock, message)
}

// ---------------------------------------------------------------- commands

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusInput {
    /// Re-detect now (window focus, Recheck), ignoring the 60 s cache.
    #[serde(default)]
    pub refresh: bool,
}

impl Validate for StatusInput {}

/// `prm_chat_status` (FR-PRM-20/33).
pub struct PrmChatStatus;

impl Command for PrmChatStatus {
    const NAME: &'static str = "prm_chat_status";
    type Input = StatusInput;
    type Output = ChatStatus;

    fn run(core: &Core, input: StatusInput) -> CoreResult<ChatStatus> {
        Ok(status(core, input.refresh))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentInput {
    pub accept: bool,
}

impl Validate for ConsentInput {}

/// `prm_chat_consent` (FR-PRM-20): accepting stores the consent and turns the chat on; "Not
/// now" changes nothing and starts nothing.
pub struct PrmChatConsent;

impl Command for PrmChatConsent {
    const NAME: &'static str = "prm_chat_consent";
    type Input = ConsentInput;
    type Output = ChatStatus;

    fn run(core: &Core, input: ConsentInput) -> CoreResult<ChatStatus> {
        if input.accept {
            let mut state = core.state();
            let mut next = state.settings.clone();
            next.prompt_chat.consent = Some(ChatConsent {
                accepted_at: core.ctx.now(),
                notice_version: NOTICE_VERSION,
            });
            next.prompt_chat.enabled = true;
            settings::save(&core.ctx, &next)?;
            state.settings = next;
        }
        Ok(status(core, false))
    }
}

/// `Some(None)` clears a nullable field; absent leaves it.
fn nullable<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatSettingsInput {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "nullable")]
    pub claude_path: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub window: Option<Option<ChatWindowRect>>,
}

impl Validate for ChatSettingsInput {
    fn validate(&self) -> CoreResult<()> {
        if let Some(Some(p)) = &self.claude_path
            && p.chars().count() > 1024
        {
            return Err(CoreError::invalid("claude_path", "the path is too long"));
        }
        if let Some(Some(w)) = &self.window
            && (w.w < 360 || w.h < 480)
        {
            return Err(CoreError::invalid(
                "window",
                "the chat window is at least 360×480 (FR-PRM-21)",
            ));
        }
        Ok(())
    }
}

/// `prm_chat_settings_put`: Settings → Prompt chat (on/off, the Claude Code path) and the
/// window's remembered rect. Consent is never set here. Turning the chat off ends any running
/// process and keeps the thread (FR-PRM-20).
pub struct PrmChatSettingsPut;

impl Command for PrmChatSettingsPut {
    const NAME: &'static str = "prm_chat_settings_put";
    type Input = ChatSettingsInput;
    type Output = ChatStatus;

    fn run(core: &Core, input: ChatSettingsInput) -> CoreResult<ChatStatus> {
        let (turned_off, path_changed) = {
            let mut state = core.state();
            let mut next = state.settings.clone();
            let c = &mut next.prompt_chat;
            let was_on = c.enabled;
            let old_path = c.claude_path.clone();
            if let Some(e) = input.enabled {
                c.enabled = e;
            }
            if let Some(p) = input.claude_path {
                c.claude_path = p.map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
            }
            if let Some(w) = input.window {
                c.window = w;
            }
            let out = (was_on && !c.enabled, old_path != c.claude_path);
            settings::save(&core.ctx, &next)?;
            state.settings = next;
            out
        };
        if turned_off || path_changed {
            core.chat.clear_status();
            core.chat.end_session();
        }
        Ok(status(core, false))
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

impl Validate for Empty {}

/// `prm_chat_logout` (FR-PRM-45): `claude auth logout` for the chat's own folder.
pub struct PrmChatLogout;

impl Command for PrmChatLogout {
    const NAME: &'static str = "prm_chat_logout";
    type Input = Empty;
    type Output = ChatStatus;

    fn run(core: &Core, _: Empty) -> CoreResult<ChatStatus> {
        let chat = core.state().settings.prompt_chat.clone();
        if !consented(&chat) {
            return Err(CoreError::new(
                ErrorCode::ConsentRequired,
                "accept the prompt chat's privacy notice first",
            ));
        }
        core.chat.end_session();
        backend(core, &chat).logout().map_err(|e| core_error(&e))?;
        core.chat.clear_status();
        Ok(status(core, true))
    }
}

/// `prm_chat_session_end`: the chat window closed (FR-PRM-35). The next turn starts a fresh
/// session.
pub struct PrmChatSessionEnd;

impl Command for PrmChatSessionEnd {
    const NAME: &'static str = "prm_chat_session_end";
    type Input = Empty;
    type Output = Empty;

    fn run(core: &Core, _: Empty) -> CoreResult<Empty> {
        core.chat.end_session();
        Ok(Empty {})
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishInput {
    pub project_id: Ulid,
    pub host: ChatHost,
    pub input: Value,
}

impl Validate for PublishInput {
    fn validate(&self) -> CoreResult<()> {
        if !self.input.is_object() {
            return Err(CoreError::invalid(
                "input",
                "the published input is an object",
            ));
        }
        Ok(())
    }
}

/// `prm_context_publish` (FR-PRM-23): the visible tab's draft input. Nothing is built or read
/// until a turn is sent.
pub struct PrmContextPublish;

impl Command for PrmContextPublish {
    const NAME: &'static str = "prm_context_publish";
    type Input = PublishInput;
    type Output = Empty;

    fn run(core: &Core, input: PublishInput) -> CoreResult<Empty> {
        open_project(core, input.project_id)?;
        let mut rt = lock(&core.chat.rt);
        rt.seq += 1;
        let seq = rt.seq;
        rt.contexts
            .insert((input.project_id, input.host), (seq, input.input));
        Ok(Empty {})
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearInput {
    pub project_id: Ulid,
    pub host: ChatHost,
}

impl Validate for ClearInput {}

/// `prm_context_clear` (FR-PRM-23): the tab was hidden or unmounted.
pub struct PrmContextClear;

impl Command for PrmContextClear {
    const NAME: &'static str = "prm_context_clear";
    type Input = ClearInput;
    type Output = Empty;

    fn run(core: &Core, input: ClearInput) -> CoreResult<Empty> {
        lock(&core.chat.rt)
            .contexts
            .remove(&(input.project_id, input.host));
        Ok(Empty {})
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectInput {
    pub project_id: Ulid,
}

impl Validate for ProjectInput {}

/// `prm_chat_thread_get`'s output.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadView {
    /// `null` before the first send.
    pub thread: Option<Thread>,
    pub running_turn_id: Option<Ulid>,
    /// A corrupt thread file was set aside and a new thread starts (§11).
    pub corrupt_set_aside: bool,
    /// The next turn starts a fresh session (FR-PRM-35): "New session: earlier messages
    /// aren't sent".
    pub fresh_session_next: bool,
    /// The host whose prompt block is attached for the project (the latest published), or
    /// `null`: the window's header and "Send disabled: no prompt block" (FR-PRM-27/43).
    pub current_host: Option<ChatHost>,
}

/// `prm_chat_thread_get` (FR-PRM-22).
pub struct PrmChatThreadGet;

impl Command for PrmChatThreadGet {
    const NAME: &'static str = "prm_chat_thread_get";
    type Input = ProjectInput;
    type Output = ThreadView;

    fn run(core: &Core, input: ProjectInput) -> CoreResult<ThreadView> {
        let (dir, read_only) = open_project(core, input.project_id)?;
        let loaded = {
            let _io = lock(&core.chat.thread_io);
            thread::load(&core.ctx, &dir, !read_only)
        };
        let running_turn_id = core.chat.running_turn();
        let fresh_session_next = running_turn_id.is_none()
            && !core
                .chat
                .session
                .try_lock()
                .map(|s| s.as_ref().is_some_and(|e| e.project_id == input.project_id))
                .unwrap_or(true);
        let current_host = lock(&core.chat.rt)
            .contexts
            .iter()
            .filter(|((p, _), _)| *p == input.project_id)
            .max_by_key(|(_, (seq, _))| *seq)
            .map(|((_, h), _)| *h);
        Ok(ThreadView {
            thread: loaded.thread,
            running_turn_id,
            corrupt_set_aside: loaded.corrupt_set_aside,
            fresh_session_next,
            current_host,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendInput {
    pub project_id: Ulid,
    pub text: String,
    #[serde(default = "include_on")]
    pub include_images: bool,
}

fn include_on() -> bool {
    true
}

impl Validate for SendInput {
    fn validate(&self) -> CoreResult<()> {
        let n = self.text.trim().chars().count();
        if n == 0 || self.text.chars().count() > MAX_TEXT_CHARS {
            return Err(CoreError::invalid(
                "text",
                format!("a message is 1–{MAX_TEXT_CHARS} characters"),
            ));
        }
        Ok(())
    }
}

/// `prm_chat_send`'s output.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SendOutput {
    /// The reply's turn id (events and `prm_chat_cancel` use it).
    pub turn_id: Ulid,
    pub user_turn_id: Ulid,
    pub thread_id: Ulid,
    pub host: ChatHost,
    pub context_summary: String,
    /// Labels of the images the turn limit dropped (FR-PRM-25).
    pub dropped: Vec<String>,
    pub fresh_session: bool,
    /// False when the thread couldn't be written (read-only project, a locked file): the
    /// banner says "not saved" (§11).
    pub saved: bool,
}

/// `prm_chat_send` (FR-PRM-27): records the user message and the host, builds the context,
/// starts the reply on its own thread and returns at once. The reply streams as
/// `prm:chat-delta` and ends with `prm:chat-turn`.
pub struct PrmChatSend;

impl Command for PrmChatSend {
    const NAME: &'static str = "prm_chat_send";
    type Input = SendInput;
    type Output = SendOutput;

    fn run(core: &Core, input: SendInput) -> CoreResult<SendOutput> {
        let pid = input.project_id;
        let (dir, read_only) = open_project(core, pid)?;
        let (chat, ready) = require_ready(core)?;
        let (host, published) = {
            let rt = lock(&core.chat.rt);
            if rt.running.is_some() {
                return Err(CoreError::new(
                    ErrorCode::TurnActive,
                    "wait for the current reply, or stop it",
                ));
            }
            rt.contexts
                .iter()
                .filter(|((p, _), _)| *p == pid)
                .max_by_key(|(_, (seq, _))| *seq)
                .map(|((_, h), (_, v))| (*h, v.clone()))
                .ok_or_else(|| no_prompt_block(None))?
        };
        let ctx = source(host).build(core, pid, &published)?;
        let backend = backend(core, &chat);
        let now = core.ctx.now();

        // The thread (created at the first send).
        let mut thread = {
            let _io = lock(&core.chat.thread_io);
            thread::load(&core.ctx, &dir, !read_only).thread
        }
        .unwrap_or_else(|| Thread::new(core.ctx.new_id(), now, backend_name(core.ctx.chat.mode)));
        let thread_id = thread.id;
        let cwd = core.ctx.data_dir.join("chat").join(thread_id.to_string());

        // The session (FR-PRM-35): reused for the same project while its child lives.
        let mut slot = lock(&core.chat.session);
        if slot
            .as_ref()
            .is_some_and(|e| e.project_id != pid || !e.session.is_alive())
            && let Some(mut old) = slot.take()
        {
            old.session.close();
        }
        let fresh = slot.is_none();
        if fresh {
            let session = backend
                .open_session(SessionOptions {
                    first_host: message::pf_host(host),
                    cwd: cwd.clone(),
                })
                .map_err(|e| core_error(&e))?;
            *slot = Some(SessionEntry {
                project_id: pid,
                first_host: host,
                session,
                received: HashSet::new(),
            });
        }
        let entry = slot
            .as_mut()
            .ok_or_else(|| CoreError::internal("no chat session"))?;

        // FR-PRM-25: unsent images only, then the turn limit (the tail is dropped).
        let (mut sent, already): (Vec<ChatImage>, Vec<ChatImage>) = if input.include_images {
            ctx.images.iter().cloned().partition(|i| {
                !entry
                    .received
                    .contains(&(i.id.clone(), i.content_hash.clone()))
            })
        } else {
            (Vec::new(), Vec::new())
        };
        let dropped = limit_images(&mut sent);
        for i in &sent {
            entry
                .received
                .insert((i.id.clone(), i.content_hash.clone()));
        }
        let note = (host != entry.first_host)
            .then(|| pf_chat::system_prompt::host_note(message::pf_host(host)));
        drop(slot);

        let block = message::context_block(&ctx, &sent, &dropped, &already);
        let summary = message::summary(&ctx, &sent, input.include_images);
        let dropped_labels: Vec<String> = dropped.iter().map(|i| i.label.clone()).collect();
        let user_turn_id = core.ctx.new_id();
        let turn_id = core.ctx.new_id();
        thread.turns.push(Turn {
            id: user_turn_id,
            at: now,
            role: Role::User,
            host,
            text: input.text.trim().to_owned(),
            state: TurnState::Pending,
            error: None,
            context: Some(TurnContext {
                summary: summary.clone(),
                images: sent
                    .iter()
                    .map(|i| SentImageRecord {
                        id: i.id.clone(),
                        role: i.role,
                        w: i.w,
                        h: i.h,
                        hash: i.content_hash.clone(),
                    })
                    .collect(),
                dropped: dropped_labels.clone(),
                final_prompt: ctx.final_prompt.clone(),
                model_id: ctx.model.id.clone(),
                controls: ctx
                    .controls
                    .iter()
                    .map(|c| (c.name.clone(), c.value.clone()))
                    .collect::<BTreeMap<_, _>>(),
            }),
            proposals: Vec::new(),
            usage: None,
            fresh_session: fresh,
            extra: Default::default(),
        });
        thread.turns.push(Turn {
            id: turn_id,
            at: now,
            role: Role::Assistant,
            host,
            text: String::new(),
            state: TurnState::Pending,
            error: None,
            context: None,
            proposals: Vec::new(),
            usage: None,
            fresh_session: false,
            extra: Default::default(),
        });
        thread.updated_at = now;
        let saved = !read_only && {
            let _io = lock(&core.chat.thread_io);
            thread::save(&core.ctx, &dir, &thread).is_ok()
        };
        write_turn_images(&cwd, &sent);

        let turn_input = TurnInput {
            host: message::pf_host(host),
            message_text: message::compose(note, &block, &input.text),
            user_text: input.text.trim().to_owned(),
            model_id: (!ctx.model.id.is_empty()).then(|| ctx.model.id.clone()),
            final_prompt: Some(ctx.final_prompt.clone()),
            images: sent
                .iter()
                .map(|i| TurnImage {
                    id: i.id.clone(),
                    content_hash: i.content_hash.clone(),
                    width: i.w,
                    height: i.h,
                    media_type: "image/jpeg".into(),
                    bytes: i.jpeg.clone(),
                })
                .collect(),
        };
        let cancel = CancelHandle::new();
        lock(&core.chat.rt).running = Some(Running {
            turn_id,
            project_id: pid,
            cancel: cancel.clone(),
        });
        let job = TurnJob {
            ctx: core.ctx.clone(),
            rt: core.chat.rt.clone(),
            session: core.chat.session.clone(),
            thread_io: core.chat.thread_io.clone(),
            project_dir: dir,
            writable: !read_only,
            thread,
            user_turn_id,
            turn_id,
            input: turn_input,
            cancel,
            notional: ready.notional,
        };
        if core.ctx.inline_background {
            job.run();
        } else {
            let spawned = std::thread::Builder::new()
                .name("pf-chat-turn".into())
                .spawn(move || job.run());
            if let Err(e) = spawned {
                lock(&core.chat.rt).running = None;
                return Err(CoreError::internal(format!(
                    "can't start the chat turn: {e}"
                )));
            }
        }
        Ok(SendOutput {
            turn_id,
            user_turn_id,
            thread_id,
            host,
            context_summary: summary,
            dropped: dropped_labels,
            fresh_session: fresh,
            saved,
        })
    }
}

/// FR-PRM-34: the child's folder holds only this turn's images (the previous ones go first).
fn write_turn_images(cwd: &Path, sent: &[ChatImage]) {
    let _ = std::fs::create_dir_all(cwd);
    for e in std::fs::read_dir(cwd).into_iter().flatten().flatten() {
        let _ = std::fs::remove_file(e.path());
    }
    for (i, img) in sent.iter().enumerate() {
        let safe: String = img
            .id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .take(40)
            .collect();
        let _ = std::fs::write(cwd.join(format!("{:02}-{safe}.jpg", i + 1)), &img.jpeg);
    }
}

/// One running turn, off the command thread.
struct TurnJob {
    ctx: Arc<Context>,
    rt: Arc<Mutex<Rt>>,
    session: Arc<Mutex<Option<SessionEntry>>>,
    thread_io: Arc<Mutex<()>>,
    project_dir: PathBuf,
    writable: bool,
    /// The thread as written at the send (used if the file can't be read back).
    thread: Thread,
    user_turn_id: Ulid,
    turn_id: Ulid,
    input: TurnInput,
    cancel: CancelHandle,
    notional: bool,
}

impl TurnJob {
    fn run(self) {
        let ctx = self.ctx.clone();
        let turn_id = self.turn_id;
        let outcome = {
            let mut slot = lock(&self.session);
            match slot.as_mut() {
                None => TurnOutcome::Failed {
                    error: ChatError::new(ChatErrorKind::BackendCrashed { exit_code: None })
                        .with_detail("the session ended before the turn started"),
                    partial: String::new(),
                },
                Some(entry) => {
                    let out = entry.session.run_turn(&self.input, &self.cancel, &mut |e| {
                        let TurnEvent::Delta(text) = e;
                        ctx.emit(EVENT_DELTA, json!({ "turn_id": turn_id, "text": text }));
                    });
                    if !entry.session.is_alive()
                        && let Some(mut e) = slot.take()
                    {
                        e.session.close();
                    }
                    out
                }
            }
        };
        let now = self.ctx.now();
        let (state, text, error, usage, proposals) = match &outcome {
            TurnOutcome::Completed { text, usage } => (
                TurnState::Done,
                text.clone(),
                None,
                usage.map(|u| TurnUsage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cost_usd: u.cost_usd,
                    notional: self.notional,
                }),
                message::proposals(text)
                    .into_iter()
                    .map(|t| Proposal {
                        id: self.ctx.new_id(),
                        text: t,
                        inserted_at: None,
                    })
                    .collect::<Vec<_>>(),
            ),
            TurnOutcome::Failed { error, partial } => (
                TurnState::Failed,
                partial.clone(),
                Some(error.clone()),
                None,
                Vec::new(),
            ),
            TurnOutcome::Canceled { partial } => {
                (TurnState::Canceled, partial.clone(), None, None, Vec::new())
            }
        };
        let saved = {
            let _io = lock(&self.thread_io);
            let mut thread = thread::load(&self.ctx, &self.project_dir, self.writable)
                .thread
                .filter(|t| t.id == self.thread.id)
                .unwrap_or_else(|| self.thread.clone());
            if let Some(u) = thread.turn_mut(self.user_turn_id) {
                u.state = TurnState::Done;
            }
            if let Some(t) = thread.turn_mut(self.turn_id) {
                t.state = state;
                t.text = text;
                t.error = error.as_ref().map(turn_error);
                t.usage = usage;
                t.proposals = proposals.clone();
                t.at = now;
            }
            thread.updated_at = now;
            self.writable && thread::save(&self.ctx, &self.project_dir, &thread).is_ok()
        };
        {
            let mut rt = lock(&self.rt);
            if rt.running.as_ref().is_some_and(|r| r.turn_id == turn_id) {
                rt.running = None;
            }
        }
        let mut payload = json!({
            "turn_id": turn_id,
            "state": state,
            "saved": saved,
        });
        if let Some(u) = usage {
            payload["usage"] = json!(u);
        }
        if let Some(e) = &error {
            payload["error"] = json!(e);
        }
        if !proposals.is_empty() {
            payload["proposals"] = json!(proposals);
        }
        self.ctx.emit(EVENT_TURN, payload);
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnIdInput {
    pub turn_id: Ulid,
}

impl Validate for TurnIdInput {}

/// `prm_chat_cancel` (FR-PRM-27): Stop. The partial reply is kept as `canceled`.
pub struct PrmChatCancel;

impl Command for PrmChatCancel {
    const NAME: &'static str = "prm_chat_cancel";
    type Input = TurnIdInput;
    type Output = Empty;

    fn run(core: &Core, input: TurnIdInput) -> CoreResult<Empty> {
        let cancel = lock(&core.chat.rt)
            .running
            .as_ref()
            .filter(|r| r.turn_id == input.turn_id)
            .map(|r| r.cancel.clone())
            .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "that turn isn't running"))?;
        cancel.cancel();
        Ok(Empty {})
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsertInput {
    pub project_id: Ulid,
    pub proposal_id: Ulid,
}

impl Validate for InsertInput {}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InsertOutput {
    pub proposal_id: Ulid,
    pub host: ChatHost,
    pub inserted_at: Timestamp,
}

/// `prm_chat_insert` (FR-PRM-29): Replace. Emits `prm:insert` for the proposal's host block,
/// which applies it as a normal edit; that block must be attached (FR-PRM-43).
pub struct PrmChatInsert;

impl Command for PrmChatInsert {
    const NAME: &'static str = "prm_chat_insert";
    type Input = InsertInput;
    type Output = InsertOutput;

    fn run(core: &Core, input: InsertInput) -> CoreResult<InsertOutput> {
        let (dir, read_only) = open_project(core, input.project_id)?;
        if read_only {
            return Err(CoreError::new(
                ErrorCode::ReadOnly,
                "this asset project is read-only",
            ));
        }
        let now = core.ctx.now();
        let _io = lock(&core.chat.thread_io);
        let mut thread = thread::load(&core.ctx, &dir, true)
            .thread
            .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "no such proposal"))?;
        let (host, text) = thread
            .turns
            .iter()
            .find_map(|t| {
                t.proposals
                    .iter()
                    .find(|p| p.id == input.proposal_id)
                    .map(|p| (t.host, p.text.clone()))
            })
            .ok_or_else(|| CoreError::new(ErrorCode::NotFound, "no such proposal"))?;
        if !lock(&core.chat.rt)
            .contexts
            .contains_key(&(input.project_id, host))
        {
            return Err(no_prompt_block(Some(host)));
        }
        for t in &mut thread.turns {
            for p in &mut t.proposals {
                if p.id == input.proposal_id {
                    p.inserted_at = Some(now);
                }
            }
        }
        thread.updated_at = now;
        thread::save(&core.ctx, &dir, &thread)?;
        core.ctx.emit(
            EVENT_INSERT,
            json!({
                "project_id": input.project_id,
                "host": host,
                "proposal_id": input.proposal_id,
                "text": text,
            }),
        );
        Ok(InsertOutput {
            proposal_id: input.proposal_id,
            host,
            inserted_at: now,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FakeCaseInput {
    pub case: String,
}

impl Validate for FakeCaseInput {}

/// `prm_chat_fake_set_case` (FR-PRM-39).
pub struct PrmChatFakeSetCase;

impl Command for PrmChatFakeSetCase {
    const NAME: &'static str = "prm_chat_fake_set_case";
    type Input = FakeCaseInput;
    type Output = Empty;

    fn run(core: &Core, input: FakeCaseInput) -> CoreResult<Empty> {
        if core.ctx.chat.mode != BackendMode::Fake {
            return Err(CoreError::new(
                ErrorCode::NotFakeMode,
                "the chat isn't using the fake backend",
            ));
        }
        let case = input
            .case
            .parse()
            .map_err(|e: String| CoreError::invalid("case", e))?;
        core.ctx.chat.fake.set_case(case);
        core.chat.clear_status();
        Ok(Empty {})
    }
}

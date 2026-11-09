//! The fake chat backend (FR-PRM-39): deterministic, starts no process and opens no
//! connection. Each turn plays stream-json lines through the same turn loop and parser as
//! the Claude Code adapter: the error cases replay the committed recordings
//! (`fixtures/chat/`), and the reply cases follow the recorded success shape with a reply
//! that quotes the user text and lists what the turn carried.

use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use crate::backend::{
    BackendMode, BackendOk, CancelHandle, ChatBackend, ChatSession, ConsoleCommand, SessionOptions,
    TurnEvent, TurnInput, TurnOutcome,
};
use crate::clock::{Clock, TurnBudget};
use crate::error::{ChatError, ChatErrorKind};
use crate::replay::{Script, ScriptedChild, Step, lines};
use crate::turn;
use crate::version::{MIN_VERSION_FLOOR, Version};

/// The owner's recording of a turn without a login (CLI 2.1.319, scrubbed).
pub const REC_NOT_LOGGED_IN: &str =
    include_str!("../../../fixtures/chat/claude-code/not-logged-in.jsonl");
/// Hand-made: the plan's usage limit (can't be forced on a real plan).
pub const REC_USAGE_LIMIT: &str = include_str!("../../../fixtures/chat/handmade/usage-limit.jsonl");
/// Hand-made: non-JSON lines on stdout.
pub const REC_MALFORMED: &str = include_str!("../../../fixtures/chat/handmade/malformed.jsonl");

/// The version the fake reports.
pub const FAKE_VERSION: Version = MIN_VERSION_FLOOR;
/// What `version_old` reports.
pub const FAKE_OLD_VERSION: Version = Version::new(2, 1, 200);
/// `slow` without a count.
pub const SLOW_DEFAULT: u32 = 5;
/// The gap between `slow(n)` deltas.
pub const SLOW_STEP: Duration = Duration::from_secs(1);

/// FR-PRM-39's cases (`PF_FAKE_CHAT_CASE`, `prm_chat_fake_set_case`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FakeCase {
    #[default]
    Success,
    /// The reply arrives in `n` deltas, [`SLOW_STEP`] apart (for Stop).
    Slow(u32),
    NoProposal,
    TwoProposals,
    NotInstalled,
    NotLoggedIn,
    VersionOld,
    IsolationUnavailable,
    UsageLimit,
    Offline,
    Timeout,
    Crash,
    Malformed,
}

impl FromStr for FakeCase {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if let Some(n) = s.strip_prefix("slow(").and_then(|r| r.strip_suffix(')')) {
            return n
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|n| (1..=600).contains(n))
                .map(Self::Slow)
                .ok_or_else(|| format!("slow(n) needs 1 ≤ n ≤ 600, got {n}"));
        }
        Ok(match s {
            "success" => Self::Success,
            "slow" => Self::Slow(SLOW_DEFAULT),
            "no_proposal" => Self::NoProposal,
            "two_proposals" => Self::TwoProposals,
            "not_installed" => Self::NotInstalled,
            "not_logged_in" => Self::NotLoggedIn,
            "version_old" => Self::VersionOld,
            "isolation_unavailable" => Self::IsolationUnavailable,
            "usage_limit" => Self::UsageLimit,
            "offline" => Self::Offline,
            "timeout" => Self::Timeout,
            "crash" => Self::Crash,
            "malformed" => Self::Malformed,
            other => return Err(format!("unknown fake chat case `{other}`")),
        })
    }
}

impl fmt::Display for FakeCase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Success => "success",
            Self::Slow(n) => return write!(f, "slow({n})"),
            Self::NoProposal => "no_proposal",
            Self::TwoProposals => "two_proposals",
            Self::NotInstalled => "not_installed",
            Self::NotLoggedIn => "not_logged_in",
            Self::VersionOld => "version_old",
            Self::IsolationUnavailable => "isolation_unavailable",
            Self::UsageLimit => "usage_limit",
            Self::Offline => "offline",
            Self::Timeout => "timeout",
            Self::Crash => "crash",
            Self::Malformed => "malformed",
        };
        f.write_str(s)
    }
}

/// The fake backend. The case can change at any time; a running session reads it per turn.
pub struct FakeBackend {
    case: Arc<Mutex<FakeCase>>,
    clock: Arc<dyn Clock>,
    budget: TurnBudget,
}

impl FakeBackend {
    pub fn new(case: FakeCase, clock: Arc<dyn Clock>) -> Self {
        Self {
            case: Arc::new(Mutex::new(case)),
            clock,
            budget: TurnBudget::default(),
        }
    }

    pub fn with_budget(mut self, budget: TurnBudget) -> Self {
        self.budget = budget;
        self
    }

    pub fn case(&self) -> FakeCase {
        self.case.lock().map(|c| *c).unwrap_or_default()
    }

    pub fn set_case(&self, case: FakeCase) {
        if let Ok(mut c) = self.case.lock() {
            *c = case;
        }
    }

    /// The detection-level errors (also refuse a session).
    fn startup_error(case: FakeCase) -> Option<ChatError> {
        match case {
            FakeCase::NotInstalled => Some(ChatError::new(ChatErrorKind::NotInstalled)),
            FakeCase::VersionOld => Some(ChatError::version_unsupported(FAKE_OLD_VERSION)),
            FakeCase::IsolationUnavailable => {
                Some(ChatError::new(ChatErrorKind::IsolationUnavailable))
            }
            _ => None,
        }
    }
}

impl ChatBackend for FakeBackend {
    fn mode(&self) -> BackendMode {
        BackendMode::Fake
    }

    fn detect(&self) -> Result<BackendOk, ChatError> {
        let case = self.case();
        if let Some(e) = Self::startup_error(case) {
            return Err(e);
        }
        if case == FakeCase::NotLoggedIn {
            return Err(ChatError::new(ChatErrorKind::NotLoggedIn));
        }
        Ok(BackendOk {
            version: FAKE_VERSION.to_string(),
            auth_method: Some("fake".into()),
            plan: None,
        })
    }

    fn login_command(&self) -> Result<Option<ConsoleCommand>, ChatError> {
        Ok(None)
    }

    /// No console: a `not_logged_in` fake logs in at once, so Log in… → Recheck can be walked
    /// on the fake (L2).
    fn open_login_console(&self) -> Result<(), ChatError> {
        if self.case() == FakeCase::NotLoggedIn {
            self.set_case(FakeCase::Success);
        }
        Ok(())
    }

    /// No process: the fake is `not_logged_in` afterwards (L2).
    fn logout(&self) -> Result<(), ChatError> {
        self.set_case(FakeCase::NotLoggedIn);
        Ok(())
    }

    fn open_session(&self, _options: SessionOptions) -> Result<Box<dyn ChatSession>, ChatError> {
        if let Some(e) = Self::startup_error(self.case()) {
            return Err(e);
        }
        Ok(Box::new(FakeSession {
            case: Arc::clone(&self.case),
            clock: Arc::clone(&self.clock),
            budget: self.budget,
            alive: true,
        }))
    }
}

/// A fake session: alive until a turn ends it as a real child would (FR-PRM-35).
pub struct FakeSession {
    case: Arc<Mutex<FakeCase>>,
    clock: Arc<dyn Clock>,
    budget: TurnBudget,
    alive: bool,
}

impl ChatSession for FakeSession {
    fn run_turn(
        &mut self,
        input: &TurnInput,
        cancel: &CancelHandle,
        on_event: &mut dyn FnMut(TurnEvent),
    ) -> TurnOutcome {
        if !self.alive {
            return TurnOutcome::Failed {
                error: ChatError::new(ChatErrorKind::BackendCrashed { exit_code: None })
                    .with_detail("the session has ended; start a new one"),
                partial: String::new(),
            };
        }
        let case = self.case.lock().map(|c| *c).unwrap_or_default();
        let script = script(case, input);
        let mut child = ScriptedChild::new(script, Arc::clone(&self.clock));
        let (outcome, alive) = turn::run_turn(
            &mut child,
            "{}",
            self.clock.as_ref(),
            self.budget,
            cancel,
            on_event,
            &|s: &str| s.to_owned(),
        );
        self.alive = alive;
        outcome
    }

    fn is_alive(&self) -> bool {
        self.alive
    }

    fn close(&mut self) {
        self.alive = false;
    }
}

/// FR-PRM-39's reply: quotes the user text, lists host, images (id, size, bytes, hash),
/// model and final prompt, and ends with `blocks` ` ```prompt ` blocks.
pub fn fake_reply(input: &TurnInput, blocks: usize) -> String {
    let mut s = String::from("Fake chat: no model was called.\n");
    s.push_str(&format!("You wrote: \"{}\"\n", input.user_text.trim()));
    s.push_str(&format!("Host: {}\n", input.host.label()));
    s.push_str(&format!("Images: {}\n", input.images.len()));
    for i in &input.images {
        s.push_str(&format!(
            "- {} {}×{}, {} bytes, {}\n",
            i.id,
            i.width,
            i.height,
            i.bytes.len(),
            i.content_hash
        ));
    }
    s.push_str(&format!(
        "Model: {}\n",
        input.model_id.as_deref().unwrap_or("none")
    ));
    s.push_str(&format!(
        "Final prompt: {}\n",
        input.final_prompt.as_deref().unwrap_or("none")
    ));
    let base = {
        let t = input
            .user_text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if t.is_empty() {
            "a detailed scene".to_owned()
        } else {
            t
        }
    };
    let proposals = [base.clone(), format!("{base}, seen from directly above")];
    for p in proposals.iter().take(blocks) {
        s.push_str(&format!("\n```prompt\n{p}\n```\n"));
    }
    s
}

fn init_line() -> String {
    json!({
        "type": "system", "subtype": "init", "cwd": "<data-dir>/chat/<thread>",
        "session_id": "<session-1>", "tools": [], "mcp_servers": [], "model": "fake",
        "slash_commands": [], "apiKeySource": "none",
        "claude_code_version": FAKE_VERSION.to_string(),
        "agents": [], "skills": [], "plugins": [],
    })
    .to_string()
}

fn assistant_line(text: &str) -> String {
    json!({
        "type": "assistant",
        "message": { "role": "assistant", "model": "fake",
                     "content": [{ "type": "text", "text": text }] },
        "parent_tool_use_id": null, "session_id": "<session-1>",
    })
    .to_string()
}

/// Deterministic usage: about four characters per token, Sonnet-like prices.
fn result_line(input: &TurnInput, reply: &str, is_error: bool) -> String {
    let image_bytes: usize = input.images.iter().map(|i| i.bytes.len()).sum();
    let tokens_in = (input.message_text.chars().count() / 4 + image_bytes / 750) as u64;
    let tokens_out = (reply.chars().count() / 4) as u64;
    let cost = tokens_in as f64 * 3e-6 + tokens_out as f64 * 15e-6;
    json!({
        "type": "result", "subtype": "success", "is_error": is_error,
        "result": reply, "session_id": "<session-1>",
        "total_cost_usd": if is_error { 0.0 } else { cost },
        "usage": { "input_tokens": if is_error { 0 } else { tokens_in },
                   "output_tokens": if is_error { 0 } else { tokens_out } },
    })
    .to_string()
}

/// The stdout of one turn for `case`.
pub fn script(case: FakeCase, input: &TurnInput) -> Script {
    let reply_turn = |blocks: usize| {
        let reply = fake_reply(input, blocks);
        vec![
            Step::Line(init_line()),
            Step::Line(assistant_line(&reply)),
            Step::Line(result_line(input, &reply, false)),
        ]
    };
    let steps = match case {
        FakeCase::Success
        | FakeCase::NotInstalled
        | FakeCase::VersionOld
        | FakeCase::IsolationUnavailable => reply_turn(1),
        FakeCase::TwoProposals => reply_turn(2),
        FakeCase::NoProposal => reply_turn(0),
        FakeCase::Slow(n) => {
            let reply = fake_reply(input, 1);
            let chars: Vec<char> = reply.chars().collect();
            let n = (n as usize).max(1);
            let size = chars.len().div_ceil(n).max(1);
            let mut steps = vec![Step::Line(init_line())];
            for chunk in chars.chunks(size) {
                steps.push(Step::Delay(SLOW_STEP));
                steps.push(Step::Line(json!({
                    "type": "stream_event",
                    "event": { "type": "content_block_delta", "index": 0,
                               "delta": { "type": "text_delta", "text": chunk.iter().collect::<String>() } },
                    "session_id": "<session-1>",
                }).to_string()));
            }
            steps.push(Step::Line(assistant_line(&reply)));
            steps.push(Step::Line(result_line(input, &reply, false)));
            steps
        }
        FakeCase::NotLoggedIn => lines(REC_NOT_LOGGED_IN),
        FakeCase::UsageLimit => lines(REC_USAGE_LIMIT),
        FakeCase::Malformed => lines(REC_MALFORMED),
        FakeCase::Offline => vec![
            Step::Line(init_line()),
            Step::Line(result_line(input, "API Error: Connection error.", true)),
        ],
        FakeCase::Timeout => vec![
            Step::Line(init_line()),
            Step::Line(assistant_line("Thinking about the room…")),
            Step::Stall,
        ],
        FakeCase::Crash => vec![
            Step::Line(init_line()),
            Step::Line(assistant_line("Here is a first idea")),
            Step::Exit(Some(1)),
        ],
    };
    let script = Script {
        turns: vec![steps],
        stderr: String::new(),
    };
    if case == FakeCase::Crash {
        script.with_stderr("fake chat: simulated crash")
    } else {
        script
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FR-PRM-39/45 (L2): fake Log in… and Log out start nothing and flip the login state.
    #[test]
    fn fr_prm_45_fake_login_and_logout_flip_the_login_state() {
        let f = FakeBackend::new(
            FakeCase::NotLoggedIn,
            Arc::new(crate::clock::SystemClock::new()),
        );
        assert_eq!(f.detect().unwrap_err().kind, ChatErrorKind::NotLoggedIn);
        f.open_login_console().unwrap();
        assert!(f.detect().is_ok());
        f.logout().unwrap();
        assert_eq!(f.case(), FakeCase::NotLoggedIn);
        // Another case stays as it is on Log in….
        f.set_case(FakeCase::UsageLimit);
        f.open_login_console().unwrap();
        assert_eq!(f.case(), FakeCase::UsageLimit);
    }

    #[test]
    fn fr_prm_39_cases_parse_and_print() {
        for s in [
            "success",
            "slow(3)",
            "no_proposal",
            "two_proposals",
            "not_installed",
            "not_logged_in",
            "version_old",
            "isolation_unavailable",
            "usage_limit",
            "offline",
            "timeout",
            "crash",
            "malformed",
        ] {
            let c: FakeCase = s.parse().unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(c.to_string(), s);
        }
        assert_eq!("slow".parse::<FakeCase>(), Ok(FakeCase::Slow(SLOW_DEFAULT)));
        assert!("slow(0)".parse::<FakeCase>().is_err());
        assert!("nope".parse::<FakeCase>().is_err());
    }
}

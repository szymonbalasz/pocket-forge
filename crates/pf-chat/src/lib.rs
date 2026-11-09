//! Prompt chat backend (PRM spec part B, #58, #60). No Tauri dependency.
//!
//! The backend (slice 4):
//! - [`backend`]: the [`backend::ChatBackend`] / [`backend::ChatSession`] traits pf-core
//!   drives, turn types, cancel, and the mode switch (§13).
//! - [`claude_code`]: the Claude Code adapter (§10, FR-PRM-33..38, FR-PRM-45, R87).
//! - [`fake`]: the fake backend (FR-PRM-39), the default in dev, tests and CI.
//! - [`process`]: the process seam and the real spawner (Job Object on Windows: `job`).
//! - [`replay`]: scripted children over recorded stdout (tests and the fake).
//! - [`turn`]: one turn over a child: budgets (FR-PRM-36), parsing, error mapping.
//! - [`clock`], [`error`]: the clock seam and budgets; Appendix D errors.
//!
//! The slice-0 pieces:
//! - [`invocation`]: the exact Claude Code argument list, the child environment
//!   (FR-PRM-34, FR-PRM-45) and executable resolution (§10).
//! - [`version`] and [`auth`]: `claude --version` and `claude auth status --json` parsing
//!   (FR-PRM-33), keeping only the allowed auth fields.
//! - [`stream`]: the stream-json stdout parser (§10, FR-PRM-37).
//! - [`message`]: the stream-json stdin user message with base64 image blocks (§10).
//! - [`scrub`]: the recordings scrubber for `fixtures/chat/claude-code/` (§10).
//! - [`claims`]: the probe's verified / failed claims table.
//! - [`system_prompt`]: Appendix B (`PROMPT_CHAT_SYSTEM_V1`).
//!
//! The probe binary (`pf-chat-probe`, feature `real-call`) runs the real CLI and is never
//! built or run in CI. Owner runbook: `docs/guides/prompt-chat-probe.md`.

pub mod auth;
pub mod backend;
pub mod base64;
pub mod claims;
pub mod claude_code;
pub mod clock;
pub mod error;
pub mod fake;
pub mod invocation;
#[cfg(windows)]
pub mod job;
pub mod message;
pub mod process;
pub mod replay;
pub mod scrub;
pub mod stream;
pub mod system_prompt;
pub mod turn;
pub mod version;

/// FR-PRV-28 by analogy (FR-PRM §13): under CI (`CI` set and not empty) the `real-call`
/// feature is refused, so the probe never runs there.
pub fn ci_guard(ci: Option<&str>, feature: bool) -> Result<(), String> {
    let in_ci = ci.is_some_and(|v| !v.trim().is_empty());
    if in_ci && feature {
        return Err("cargo feature `real-call` is set in CI (AGENTS §3.7, PRM §13)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prm_ci_guard_refuses_the_probe_in_ci() {
        assert!(ci_guard(Some("true"), true).is_err());
        assert!(ci_guard(Some("true"), false).is_ok());
        assert!(ci_guard(Some(" "), true).is_ok());
        assert!(ci_guard(None, true).is_ok());
    }
}

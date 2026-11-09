//! Prompt chat backend (PRM spec part B, #58, #60). No Tauri dependency.
//!
//! Slice 0 holds the pieces the probe needs and slice 4 builds on:
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
pub mod base64;
pub mod claims;
pub mod invocation;
pub mod message;
pub mod scrub;
pub mod stream;
pub mod system_prompt;
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

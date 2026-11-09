//! Backend errors (FR-PRM-37, Appendix D): a code, the readable message and its fix.

use serde::Serialize;

use crate::version::{MIN_VERSION_FLOOR, Version};

/// The FR-PRM-37 codes. Serialised as `{"code": "…", …fields}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ChatErrorKind {
    NotInstalled,
    VersionUnsupported { found: String, min: String },
    IsolationUnavailable,
    NotLoggedIn,
    UsageLimit,
    Offline,
    Timeout,
    BackendCrashed { exit_code: Option<i32> },
    StreamMalformed,
    BackendError,
}

impl ChatErrorKind {
    /// The snake_case code (Appendix D).
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::VersionUnsupported { .. } => "version_unsupported",
            Self::IsolationUnavailable => "isolation_unavailable",
            Self::NotLoggedIn => "not_logged_in",
            Self::UsageLimit => "usage_limit",
            Self::Offline => "offline",
            Self::Timeout => "timeout",
            Self::BackendCrashed { .. } => "backend_crashed",
            Self::StreamMalformed => "stream_malformed",
            Self::BackendError => "backend_error",
        }
    }
}

/// One backend error: Appendix D's message and fix, plus an optional detail (the CLI's
/// message, or the stderr tail with the home path replaced by `<home>`). Never written to a
/// project (FR-PRM-37).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatError {
    #[serde(flatten)]
    pub kind: ChatErrorKind,
    pub message: String,
    pub fix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ChatError {
    /// The error for `kind` with Appendix D's text. `usage_limit`'s fix is the CLI's message
    /// verbatim when given ([`ChatError::usage_limit`]).
    pub fn new(kind: ChatErrorKind) -> Self {
        let (message, fix) = match &kind {
            ChatErrorKind::NotInstalled => (
                "Claude Code isn't installed, or isn't on PATH.".to_owned(),
                "Install Claude Code, or set its path in Settings → Prompt chat.".to_owned(),
            ),
            ChatErrorKind::VersionUnsupported { found, min } => (
                format!("Claude Code {found} is too old (needs {min})."),
                "Run `claude update`.".to_owned(),
            ),
            ChatErrorKind::IsolationUnavailable => (
                "Prompt chat can't run Claude Code without your own Claude Code settings, so it stays off.".to_owned(),
                "Update Claude Code, then Recheck.".to_owned(),
            ),
            ChatErrorKind::NotLoggedIn => (
                "The prompt chat isn't logged in to Claude.".to_owned(),
                "Log in… in Settings → Prompt chat (runs `claude auth login` for the chat's own folder), then Recheck.".to_owned(),
            ),
            ChatErrorKind::UsageLimit => (
                "Your Claude plan's usage limit is reached.".to_owned(),
                "Try again when the limit resets.".to_owned(),
            ),
            ChatErrorKind::Offline => (
                "Can't reach Anthropic.".to_owned(),
                "Check the connection, then Recheck.".to_owned(),
            ),
            ChatErrorKind::Timeout => (
                "Claude Code didn't answer in time.".to_owned(),
                "Retry.".to_owned(),
            ),
            ChatErrorKind::BackendCrashed { exit_code } => (
                match exit_code {
                    Some(c) => format!("Claude Code stopped unexpectedly (exit {c})."),
                    None => "Claude Code stopped unexpectedly.".to_owned(),
                },
                "Retry; details show the stderr tail.".to_owned(),
            ),
            ChatErrorKind::StreamMalformed | ChatErrorKind::BackendError => (
                "Unexpected output from Claude Code.".to_owned(),
                "Retry; details show the message.".to_owned(),
            ),
        };
        Self {
            kind,
            message,
            fix,
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        let d = detail.into();
        self.detail = (!d.trim().is_empty()).then_some(d);
        self
    }

    /// `usage_limit` with the CLI's message as the fix, verbatim (e.g. the reset time).
    pub fn usage_limit(cli_message: Option<&str>) -> Self {
        let mut e = Self::new(ChatErrorKind::UsageLimit);
        if let Some(m) = cli_message.filter(|m| !m.trim().is_empty()) {
            e.fix = m.trim().to_owned();
        }
        e
    }

    pub fn version_unsupported(found: Version) -> Self {
        Self::new(ChatErrorKind::VersionUnsupported {
            found: found.to_string(),
            min: MIN_VERSION_FLOOR.to_string(),
        })
    }

    pub fn backend_error(message: impl Into<String>) -> Self {
        Self::new(ChatErrorKind::BackendError).with_detail(message)
    }

    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message)
    }
}

impl std::error::Error for ChatError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prm_37_codes_serialise_with_message_and_fix() {
        let e = ChatError::new(ChatErrorKind::BackendCrashed { exit_code: Some(3) });
        let v = serde_json::to_value(&e).unwrap_or_default();
        assert_eq!(v["code"], "backend_crashed");
        assert_eq!(v["exit_code"], 3);
        assert_eq!(v["message"], "Claude Code stopped unexpectedly (exit 3).");
        assert!(v.get("detail").is_none());

        let e = ChatError::version_unsupported(Version::new(2, 1, 300));
        assert_eq!(e.message, "Claude Code 2.1.300 is too old (needs 2.1.319).");
        let v = serde_json::to_value(&e).unwrap_or_default();
        assert_eq!(
            (v["found"].as_str(), v["min"].as_str()),
            (Some("2.1.300"), Some("2.1.319"))
        );
    }

    #[test]
    fn fr_prm_37_usage_limit_shows_the_cli_message_verbatim() {
        let e = ChatError::usage_limit(Some("You've hit your limit · resets 3pm"));
        assert_eq!(e.code(), "usage_limit");
        assert_eq!(e.fix, "You've hit your limit · resets 3pm");
        assert_eq!(
            ChatError::usage_limit(None).fix,
            "Try again when the limit resets."
        );
    }
}

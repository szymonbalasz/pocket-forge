//! `claude auth status --json` parsing (FR-PRM-33, FR-PRM-45).
//!
//! Only `loggedIn`, `authMethod`, `subscriptionType` and `configDirectory` are deserialised.
//! Every other field (email, organisation, paths) is skipped by serde without being stored,
//! so it can't be logged. Callers must never print the raw output.

use std::path::Path;

use serde::Deserialize;

/// The four allowed fields. `config_directory` is a local path: never log or record it.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    #[serde(default)]
    pub logged_in: Option<bool>,
    #[serde(default)]
    pub auth_method: Option<String>,
    #[serde(default)]
    pub subscription_type: Option<String>,
    #[serde(default)]
    pub config_directory: Option<String>,
}

impl std::fmt::Debug for AuthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthStatus")
            .field("logged_in", &self.logged_in)
            .field("auth_method", &self.auth_method)
            .field("subscription_type", &self.subscription_type)
            .field(
                "config_directory",
                &self.config_directory.as_ref().map(|_| "<path>"),
            )
            .finish()
    }
}

/// What `auth status` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthState {
    LoggedIn(AuthStatus),
    /// Exit 1, or `loggedIn: false` (FR-PRM-33).
    NotLoggedIn(AuthStatus),
    /// Neither: an unexpected exit code or output that isn't the JSON object.
    Unreadable {
        exit_code: Option<i32>,
    },
}

/// Classifies one `auth status --json` run. Exit 1 → not logged in (documented). Exit 0
/// with `loggedIn: true` → logged in.
pub fn parse_auth_status(exit_code: Option<i32>, stdout: &str) -> AuthState {
    let parsed = json_object(stdout).and_then(|s| serde_json::from_str::<AuthStatus>(s).ok());
    match (exit_code, parsed) {
        (Some(1), p) => AuthState::NotLoggedIn(p.unwrap_or_default()),
        (Some(0), Some(p)) if p.logged_in == Some(true) => AuthState::LoggedIn(p),
        (Some(0), Some(p)) if p.logged_in == Some(false) => AuthState::NotLoggedIn(p),
        (code, _) => AuthState::Unreadable { exit_code: code },
    }
}

/// The outermost `{ … }` of the output (the CLI may print a notice line around it).
fn json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    (end > start).then(|| &s[start..=end])
}

/// FR-PRM-45 fail-closed check: does the reported `configDirectory` name `expected`?
/// Slashes, a `\\?\` prefix and a trailing separator are ignored; on Windows the case too.
pub fn same_config_dir(reported: &str, expected: &Path) -> bool {
    let case_insensitive = cfg!(windows);
    normalise_dir(reported, case_insensitive)
        == normalise_dir(&expected.to_string_lossy(), case_insensitive)
}

/// The comparison form used by [`same_config_dir`].
pub fn normalise_dir(path: &str, case_insensitive: bool) -> String {
    let p = path.trim();
    let p = p
        .strip_prefix(r"\\?\")
        .or_else(|| p.strip_prefix("//?/"))
        .unwrap_or(p);
    let mut s = p.replace('\\', "/");
    while s.len() > 1 && s.ends_with('/') {
        s.pop();
    }
    if case_insensitive {
        s = s.to_lowercase();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGGED_IN: &str = r#"{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max","configDirectory":"D:/pf-data/claude-config","email":"someone@example.com","orgId":"org-123","orgName":"Someone's org"}"#;

    #[test]
    fn fr_prm_33_auth_status_keeps_only_the_allowed_fields() {
        let AuthState::LoggedIn(s) = parse_auth_status(Some(0), LOGGED_IN) else {
            panic!("expected logged in");
        };
        assert_eq!(s.auth_method.as_deref(), Some("claude.ai"));
        assert_eq!(s.subscription_type.as_deref(), Some("max"));
        // The Debug form never shows the path, and the struct has no field for the rest.
        let dbg = format!("{s:?}");
        assert!(!dbg.contains("pf-data") && !dbg.contains("example") && !dbg.contains("org"));
    }

    #[test]
    fn fr_prm_33_not_logged_in_on_exit_1_or_logged_in_false() {
        assert!(matches!(
            parse_auth_status(Some(1), "Not logged in"),
            AuthState::NotLoggedIn(_)
        ));
        assert!(matches!(
            parse_auth_status(Some(0), r#"{"loggedIn":false,"authMethod":"none"}"#),
            AuthState::NotLoggedIn(_)
        ));
        assert!(matches!(
            parse_auth_status(Some(0), "garbage"),
            AuthState::Unreadable { exit_code: Some(0) }
        ));
        assert!(matches!(
            parse_auth_status(None, LOGGED_IN),
            AuthState::Unreadable { exit_code: None }
        ));
    }

    #[test]
    fn fr_prm_33_auth_status_tolerates_a_notice_around_the_json() {
        let out = format!("Update available\n{LOGGED_IN}\n");
        assert!(matches!(
            parse_auth_status(Some(0), &out),
            AuthState::LoggedIn(_)
        ));
    }

    #[test]
    fn fr_prm_45_foreign_config_directory_does_not_match() {
        assert_eq!(
            normalise_dir(r"\\?\D:\Data\PF\claude-config\", true),
            "d:/data/pf/claude-config"
        );
        assert_eq!(
            normalise_dir("/srv/pf/claude-config/", false),
            "/srv/pf/claude-config"
        );
        assert_ne!(
            normalise_dir("D:/data/claude-config", true),
            normalise_dir("D:/data/other", true)
        );
        let expected = Path::new("/srv/pf/claude-config");
        assert!(same_config_dir("/srv/pf/claude-config/", expected));
        assert!(!same_config_dir("/srv/pf/.claude", expected));
    }
}

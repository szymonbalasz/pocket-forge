//! The probe's claims table (§10 "Claim | Status", §14 slice 0) and the isolation checks
//! over `system/init` that the probe runs.

use std::fmt::Write as _;

use crate::stream::InitInfo;
use crate::version::{MIN_VERSION_FLOOR, Version};

/// The outcome of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Verified,
    Failed,
    /// Not run (a prerequisite failed, or the owner skipped it).
    Skipped,
    /// Observed, for the record; no pass or fail.
    Info,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Verified => "verified",
            Status::Failed => "FAILED",
            Status::Skipped => "skipped",
            Status::Info => "info",
        }
    }
}

/// One row. `gate` marks the rows slice 4 depends on (FR-PRM-45: isolation first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub id: &'static str,
    pub claim: String,
    pub status: Status,
    pub detail: String,
    pub gate: bool,
}

impl Claim {
    pub fn new(id: &'static str, claim: impl Into<String>, gate: bool) -> Self {
        Self {
            id,
            claim: claim.into(),
            status: Status::Skipped,
            detail: String::new(),
            gate,
        }
    }

    pub fn set(&mut self, status: Status, detail: impl Into<String>) {
        self.status = status;
        self.detail = detail.into();
    }

    /// Verified if `ok`, else failed.
    pub fn check(&mut self, ok: bool, detail: impl Into<String>) {
        self.set(if ok { Status::Verified } else { Status::Failed }, detail);
    }
}

/// The minimum version the probe can stand behind: the installed one, if every gate row
/// is verified and it is at least [`MIN_VERSION_FLOOR`]. Older versions were never checked.
pub fn minimum_version(found: Option<Version>, claims: &[Claim]) -> Option<Version> {
    let found = found?;
    let gates_ok = claims
        .iter()
        .filter(|c| c.gate)
        .all(|c| c.status == Status::Verified);
    (gates_ok && found >= MIN_VERSION_FLOOR).then_some(found)
}

/// The summary the owner pastes back: a Markdown table plus the verdict lines. It holds
/// no paths, ids or account data (the probe only puts scrubbed text in `detail`).
pub fn render(claims: &[Claim], found: Option<Version>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "| # | Claim | Status | Detail |");
    let _ = writeln!(out, "|---|---|---|---|");
    for c in claims {
        let gate = if c.gate { " (gate)" } else { "" };
        let _ = writeln!(
            out,
            "| {} | {}{} | {} | {} |",
            c.id,
            cell(&c.claim),
            gate,
            c.status.label(),
            cell(&c.detail)
        );
    }
    let _ = writeln!(out);
    let found_text = found.map_or_else(|| "not found".to_owned(), |v| v.to_string());
    let _ = writeln!(out, "Installed version: {found_text}");
    match minimum_version(found, claims) {
        Some(v) => {
            let _ = writeln!(
                out,
                "Minimum version: {v} (every gate verified; floor {MIN_VERSION_FLOOR})"
            );
        }
        None => {
            let _ = writeln!(
                out,
                "Minimum version: not fixed (a gate row isn't verified). Chat stays disabled: isolation_unavailable (FR-PRM-45)."
            );
        }
    }
    out
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\r', '\n'], " ")
}

/// Names in `init` that contain `marker` (case-insensitive), as `list: name`.
pub fn marker_hits(init: &InitInfo, marker: &str) -> Vec<String> {
    let m = marker.to_lowercase();
    init.lists()
        .iter()
        .flat_map(|(list, names)| {
            names
                .iter()
                .filter(|n| n.to_lowercase().contains(&m))
                .map(move |n| format!("{list}: {n}"))
        })
        .collect()
}

/// Names in `init` that `baseline` (the same CLI, isolated, with an empty stand-in home)
/// doesn't have, as `list: name`. Anything here came from the real home's setup.
pub fn extra_over_baseline(init: &InitInfo, baseline: &InitInfo) -> Vec<String> {
    init.lists()
        .iter()
        .zip(baseline.lists().iter())
        .flat_map(|((list, names), (_, base))| {
            names
                .iter()
                .filter(|n| !base.contains(n))
                .map(move |n| format!("{list}: {n}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init(plugins: &[&str], skills: &[&str]) -> InitInfo {
        InitInfo {
            plugins: plugins.iter().map(|s| (*s).to_owned()).collect(),
            skills: skills.iter().map(|s| (*s).to_owned()).collect(),
            ..InitInfo::default()
        }
    }

    #[test]
    fn fr_prm_45_marker_hits_and_baseline_extras() {
        let a = init(&["PF-MARKER-plugin"], &["builtin"]);
        assert_eq!(marker_hits(&a, "pf-marker"), ["plugins: PF-MARKER-plugin"]);
        let real = init(&[], &["builtin", "my-own-skill"]);
        let base = init(&[], &["builtin"]);
        assert_eq!(extra_over_baseline(&real, &base), ["skills: my-own-skill"]);
        assert!(extra_over_baseline(&base, &base).is_empty());
    }

    #[test]
    fn prm_s0_minimum_version_needs_every_gate() {
        let v = Some(Version::new(2, 1, 319));
        let mut gate = Claim::new("C1", "isolation", true);
        let mut info = Claim::new("C2", "info", false);
        info.set(Status::Failed, "x");
        assert_eq!(minimum_version(v, &[gate.clone(), info.clone()]), None);
        gate.check(true, "ok");
        assert_eq!(minimum_version(v, &[gate.clone(), info.clone()]), v);
        assert_eq!(
            minimum_version(Some(Version::new(2, 1, 1)), std::slice::from_ref(&gate)),
            None
        );
        let table = render(&[gate, info], v);
        assert!(table.contains("| C1 | isolation (gate) | verified | ok |"));
        assert!(table.contains("Minimum version: 2.1.319"));
    }
}

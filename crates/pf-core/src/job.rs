//! The job record `.pforge/jobs/<id>/job.json` (manifest-schema §4.6, R2). The job file is
//! authoritative; manifest `jobs[]` is an index rebuilt on open. RGN's job runner drives the
//! transitions (R14); PRJ persists them. Fields whose meaning RGN or PRV owns stay raw JSON.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::manifest::Extra;
use crate::time::Timestamp;

pub const JOB_SCHEMA_VERSION: u32 = 1;
pub const JOB_FILE: &str = "job.json";
pub const REGION_INPUT: &str = "region.png";
pub const MASK_INPUT: &str = "mask.png";

/// Job states (schema §4.6 state diagram).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Requested,
    Submitted,
    Queued,
    Running,
    Succeeded,
    Fetched,
    Closed,
    Failed,
    Canceled,
    Unknown,
    Lost,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Requested => "requested",
            JobState::Submitted => "submitted",
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Succeeded => "succeeded",
            JobState::Fetched => "fetched",
            JobState::Closed => "closed",
            JobState::Failed => "failed",
            JobState::Canceled => "canceled",
            JobState::Unknown => "unknown",
            JobState::Lost => "lost",
        }
    }

    /// Terminal = `failed | canceled | lost | closed`.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobState::Failed | JobState::Canceled | JobState::Lost | JobState::Closed
        )
    }

    /// Every edge of the schema's state diagram.
    pub fn can_become(self, next: JobState) -> bool {
        use JobState::*;
        matches!(
            (self, next),
            (Requested, Submitted | Failed | Lost)
                | (Submitted, Queued | Running | Failed | Canceled | Unknown)
                | (Queued, Running | Failed | Canceled | Unknown)
                | (Running, Succeeded | Failed | Canceled | Unknown)
                | (Unknown, Queued | Running | Succeeded | Failed | Canceled)
                | (Succeeded, Fetched | Failed)
                | (Fetched, Closed)
        )
    }
}

/// `seed{requested, returned}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSeed {
    pub requested: Option<i64>,
    pub returned: Option<i64>,
}

/// `inputs{region, mask}`: the files as sent, inside the job folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobInputs {
    pub region: String,
    pub mask: String,
}

impl Default for JobInputs {
    fn default() -> Self {
        JobInputs {
            region: REGION_INPUT.into(),
            mask: MASK_INPUT.into(),
        }
    }
}

/// `iteration{n, dir}`: set by `add_candidates` before any candidate file is written (FR-RGN-39).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Iteration {
    pub n: u32,
    /// `results/NNN <slug>`.
    pub dir: String,
}

/// §4.6 Job file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobFile {
    pub schema_version: u32,
    pub id: Ulid,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub state: JobState,
    /// `{<state>: time}`, written on every transition (R2).
    #[serde(default)]
    pub state_times: BTreeMap<String, Timestamp>,
    #[serde(default)]
    pub resolved_at: Option<Timestamp>,
    #[serde(default)]
    pub operation: String,
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub capability_snapshot: Value,
    #[serde(default)]
    pub final_prompt: String,
    #[serde(default)]
    pub controls: BTreeMap<String, Value>,
    #[serde(default)]
    pub candidate_count: u32,
    #[serde(default)]
    pub seed: JobSeed,
    /// `{inner, outer, margin}` (R1).
    #[serde(default)]
    pub region: Value,
    /// `{sent_w, sent_h, scale}`.
    #[serde(default)]
    pub fit: Value,
    /// The draft mask the job was built from.
    #[serde(default)]
    pub mask_id: Option<Ulid>,
    #[serde(default)]
    pub inputs: JobInputs,
    /// The reference files sent, in request order (#51, FR-REF-25, FR-RGN-43); `[]` when none.
    /// Written with `requested`, never changed. REF owns the entry shape.
    #[serde(default)]
    pub references: Vec<Value>,
    #[serde(default)]
    pub estimate: Value,
    #[serde(default)]
    pub cost: Value,
    /// Opaque (PRV, provider spec Appendix B); persisted verbatim on every transition.
    #[serde(default)]
    pub provider: Value,
    #[serde(default)]
    pub iteration: Option<Iteration>,
    /// In provider order.
    #[serde(default)]
    pub candidate_ids: Vec<Ulid>,
    /// `{code, hint}` (ADR 0001).
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl JobFile {
    /// Project path of the job folder.
    pub fn dir(&self) -> String {
        crate::manifest::job_dir(self.id)
    }

    /// Project paths of the inputs.
    pub fn input_files(&self) -> [String; 2] {
        let d = self.dir();
        [
            format!("{d}/{}", self.inputs.region),
            format!("{d}/{}", self.inputs.mask),
        ]
    }

    /// Moves to `next` and stamps the time. Same state = no transition (a data update).
    pub fn transition(&mut self, next: JobState, now: Timestamp) -> Result<(), String> {
        if next == self.state {
            self.updated_at = now;
            return Ok(());
        }
        if !self.state.can_become(next) {
            return Err(format!(
                "a {} job can't become {}",
                self.state.as_str(),
                next.as_str()
            ));
        }
        self.state = next;
        self.state_times.insert(next.as_str().into(), now);
        self.updated_at = now;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prj_28_every_drawn_edge_is_allowed_and_terminals_have_none() {
        use JobState::*;
        let all = [
            Requested, Submitted, Queued, Running, Succeeded, Fetched, Closed, Failed, Canceled,
            Unknown, Lost,
        ];
        for s in all.iter().filter(|s| s.is_terminal()) {
            assert!(all.iter().all(|n| !s.can_become(*n)), "{s:?} is terminal");
        }
        assert!(Requested.can_become(Lost));
        assert!(Unknown.can_become(Succeeded));
        assert!(Succeeded.can_become(Fetched));
        assert!(!Requested.can_become(Running));
        assert!(!Fetched.can_become(Failed));
    }

    #[test]
    fn fr_prj_28_transition_stamps_state_times() {
        let mut j: JobFile = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": Ulid::from_parts(0, 1), "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z", "state": "requested",
            "provider": {"handle": "opaque", "nested": [1, 2]}
        }))
        .unwrap();
        j.transition(JobState::Submitted, Timestamp(5)).unwrap();
        assert_eq!(j.state_times["submitted"], Timestamp(5));
        assert!(j.transition(JobState::Fetched, Timestamp(6)).is_err());
        let back = serde_json::to_value(&j).unwrap();
        assert_eq!(back["provider"]["nested"][1], 2, "provider kept verbatim");
        assert_eq!(back["inputs"]["region"], "region.png");
    }
}

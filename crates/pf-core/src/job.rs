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
/// `operation` of a New image job (GEN, FR-GEN-22).
pub const OPERATION_GENERATE: &str = "generate";
/// `operation` of a region edit job.
pub const OPERATION_EDIT: &str = "edit_masked";

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

    /// Every edge of the schema's state diagram. R102 adds `→ lost` from the polled states
    /// (polling gave up after repeated errors, the handle kept).
    pub fn can_become(self, next: JobState) -> bool {
        use JobState::*;
        matches!(
            (self, next),
            (Requested, Submitted | Failed | Lost)
                | (
                    Submitted,
                    Queued | Running | Failed | Canceled | Unknown | Lost
                )
                | (Queued, Running | Failed | Canceled | Unknown | Lost)
                | (Running, Succeeded | Failed | Canceled | Unknown | Lost)
                | (
                    Unknown,
                    Queued | Running | Succeeded | Failed | Canceled | Lost
                )
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

fn default_inputs() -> Option<JobInputs> {
    Some(JobInputs::default())
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
    /// The bottom image's layer id (`layers[0]`, the original or a picked base image) a region
    /// edit job was built on (R97). Accept refuses a candidate made on another base image.
    /// Absent: a `generate` job, or a job from before R97 (not checked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_layer_id: Option<Ulid>,
    /// `null` for a `generate` job, which sends no input files (FR-GEN-22, schema §4.6).
    /// Absent (a job from before GEN) = the default names.
    #[serde(default = "default_inputs")]
    pub inputs: Option<JobInputs>,
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
    /// The prompt as the user typed it: `typed_prompt` (#55, the prompt before the blend
    /// instruction), else the prompt as sent (jobs from before #55).
    pub fn typed_prompt(&self) -> &str {
        self.extra
            .get("typed_prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&self.final_prompt)
    }

    /// The ticked helpers (FR-PRM-09, R94), in order, the ones not sent with `skipped`; a job
    /// from before PRM has none (`[]`), one from before R94 lists only those sent.
    pub fn prompt_helpers(&self) -> Vec<crate::prompt::SentHelper> {
        self.extra
            .get(crate::prompt::JOB_FIELD)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    /// Project path of the job folder.
    pub fn dir(&self) -> String {
        crate::manifest::job_dir(self.id)
    }

    /// Project paths of the inputs; none for a `generate` job (FR-GEN-22).
    pub fn input_files(&self) -> Vec<String> {
        let d = self.dir();
        match &self.inputs {
            Some(i) => vec![format!("{d}/{}", i.region), format!("{d}/{}", i.mask)],
            None => vec![],
        }
    }

    /// True once the provider's job handle is saved (`provider.job_handle`, PRV Appendix B): the
    /// job can be polled and fetched without a new submit.
    pub fn has_handle(&self) -> bool {
        self.provider
            .get("job_handle")
            .is_some_and(|h| !h.is_null())
    }

    /// True if this copy holds paid work: a job handle or candidates (R102).
    pub fn holds_paid_work(&self) -> bool {
        self.has_handle() || !self.candidate_ids.is_empty()
    }

    /// True for a New image job (FR-GEN-22).
    pub fn is_generate(&self) -> bool {
        self.operation == OPERATION_GENERATE
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
        // R102: polling gives up after repeated errors.
        for s in [Submitted, Queued, Running, Unknown] {
            assert!(s.can_become(Lost), "{s:?}");
        }
        assert!(
            !Succeeded.can_become(Lost),
            "a succeeded job keeps fetching"
        );
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

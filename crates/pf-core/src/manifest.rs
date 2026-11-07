//! Manifest v1 and revision file types ([manifest-schema](../../../docs/architecture/manifest-schema.md)).
//!
//! Shared-file rule (AGENTS §9): **add fields only**. Fields serialise in declared order; unknown
//! fields are kept (`extra`) and written after the known ones (FR-PRJ-27). Records other features
//! own (masks, jobs, candidates, trash, export) stay as raw JSON until their slices type them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::time::Timestamp;

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const REVISION_SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "manifest.json";
pub const REVISIONS_DIR: &str = "revisions";
/// The asset project of `<file>` is `<file>.pforge/` (FR-PRJ-12).
pub const PROJECT_SUFFIX: &str = ".pforge";

pub type Extra = BTreeMap<String, Value>;

/// `commit{seq, install_id, at}`: `seq` +1 per manifest write (FR-PRJ-35).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub seq: u64,
    pub install_id: Ulid,
    pub at: Timestamp,
}

/// §4.2 Original.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Original {
    /// Relative to the project folder, `/` separators (e.g. `../crypt.png`).
    pub file: String,
    pub content_hash: String,
    pub size: u64,
    pub mtime: Timestamp,
    pub width: u32,
    pub height: u32,
    /// `png|jpeg|webp`.
    pub format: String,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Original,
    Patch,
}

/// §4.3 Layer. Patch-only fields are omitted for the original layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: Ulid,
    pub kind: LayerKind,
    /// `null` for the original (it uses §4.2).
    pub file: Option<String>,
    pub cx: f64,
    pub cy: f64,
    pub scale: f64,
    pub rotation: f64,
    pub opacity: f64,
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Ulid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<Ulid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_id: Option<Ulid>,
    /// `{inner, outer, margin}`; typed by PRJ slice 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Value>,
    /// `{feather_px, colour_match{on, gain[3], offset[3]}}`; typed by PRJ slice 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retune_of: Option<Ulid>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Layer {
    /// The `[0]` layer: the original, centred, untransformed.
    pub fn original(id: Ulid, width: u32, height: u32) -> Self {
        Layer {
            id,
            kind: LayerKind::Original,
            file: None,
            cx: f64::from(width) / 2.0,
            cy: f64::from(height) / 2.0,
            scale: 1.0,
            rotation: 0.0,
            opacity: 1.0,
            visible: true,
            job_id: None,
            candidate_id: None,
            mask_id: None,
            region: None,
            rect: None,
            blend: None,
            retune_of: None,
            extra: Extra::new(),
        }
    }
}

/// §4.5 Draft: unsubmitted work, saved after a short idle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub final_prompt: String,
    pub model_id: Option<String>,
    /// Keyed by capability control `name` (#37).
    #[serde(default)]
    pub controls: BTreeMap<String, Value>,
    pub seed: Option<i64>,
    /// `{inner, margin}`; typed by RGN/PRJ slice 4.
    pub region: Option<Value>,
    pub mask_id: Option<Ulid>,
    pub updated_at: Timestamp,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Why a revision was made (FR-PRJ-23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionReason {
    Create,
    Accept,
    Save,
    Restore,
    Retune,
    Conflict,
}

/// Manifest `revisions[]` index row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevisionEntry {
    pub id: Ulid,
    pub seq: u64,
    pub created_at: Timestamp,
    pub reason: RevisionReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// `revisions/<id>.json`: a write-once snapshot of the layer stack plus the draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevisionFile {
    pub schema_version: u32,
    pub id: Ulid,
    pub seq: u64,
    pub created_at: Timestamp,
    pub reason: RevisionReason,
    pub label: Option<String>,
    /// The revision this one descends from (`null` for `r0`). Undo restores it (FR-PRJ-43).
    pub parent: Option<Ulid>,
    pub layers: Vec<Layer>,
    pub draft: Option<Draft>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl RevisionFile {
    pub fn entry(&self) -> RevisionEntry {
        RevisionEntry {
            id: self.id,
            seq: self.seq,
            created_at: self.created_at,
            reason: self.reason,
            label: self.label.clone(),
        }
    }
}

/// §4.1 Manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub project_id: Ulid,
    pub created_at: Timestamp,
    pub commit: Commit,
    pub original: Original,
    /// Current stack in draw order; `[0]` is the original.
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub masks: Vec<Value>,
    #[serde(default)]
    pub draft: Option<Draft>,
    /// `{id, state}` index, rebuilt from `jobs/*/job.json` on open (R2).
    #[serde(default)]
    pub jobs: Vec<Value>,
    #[serde(default)]
    pub candidates: Vec<Value>,
    #[serde(default)]
    pub revisions: Vec<RevisionEntry>,
    pub head_revision: Ulid,
    #[serde(default)]
    pub trash: Vec<Value>,
    #[serde(default)]
    pub export: Option<Value>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Reads just `schema_version` from manifest bytes, so a newer file is recognised even when
/// the rest doesn't parse as v1.
pub fn peek_schema_version(bytes: &[u8]) -> Option<u64> {
    serde_json::from_slice::<Value>(bytes)
        .ok()?
        .get("schema_version")?
        .as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        let id = |n: u128| Ulid::from_parts(0, n);
        Manifest {
            schema_version: 1,
            project_id: id(1),
            created_at: Timestamp(0),
            commit: Commit {
                seq: 1,
                install_id: id(2),
                at: Timestamp(0),
            },
            original: Original {
                file: "../crypt.png".into(),
                content_hash: "sha256:00".into(),
                size: 10,
                mtime: Timestamp(5),
                width: 100,
                height: 80,
                format: "png".into(),
                extra: Extra::new(),
            },
            layers: vec![Layer::original(id(3), 100, 80)],
            masks: vec![],
            draft: None,
            jobs: vec![],
            candidates: vec![],
            revisions: vec![RevisionEntry {
                id: id(4),
                seq: 0,
                created_at: Timestamp(0),
                reason: RevisionReason::Create,
                label: None,
            }],
            head_revision: id(4),
            trash: vec![],
            export: None,
            extra: Extra::new(),
        }
    }

    fn top_level_keys(text: &str) -> Vec<String> {
        text.lines()
            .filter(|l| l.starts_with("  \""))
            .map(|l| l.trim().split('"').nth(1).unwrap_or_default().to_owned())
            .collect()
    }

    #[test]
    fn fr_prj_27_manifest_fields_in_declared_order() {
        let text = String::from_utf8(crate::fsx::to_json_bytes(&sample()).unwrap()).unwrap();
        assert_eq!(
            top_level_keys(&text),
            [
                "schema_version",
                "project_id",
                "created_at",
                "commit",
                "original",
                "layers",
                "masks",
                "draft",
                "jobs",
                "candidates",
                "revisions",
                "head_revision",
                "trash",
                "export"
            ]
        );
        assert!(!text.contains('\r'));
    }

    #[test]
    fn fr_prj_27_unknown_fields_survive_a_round_trip_after_known_ones() {
        let mut v = serde_json::to_value(sample()).unwrap();
        v["zz_future"] = serde_json::json!({"a": 1});
        v["layers"][0]["glow"] = serde_json::json!(true);
        let m: Manifest = serde_json::from_value(v).unwrap();
        assert_eq!(m.extra["zz_future"]["a"], 1);
        assert_eq!(m.layers[0].extra["glow"], true);
        let text = String::from_utf8(crate::fsx::to_json_bytes(&m).unwrap()).unwrap();
        assert_eq!(top_level_keys(&text).last().unwrap(), "zz_future");
    }

    #[test]
    fn fr_prj_22_manifest_round_trip_is_field_identical() {
        let m = sample();
        let bytes = crate::fsx::to_json_bytes(&m).unwrap();
        let back: Manifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, m);
        assert_eq!(crate::fsx::to_json_bytes(&back).unwrap(), bytes);
    }

    #[test]
    fn original_layer_omits_patch_fields() {
        let v = serde_json::to_value(Layer::original(Ulid::from_parts(0, 1), 10, 20)).unwrap();
        assert_eq!(v["file"], Value::Null);
        assert_eq!(v["cx"], 5.0);
        assert!(v.get("job_id").is_none());
    }

    #[test]
    fn peeks_schema_version() {
        assert_eq!(
            peek_schema_version(br#"{"schema_version": 7, "x": []}"#),
            Some(7)
        );
        assert_eq!(peek_schema_version(b"nope"), None);
    }
}

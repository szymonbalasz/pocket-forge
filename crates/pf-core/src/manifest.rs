//! Manifest v1 (`project.json`), revision files and the project folder layout
//! ([manifest-schema](../../../docs/architecture/manifest-schema.md)). #44 redefined v1 in place
//! (project-as-folder); there is no migration code.
//!
//! Shared-file rule (AGENTS §9): **add fields only**. Fields serialise in declared order; unknown
//! fields are kept (`extra`) and written after the known ones (FR-PRJ-27). Records whose meaning
//! another feature owns (draft region, layer region and blend, export) stay raw JSON.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::job::JobState;
use crate::time::Timestamp;

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const REVISION_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------- layout (schema §2)

/// The manifest, at the top of the project folder (FR-PRJ-12).
pub const MANIFEST_FILE: &str = "project.json";
/// The hidden app folder.
pub const PFORGE_DIR: &str = ".pforge";
pub const REVISIONS_DIR: &str = ".pforge/revisions";
pub const LAYERS_DIR: &str = ".pforge/layers";
pub const MASKS_DIR: &str = ".pforge/masks";
pub const JOBS_DIR: &str = ".pforge/jobs";
pub const TRASH_DIR: &str = ".pforge/trash";
/// Home-grid thumbnail (FR-PRJ-50), rebuildable.
pub const THUMB_FILE: &str = ".pforge/thumb.jpg";
/// The original lives here (FR-PRJ-04); written only at creation and relink (FR-PRJ-47).
pub const WORKING_DIR: &str = "working";
pub const RESULTS_DIR: &str = "results";
pub const EXPORTS_DIR: &str = "exports";
/// User-owned folders: the app only adds copies (FR-PRJ-47).
pub const USER_DIRS: [&str; 3] = ["references", "structure", "parts"];
/// Every folder a new project gets (FR-PRJ-12).
pub const PROJECT_DIRS: [&str; 11] = [
    WORKING_DIR,
    "references",
    "structure",
    "parts",
    RESULTS_DIR,
    EXPORTS_DIR,
    REVISIONS_DIR,
    LAYERS_DIR,
    MASKS_DIR,
    JOBS_DIR,
    TRASH_DIR,
];

/// `kind` of a project, from its section folder (FR-PRJ-46).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Map,
    Token,
    Item,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Map, Kind::Token, Kind::Item];

    /// The section folder under a library root.
    pub fn section_dir(self) -> &'static str {
        match self {
            Kind::Map => "Maps",
            Kind::Token => "Tokens",
            Kind::Item => "Items",
        }
    }

    /// The kind of a section folder name (case-insensitive, as Windows folders are).
    pub fn from_section_dir(name: &str) -> Option<Kind> {
        Kind::ALL
            .into_iter()
            .find(|k| k.section_dir().eq_ignore_ascii_case(name))
    }
}

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
    /// `working/<source file name>`.
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

/// Integer image-px rectangle (schema §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Original,
    Patch,
}

/// §4.3 Layer. Patch-only fields are omitted for the original layer (R34).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// For the original layer this is the original's stable image id (#48 (3)).
    pub id: Ulid,
    pub kind: LayerKind,
    /// `.pforge/layers/<id>.png`; `null` for the original (it uses §4.2).
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
    /// `{inner, outer, margin}` (R1); RGN owns the meaning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<Value>,
    /// Placement of the cropped PNG.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    /// `{feather_px, colour_match{on, gain[3], offset[3]}}`; RGN owns the meaning.
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

/// §4.4 Mask: write-once 8-bit grey PNG, inner-rect size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub id: Ulid,
    /// `.pforge/masks/<id>.png`.
    pub file: String,
    pub rect: Rect,
    pub created_at: Timestamp,
    #[serde(flatten)]
    pub extra: Extra,
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
    /// `{inner, margin}`; RGN owns the meaning.
    pub region: Option<Value>,
    pub mask_id: Option<Ulid>,
    /// REF's `{packs, items}` (#51, FR-RGN-41); `null` = REF's defaults (FR-REF-17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub references: Option<Value>,
    pub updated_at: Timestamp,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Manifest `jobs[]` index row, rebuilt from the job files on open (R2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobIndex {
    pub id: Ulid,
    pub state: JobState,
    /// The job's `iteration.n`, so a home card counts iterations from this file alone.
    #[serde(default)]
    pub iteration: Option<u32>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// §4.7 candidate status (R10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    New,
    Accepted,
    Dismissed,
    Discarded,
    Trashed,
    Invalid,
}

/// §4.7 Candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: Ulid,
    pub job_id: Ulid,
    /// Provider order.
    pub index: u32,
    /// `results/NNN <slug>/NN.<ext>`.
    pub file: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub seed: Option<i64>,
    #[serde(default)]
    pub provider_meta: Value,
    pub status: CandidateStatus,
    #[serde(default)]
    pub starred: bool,
    /// Set while the file is in the trash. After a purge it still names the purged item, and
    /// `purged_at` says the file is gone.
    #[serde(default)]
    pub trash_id: Option<Ulid>,
    /// When the 30-day purge deleted the file (FR-PRJ-32, BL-23). The record stays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purged_at: Option<Timestamp>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// §4.8 trash item kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashKind {
    Candidates,
    JobInputs,
    Mask,
    Layer,
    Manifest,
    Orphan,
    ConflictCopy,
}

/// §4.8 Trash item. Each `files[]` entry is a project path; its trashed copy is
/// `.pforge/trash/<id>/<path>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrashItem {
    pub id: Ulid,
    pub kind: TrashKind,
    pub files: Vec<String>,
    pub trashed_at: Timestamp,
    pub reason: String,
    #[serde(flatten)]
    pub extra: Extra,
}

impl TrashItem {
    /// Where a trashed file sits: `.pforge/trash/<id>/<from>`.
    pub fn trashed_path(&self, from: &str) -> String {
        format!("{TRASH_DIR}/{}/{from}", self.id)
    }
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

/// `.pforge/revisions/<id>.json`: a write-once snapshot of the layer stack plus the draft.
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

/// `.pforge/revisions/<id>.json`.
pub fn revision_file(id: Ulid) -> String {
    format!("{REVISIONS_DIR}/{id}.json")
}

/// `.pforge/layers/<id>.png`.
pub fn layer_file(id: Ulid) -> String {
    format!("{LAYERS_DIR}/{id}.png")
}

/// `.pforge/masks/<id>.png`.
pub fn mask_file(id: Ulid) -> String {
    format!("{MASKS_DIR}/{id}.png")
}

/// `.pforge/jobs/<id>`.
pub fn job_dir(id: Ulid) -> String {
    format!("{JOBS_DIR}/{id}")
}

/// §4.1 Manifest (`project.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub project_id: Ulid,
    pub kind: Kind,
    pub created_at: Timestamp,
    pub commit: Commit,
    pub original: Original,
    /// Current stack in draw order; `[0]` is the original (R6).
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub masks: Vec<Mask>,
    #[serde(default)]
    pub draft: Option<Draft>,
    /// Index, rebuilt from `.pforge/jobs/*/job.json` on open (R2).
    #[serde(default)]
    pub jobs: Vec<JobIndex>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    /// Index, rebuilt from `.pforge/revisions/*.json` on open.
    #[serde(default)]
    pub revisions: Vec<RevisionEntry>,
    pub head_revision: Ulid,
    #[serde(default)]
    pub trash: Vec<TrashItem>,
    /// EXP owns the meaning (schema §4.10).
    #[serde(default)]
    pub export: Option<Value>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Manifest {
    /// The original layer's id: the original's stable image id (#48 (3)).
    pub fn original_image_id(&self) -> Option<Ulid> {
        self.layers
            .iter()
            .find(|l| l.kind == LayerKind::Original)
            .map(|l| l.id)
    }

    /// How many iterations (jobs with an iteration number) the project has.
    pub fn iteration_count(&self) -> usize {
        self.jobs.iter().filter(|j| j.iteration.is_some()).count()
    }
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
            kind: Kind::Map,
            created_at: Timestamp(0),
            commit: Commit {
                seq: 1,
                install_id: id(2),
                at: Timestamp(0),
            },
            original: Original {
                file: "working/crypt.png".into(),
                content_hash: "sha256:00".into(),
                size: 10,
                mtime: Timestamp(5),
                width: 100,
                height: 80,
                format: "png".into(),
                extra: Extra::new(),
            },
            layers: vec![Layer::original(id(3), 100, 80)],
            masks: vec![Mask {
                id: id(5),
                file: mask_file(id(5)),
                rect: Rect {
                    x: 1,
                    y: 2,
                    w: 3,
                    h: 4,
                },
                created_at: Timestamp(0),
                extra: Extra::new(),
            }],
            draft: None,
            jobs: vec![JobIndex {
                id: id(6),
                state: JobState::Fetched,
                iteration: Some(1),
                extra: Extra::new(),
            }],
            candidates: vec![Candidate {
                id: id(7),
                job_id: id(6),
                index: 0,
                file: "results/001 moss/01.png".into(),
                format: "png".into(),
                width: 8,
                height: 8,
                seed: Some(3),
                provider_meta: serde_json::json!({"x": 1}),
                status: CandidateStatus::New,
                starred: false,
                trash_id: None,
                purged_at: None,
                extra: Extra::new(),
            }],
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
                "kind",
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
        v["candidates"][0]["score"] = serde_json::json!(0.5);
        let m: Manifest = serde_json::from_value(v).unwrap();
        assert_eq!(m.extra["zz_future"]["a"], 1);
        assert_eq!(m.layers[0].extra["glow"], true);
        assert_eq!(m.candidates[0].extra["score"], 0.5);
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
    fn fr_prj_12_paths_are_project_relative() {
        let id = Ulid::from_parts(0, 9);
        assert_eq!(
            layer_file(id),
            format!(".pforge/layers/{id}.png"),
            "layers live in .pforge"
        );
        assert_eq!(mask_file(id), format!(".pforge/masks/{id}.png"));
        assert_eq!(revision_file(id), format!(".pforge/revisions/{id}.json"));
        assert_eq!(job_dir(id), format!(".pforge/jobs/{id}"));
        let text = String::from_utf8(crate::fsx::to_json_bytes(&sample()).unwrap()).unwrap();
        assert!(text.contains(r#""file": "working/crypt.png""#));
    }

    #[test]
    fn fr_prj_46_kind_follows_the_section_folder() {
        assert_eq!(Kind::from_section_dir("Maps"), Some(Kind::Map));
        assert_eq!(Kind::from_section_dir("tokens"), Some(Kind::Token));
        assert_eq!(Kind::from_section_dir("Items"), Some(Kind::Item));
        assert_eq!(Kind::from_section_dir("Imported"), None);
        assert_eq!(serde_json::to_value(Kind::Token).unwrap(), "token");
    }

    #[test]
    fn r34_original_layer_omits_patch_fields() {
        let v = serde_json::to_value(Layer::original(Ulid::from_parts(0, 1), 10, 20)).unwrap();
        assert_eq!(v["file"], Value::Null);
        assert_eq!(v["cx"], 5.0);
        assert!(v.get("job_id").is_none() && v.get("rect").is_none());
    }

    #[test]
    fn fr_prj_26_peeks_schema_version() {
        assert_eq!(
            peek_schema_version(br#"{"schema_version": 7, "x": []}"#),
            Some(7)
        );
        assert_eq!(peek_schema_version(b"nope"), None);
    }
}

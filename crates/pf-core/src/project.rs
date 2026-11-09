//! The asset project store (PRJ): open/close, the commit protocol, revisions, the draft, relink,
//! project folders and image resolution. The store API other features call (jobs, candidates,
//! masks, layers, export) is in [`store`]; open-time recovery is in `sweep`.
//!
//! A project is a folder `<root>/<Maps|Tokens|Items>/<name>/` with `project.json` (#44). Every
//! mutating call commits before it returns (FR-PRJ-18): new write-once files, then
//! `project.json`, each temp → flush → rename. The manifest rename is the commit point.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use ulid::Ulid;

use crate::context::Context;
use crate::decoded::DecodedOriginal;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::job::{JOB_FILE, JobFile};
use crate::library;
use crate::manifest::*;
use crate::settings::Settings;
use crate::thumb::{self, ThumbJob, Thumbnails};
use crate::time::Timestamp;

pub mod commands;
pub mod conflict;
pub mod retention;
pub mod spool;
pub mod store;
mod sweep;

/// Why a project is read-only (FR-PRJ-15/16/26, FR-PRJ-41).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadOnlyReason {
    /// "Made by a newer Pocket Forge".
    NewerVersion,
    /// "Original changed".
    OriginalChanged,
    /// "Original missing": Relink clears it (FR-PRJ-16).
    OriginalMissing,
    /// "Base image changed": a picked base image (GEN) no longer matches its `content_hash`,
    /// exactly like a changed original (FR-PRJ-15, R84).
    BaseChanged,
}

/// A non-fatal note from `project_open`: `newer_version`, `original_changed`,
/// `original_missing`, `kind_changed`, `conflict`, `recovered`, `resumable`, `lost`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Warning {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub detail: Value,
}

impl Warning {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Warning {
            code,
            message: message.into(),
            detail: Value::Null,
        }
    }
}

/// Ids that write-once revision files reference (so commits never trash them).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RevisionRefs {
    pub layers: HashSet<Ulid>,
    pub masks: HashSet<Ulid>,
    /// Bottom-image layers of revisions: id → `Some(file)` for a base image (FR-GEN-27), `None`
    /// for the original (its file is the manifest's `original.file`).
    pub base_images: HashMap<Ulid, Option<String>>,
}

impl RevisionRefs {
    pub(crate) fn add(&mut self, rev: &RevisionFile) {
        for l in &rev.layers {
            self.layers.insert(l.id);
            self.masks.extend(l.mask_id);
            match l.kind {
                LayerKind::Base => {
                    self.base_images.insert(l.id, l.file.clone());
                }
                LayerKind::Original => {
                    self.base_images.insert(l.id, None);
                }
                LayerKind::Patch => {}
            }
        }
        self.masks
            .extend(rev.draft.as_ref().and_then(|d| d.mask_id));
    }
}

/// An asset project held in memory while it's open.
#[derive(Debug)]
pub struct OpenProject {
    /// Absolute project folder.
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// The `commit{seq, install_id}` last loaded or written (FR-PRJ-35).
    pub last_commit: Option<(u64, Ulid)>,
    pub read_only: Option<ReadOnlyReason>,
    pub warnings: Vec<Warning>,
    /// The job files, authoritative (R2), as last loaded or written.
    pub jobs: BTreeMap<Ulid, JobFile>,
    pub(crate) refs: RevisionRefs,
    /// FR-PRJ-42, R11.
    pub decoded: DecodedOriginal,
    /// Where thumbnails are queued (`None` in unit tests that don't need them).
    pub thumbs: Option<Thumbnails>,
    /// A draft save that failed with `locked` (retried every 10 s, FR-PRJ-21) or `conflict` (saved
    /// by the resolve).
    pub pending_draft: Option<PendingDraft>,
    /// Set while a sync conflict is open: every commit is refused with `conflict` until
    /// `project_resolve_conflict` (FR-PRJ-34 … 36).
    pub conflict: Option<conflict::ConflictSource>,
}

/// A draft save kept in memory after `locked` (FR-PRJ-21).
#[derive(Debug, Clone, PartialEq)]
pub struct PendingDraft {
    pub draft: DraftInput,
    pub export: Option<serde_json::Map<String, Value>>,
}

/// How often a save that failed with `locked` is retried (FR-PRJ-21).
pub const SAVE_RETRY_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

/// `project:save-failed{reason, retrying}` (FR-PRJ-21).
pub(crate) fn emit_save_failed(ctx: &Context, reason: &str, retrying: bool) {
    ctx.emit(
        "project:save-failed",
        serde_json::json!({ "reason": reason, "retrying": retrying }),
    );
}

/// `detail.reason` of the error for a project folder moved or renamed while open (§11).
pub const FOLDER_MOVED: &str = "folder_moved";

/// §11: the folder was moved or renamed in Explorer while open. `not_found` with
/// `detail.reason = folder_moved`; the message never carries a path.
pub(crate) fn folder_moved() -> CoreError {
    CoreError::new(
        ErrorCode::NotFound,
        "The project folder was moved or renamed outside Pocket Forge. Reopen it from home.",
    )
    .with_detail(serde_json::json!({ "reason": FOLDER_MOVED }))
}

/// True for [`folder_moved`] errors.
pub fn is_folder_moved(e: &CoreError) -> bool {
    e.code == ErrorCode::NotFound && e.detail["reason"] == FOLDER_MOVED
}

/// §11: refuses with [`folder_moved`] once the folder is gone, and notes it in the warnings
/// (the panel's "reopen from home" banner). State stays in memory.
pub(crate) fn require_folder(p: &mut OpenProject) -> CoreResult<()> {
    if p.dir.is_dir() {
        return Ok(());
    }
    if !p.warnings.iter().any(|w| w.code == FOLDER_MOVED) {
        p.warnings.push(Warning::new(
            FOLDER_MOVED,
            "The project folder was moved or renamed outside Pocket Forge. Reopen it from home.",
        ));
    }
    Err(folder_moved())
}

/// What `project_open` returns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectSnapshot {
    pub project_id: Ulid,
    /// The folder name (FR-PRJ-44).
    pub name: String,
    pub kind: Kind,
    /// Absolute project folder.
    pub project_dir: PathBuf,
    /// Absolute path of `layers[0]`'s file (the original or the base image); `null` with no image.
    pub image_path: Option<PathBuf>,
    /// `layers[0].id`: the original's stable image id (#48 (3)) or the base image's layer id;
    /// `null` with no image (FR-GEN-03).
    pub original_image_id: Option<Ulid>,
    /// False for a project made by Generate new until its first pick (FR-GEN-03).
    pub has_image: bool,
    pub read_only: Option<ReadOnlyReason>,
    pub warnings: Vec<Warning>,
    /// `layers[0]`'s size; 0 × 0 with no image.
    pub width: u32,
    pub height: u32,
    /// `layers[0]`'s format; `null` with no image.
    pub format: Option<String>,
    pub layers: Vec<Layer>,
    pub draft: Option<Draft>,
    pub revisions: Vec<RevisionEntry>,
    /// The manifest's candidate records (R44), for RGN's tracker and EDT's picker.
    pub candidates: Vec<Candidate>,
    pub head_revision: Ulid,
    pub commit_seq: u64,
    /// Always true: a project exists on disk from its creation (FR-PRJ-09). Kept for the UI.
    pub saved: bool,
    /// `commit.at`: last edited.
    pub committed_at: Timestamp,
    /// EXP's record (schema §4.10).
    pub export: Option<Value>,
}

impl OpenProject {
    /// The warnings as of now. `resumable` (FR-PRJ-29) names only jobs found on open that are
    /// still running: a job this session started never counts, and the warning ends with the
    /// last of them (walk-p4r L1).
    fn live_warnings(&self) -> Vec<Warning> {
        let mut out = Vec::with_capacity(self.warnings.len());
        for w in &self.warnings {
            if w.code != "resumable" {
                out.push(w.clone());
                continue;
            }
            let jobs: Vec<Value> = w.detail["jobs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|j| {
                    let id: Ulid = j["job_id"].as_str()?.parse().ok()?;
                    let job = self.jobs.get(&id).filter(|f| !f.state.is_terminal())?;
                    Some(serde_json::json!({ "job_id": id, "state": job.state }))
                })
                .collect();
            if !jobs.is_empty() {
                out.push(Warning {
                    code: w.code,
                    message: format!("{} job(s) can resume.", jobs.len()),
                    detail: serde_json::json!({ "jobs": jobs }),
                });
            }
        }
        out
    }

    pub fn snapshot(&self) -> ProjectSnapshot {
        let m = &self.manifest;
        let (width, height) = m.image_size().unwrap_or((0, 0));
        ProjectSnapshot {
            project_id: m.project_id,
            name: folder_name(&self.dir),
            kind: m.kind,
            project_dir: self.dir.clone(),
            image_path: self.base_path(),
            original_image_id: m.original_image_id(),
            has_image: m.has_image(),
            read_only: self.read_only,
            warnings: self.live_warnings(),
            width,
            height,
            format: m.image_format(),
            layers: m.layers.clone(),
            draft: m.draft.clone(),
            revisions: m.revisions.clone(),
            candidates: m.candidates.clone(),
            head_revision: m.head_revision,
            commit_seq: self.last_commit.map_or(0, |c| c.0),
            saved: true,
            committed_at: m.commit.at,
            export: m.export.clone(),
        }
    }

    /// Absolute path of a project path (`working/x.png`, `.pforge/…`).
    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    /// Absolute path of the original; `None` for a project made by Generate new (FR-GEN-02).
    pub fn original_path(&self) -> Option<PathBuf> {
        self.manifest.original.as_ref().map(|o| self.path(&o.file))
    }

    /// Absolute path of `layers[0]`'s file: the original or the base image (FR-GEN-33).
    pub fn base_path(&self) -> Option<PathBuf> {
        self.manifest.base_file().map(|f| self.path(f))
    }

    /// Fails with `no_image` while the project has no image (FR-GEN-03).
    pub fn require_image(&self) -> CoreResult<()> {
        if self.manifest.has_image() {
            Ok(())
        } else {
            Err(crate::decoded::no_image())
        }
    }

    fn manifest_path(&self) -> PathBuf {
        self.path(MANIFEST_FILE)
    }

    /// Fails fast with `read_only` (FR-PRJ-41).
    pub fn require_writable(&self) -> CoreResult<()> {
        match self.read_only {
            Some(reason) => Err(CoreError::new(
                ErrorCode::ReadOnly,
                "this asset project is read-only",
            )
            .with_detail(serde_json::json!({ "reason": reason }))),
            None => Ok(()),
        }
    }

    fn request_thumbnail(&self, ctx: &Context) {
        if let Some(t) = &self.thumbs {
            t.request(ThumbJob::new(ctx, &self.dir, &self.manifest));
        }
    }
}

/// The decoded `layers[0]` handle for a manifest (FR-PRJ-42 amended, FR-GEN-33).
pub(crate) fn decoded_for(dir: &Path, m: &Manifest) -> DecodedOriginal {
    match m.base_file() {
        Some(f) => DecodedOriginal::new(m.project_id, dir.join(f)),
        None => DecodedOriginal::none(m.project_id),
    }
}

pub(crate) fn folder_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub(crate) fn mtime_of(meta: &std::fs::Metadata) -> Timestamp {
    meta.modified()
        .map(Timestamp::from_system)
        .unwrap_or(Timestamp(0))
}

pub(crate) fn probe_error(e: pf_imaging::ProbeError) -> CoreError {
    use pf_imaging::ProbeError as P;
    let code = match &e {
        P::Unsupported => ErrorCode::Unsupported,
        P::Animated => ErrorCode::Animated,
        P::TooLarge { .. } => ErrorCode::TooLarge,
        P::Corrupt(_) => ErrorCode::Corrupt,
        P::Io(io) => return CoreError::io("reading the image", io),
    };
    CoreError::new(code, e.to_string())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> CoreResult<T> {
    let bytes = std::fs::read(path)
        .map_err(|e| CoreError::io(format!("reading {}", path.display()), &e))?;
    serde_json::from_slice(&bytes).map_err(|e| {
        CoreError::new(
            ErrorCode::Corrupt,
            format!("{} is unreadable: {e}", path.display()),
        )
    })
}

// ---------------------------------------------------------------- open

/// `project_open` (FR-PRJ-14 … 17, 20, 24, 26, 29): opens a project folder inside a root.
pub fn open(
    ctx: &Context,
    settings: &Settings,
    path: &Path,
    thumbs: Option<Thumbnails>,
) -> CoreResult<OpenProject> {
    let dir = library::require_project_folder(ctx, settings, path)?;
    if !dir.is_dir() {
        return Err(library::not_a_project(&dir));
    }
    let manifest_path = dir.join(MANIFEST_FILE);
    let mut warnings = vec![];
    let mut read_only = None;
    let manifest = if std::fs::symlink_metadata(&manifest_path).is_ok() {
        library::hydrate_if_needed(ctx, &manifest_path)?;
        let bytes =
            std::fs::read(&manifest_path).map_err(|e| CoreError::io("reading project.json", &e))?;
        let version = peek_schema_version(&bytes);
        if version.is_some_and(|v| v > u64::from(MANIFEST_SCHEMA_VERSION)) {
            read_only = Some(ReadOnlyReason::NewerVersion);
            warnings.push(Warning::new(
                "newer_version",
                "Made by a newer Pocket Forge. The project opens read-only.",
            ));
        }
        match serde_json::from_slice::<Manifest>(&bytes) {
            Ok(m) => Some(m),
            Err(e) if read_only.is_some() => {
                return Err(CoreError::new(
                    ErrorCode::Corrupt,
                    format!("project.json was made by a newer Pocket Forge and can't be read: {e}"),
                ));
            }
            Err(e) => {
                tracing::warn!(error = %e, "project.json unreadable; rebuilding");
                None
            }
        }
    } else {
        None
    };
    let (manifest, recovered) = match manifest {
        Some(m) => (m, false),
        // FR-PRJ-24: rebuilt from the revision and job files, if there are any.
        None => match sweep::rebuild_manifest(ctx, settings, &dir)? {
            Some(m) => (m, true),
            None => return Err(library::not_a_project(&dir)),
        },
    };
    if recovered {
        warnings.push(Warning::new(
            "recovered",
            "project.json was missing or unreadable and was rebuilt from the history.",
        ));
        ctx.emit(
            "project:recovered",
            serde_json::json!({ "project_id": manifest.project_id }),
        );
    }

    let writable_layout = read_only.is_none();
    if writable_layout {
        // Leftovers of an interrupted commit (FR-PRJ-20). User-owned folders are never touched.
        sweep::remove_temp_files(&dir);
    }

    let mut p = OpenProject {
        decoded: decoded_for(&dir, &manifest),
        last_commit: (!recovered).then_some((manifest.commit.seq, manifest.commit.install_id)),
        dir,
        manifest,
        read_only,
        warnings,
        jobs: BTreeMap::new(),
        refs: RevisionRefs::default(),
        thumbs,
        pending_draft: None,
        conflict: None,
    };
    check_original(ctx, &mut p)?;
    check_base(ctx, &mut p)?;
    if let Some(kind) = library::kind_of_folder(&p.dir)
        && kind != p.manifest.kind
    {
        // FR-PRJ-46: the folder wins; written at the next commit.
        p.manifest.kind = kind;
        p.warnings.push(Warning::new(
            "kind_changed",
            format!("The project is in the {} folder now.", kind.section_dir()),
        ));
    }
    if let Some(copy) = conflict::manifest_copies(&p.dir).into_iter().next() {
        // FR-PRJ-34: shown on open; writes wait for the user's pick.
        conflict::mark(ctx, &mut p, conflict::ConflictSource::Copy(copy));
    }
    sweep::load_history(&mut p)?;
    if recovered && p.read_only.is_none() {
        // Write the rebuilt manifest at once so the recovery is durable.
        p.last_commit = None;
        write_recovered(ctx, settings, &mut p)?;
    }
    sweep::on_open(ctx, settings.install_id, &mut p)?;
    if p.read_only != Some(ReadOnlyReason::NewerVersion)
        && p.read_only != Some(ReadOnlyReason::OriginalMissing)
        && thumb::needs_rebuild(&p.dir)
        // With no image only a stale thumbnail needs work: it is removed (FR-GEN-03).
        && (p.manifest.has_image() || p.dir.join(THUMB_FILE).is_file())
    {
        p.request_thumbnail(ctx);
    }
    Ok(p)
}

/// FR-PRJ-15/16/17: hydrate, then compare size and mtime, then the hash. A project made by
/// Generate new has no original to check (FR-GEN-02).
fn check_original(ctx: &Context, p: &mut OpenProject) -> CoreResult<()> {
    let Some(path) = p.original_path() else {
        return Ok(());
    };
    let meta = match std::fs::symlink_metadata(&path) {
        Err(_) => {
            p.read_only.get_or_insert(ReadOnlyReason::OriginalMissing);
            p.warnings.push(Warning::new(
                "original_missing",
                "Original missing. Relink it to keep working; until then the project is read-only.",
            ));
            return Ok(());
        }
        Ok(m) => m,
    };
    library::hydrate_if_needed(ctx, &path)?;
    // R33: refuse an original that fails the cheap end-of-file check.
    pf_imaging::probe_file_complete(&path).map_err(probe_error)?;
    let meta = std::fs::metadata(&path).unwrap_or(meta);
    let Some(o) = p.manifest.original.as_ref() else {
        return Ok(());
    };
    // R45: hashed on every open until the project's first commit after creation (seq 1), and
    // after that whenever the racily-clean window says size + mtime can't be trusted.
    let never_committed = p.manifest.commit.seq <= CREATION_SEQ;
    if meta.len() != o.size
        || mtime_of(&meta) != o.mtime
        || never_committed
        || racily_clean(&p.dir, o.mtime)
    {
        let hash = fsx::hash_file(&path).map_err(|e| CoreError::io("hashing the original", &e))?;
        if hash != o.content_hash {
            p.read_only.get_or_insert(ReadOnlyReason::OriginalChanged);
            p.warnings.push(Warning::new(
                "original_changed",
                "Original changed outside Pocket Forge. The project opens read-only.",
            ));
        }
    }
    Ok(())
}

/// R84 (FR-PRJ-15 for a picked base image): `layers[0]`'s `working/` file is write-once, so a
/// file that is gone or whose hash differs from the base layer's `content_hash` was changed
/// outside the app: the project opens read-only, and region edit and export never use the wrong
/// pixels. The base layer keeps no size or mtime, so the file is hashed on every open.
fn check_base(ctx: &Context, p: &mut OpenProject) -> CoreResult<()> {
    let Some(layer) = p
        .manifest
        .base_layer()
        .filter(|l| l.kind == LayerKind::Base)
    else {
        return Ok(());
    };
    let (Some(file), Some(want)) = (layer.file.clone(), layer.content_hash.clone()) else {
        return Ok(());
    };
    let path = p.path(&file);
    let changed = if std::fs::symlink_metadata(&path).is_err() {
        true
    } else {
        library::hydrate_if_needed(ctx, &path)?;
        fsx::hash_file(&path).map_err(|e| CoreError::io("hashing the base image", &e))? != want
    };
    if changed {
        p.read_only.get_or_insert(ReadOnlyReason::BaseChanged);
        p.warnings.push(Warning::new(
            "base_changed",
            "Base image changed or missing outside Pocket Forge. The project opens read-only.",
        ));
    }
    Ok(())
}

/// The `commit.seq` creation writes (FR-PRJ-09).
const CREATION_SEQ: u64 = 1;

/// Size + mtime can't see a rewrite within one mtime tick of the recorded one. When the recorded
/// mtime is within [`RACY_WINDOW_MS`] of `project.json`'s own mtime, the stored hash decides
/// (git's "racily clean" rule). Once a later commit rewrites `project.json`, a change to the
/// original shows up as a new mtime again and open stays hash-free.
const RACY_WINDOW_MS: i64 = 2000;

fn racily_clean(dir: &Path, recorded: Timestamp) -> bool {
    std::fs::metadata(dir.join(MANIFEST_FILE))
        .map(|m| mtime_of(&m).millis() <= recorded.millis() + RACY_WINDOW_MS)
        .unwrap_or(true)
}

fn write_recovered(ctx: &Context, settings: &Settings, p: &mut OpenProject) -> CoreResult<()> {
    let _ = settings;
    let seq = p.manifest.commit.seq + 1;
    p.manifest.commit = Commit {
        seq,
        install_id: p.manifest.commit.install_id,
        at: ctx.now(),
    };
    fsx::write_json_atomic(ctx, &p.manifest_path(), &p.manifest)?;
    p.last_commit = Some((seq, p.manifest.commit.install_id));
    Ok(())
}

// ---------------------------------------------------------------- commit

/// What a commit writes besides the manifest.
#[derive(Debug, Default)]
pub struct CommitPlan {
    /// Write-once files written before the manifest: project path → bytes.
    pub files: Vec<(String, Vec<u8>)>,
    pub revisions: Vec<RevisionFile>,
    /// Job files written before the manifest (a new job, an iteration folder).
    pub jobs_before: Vec<JobFile>,
    /// Job files written after the manifest (follow-ups rolled forward on open).
    pub jobs_after: Vec<JobFile>,
}

/// One commit (FR-PRJ-18). `change` edits a copy of the manifest and fills the plan; on any
/// error before the commit point the in-memory project is unchanged. Returns the new `seq`.
pub fn commit(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    change: impl FnOnce(&Context, &OpenProject, &mut Manifest, &mut CommitPlan) -> CoreResult<()>,
) -> CoreResult<u64> {
    commit_with(ctx, install_id, p, false, change)
}

/// [`commit`]; `resolving` = the conflict resolve's own commit, which skips the conflict checks
/// (FR-PRJ-34, 35) because it is what ends the conflict.
pub(crate) fn commit_with(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    resolving: bool,
    change: impl FnOnce(&Context, &OpenProject, &mut Manifest, &mut CommitPlan) -> CoreResult<()>,
) -> CoreResult<u64> {
    p.require_writable()?;
    require_folder(p)?;
    if !resolving {
        conflict::check_before_commit(ctx, p)?;
    }
    // FR-PRJ-34, OQ-PRJ-10 A: job.json copies settle (the later state wins); they and thumbnail
    // copies go to the trash with this commit.
    conflict::settle_job_copies(ctx, p);
    conflict::settle_chat_copies(ctx, p);
    let copies = conflict::side_copies(p);
    let mut next = p.manifest.clone();
    let mut plan = CommitPlan::default();
    change(ctx, p, &mut next, &mut plan)?;
    if !copies.is_empty() {
        store::trash_files(
            ctx,
            &mut next,
            TrashKind::ConflictCopy,
            copies,
            "sync conflict copy",
        );
    }
    if let Some(kind) = library::kind_of_folder(&p.dir) {
        next.kind = kind;
    }
    store::trash_superseded_masks(ctx, p, &mut next, &plan);
    if !p.dir.is_dir() {
        // Renamed or moved in Explorer during the change (§11): state stays in memory.
        return Err(folder_moved());
    }

    for (rel, bytes) in &plan.files {
        let target = p.path(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::io(format!("creating {}", parent.display()), &e))?;
        }
        fsx::write_atomic(ctx, &target, bytes)?;
    }
    for rev in &plan.revisions {
        fsx::write_json_atomic(ctx, &p.path(&revision_file(rev.id)), rev)?;
    }
    for job in &plan.jobs_before {
        write_job(ctx, p, job)?;
    }
    let seq = p.last_commit.map_or(0, |c| c.0) + 1;
    next.commit = Commit {
        seq,
        install_id,
        at: ctx.now(),
    };
    fsx::write_json_atomic(ctx, &p.manifest_path(), &next)?;

    // Committed. Everything below is rolled forward on open if it fails.
    let layers_changed = next.layers != p.manifest.layers;
    let base_changed = next.base_file() != p.manifest.base_file();
    p.manifest = next;
    if base_changed {
        // A pick, or a restore across one (FR-GEN-27, 29): the next decode reads the new file.
        p.decoded.release();
        p.decoded = decoded_for(&p.dir, &p.manifest);
    }
    p.last_commit = Some((seq, install_id));
    for rev in &plan.revisions {
        p.refs.add(rev);
    }
    for job in plan.jobs_before {
        p.jobs.insert(job.id, job);
    }
    for job in plan.jobs_after {
        if let Err(e) = write_job(ctx, p, &job) {
            tracing::warn!(job = %job.id, error = %e, "job follow-up not written; rolled forward on open");
        }
        p.jobs.insert(job.id, job);
    }
    sweep::apply_file_moves(p);
    ctx.emit("project:committed", serde_json::json!({ "seq": seq }));
    if layers_changed {
        p.request_thumbnail(ctx);
    }
    Ok(seq)
}

pub(crate) fn write_job(ctx: &Context, p: &OpenProject, job: &JobFile) -> CoreResult<()> {
    let path = p.path(&format!("{}/{JOB_FILE}", job.dir()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CoreError::io(format!("creating {}", parent.display()), &e))?;
    }
    fsx::write_json_atomic(ctx, &path, job)
}

/// The on-disk `commit{seq, install_id}` of `project.json`, if it can be read.
pub(crate) fn disk_commit(p: &OpenProject) -> Option<(u64, Ulid)> {
    let disk: Value = serde_json::from_slice(&std::fs::read(p.manifest_path()).ok()?).ok()?;
    let seq = disk["commit"]["seq"].as_u64()?;
    let install = disk["commit"]["install_id"]
        .as_str()?
        .parse::<Ulid>()
        .ok()?;
    Some((seq, install))
}

/// FR-PRJ-35: the on-disk commit must still be the one last loaded or written.
fn check_commit_unchanged(p: &OpenProject) -> CoreResult<()> {
    let conflict = || {
        CoreError::new(
            ErrorCode::Conflict,
            "project.json changed on disk (another machine?)",
        )
    };
    let bytes = match std::fs::read(p.manifest_path()) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !p.dir.is_dir() => {
            return Err(folder_moved());
        }
        Err(_) => return Err(conflict()),
    };
    let disk: Value = serde_json::from_slice(&bytes).map_err(|_| conflict())?;
    let seq = disk["commit"]["seq"].as_u64();
    let install = disk["commit"]["install_id"]
        .as_str()
        .and_then(|s| s.parse::<Ulid>().ok());
    match (p.last_commit, seq, install) {
        (Some((s, i)), Some(ds), Some(di)) if s == ds && i == di => Ok(()),
        _ => Err(conflict()),
    }
}

/// Appends a revision snapshot of the current layers + draft, makes it the head, and returns
/// the file to write.
pub(crate) fn push_revision(
    ctx: &Context,
    m: &mut Manifest,
    reason: RevisionReason,
    label: Option<String>,
) -> RevisionFile {
    let id = ctx.new_id();
    let parent = (!m.revisions.is_empty()).then_some(m.head_revision);
    let rev = RevisionFile {
        schema_version: REVISION_SCHEMA_VERSION,
        id,
        seq: m.revisions.iter().map(|r| r.seq + 1).max().unwrap_or(0),
        created_at: ctx.now(),
        reason,
        label,
        parent,
        layers: m.layers.clone(),
        draft: m.draft.clone(),
        extra: Extra::new(),
    };
    m.revisions.push(rev.entry());
    m.head_revision = id;
    rev
}

// ---------------------------------------------------------------- revisions

/// Reads a revision file.
pub fn read_revision(p: &OpenProject, id: Ulid) -> CoreResult<RevisionFile> {
    if !p.manifest.revisions.iter().any(|r| r.id == id) {
        return Err(CoreError::new(ErrorCode::UnknownId, "unknown revision").with_field("rev_id"));
    }
    read_json(&p.path(&revision_file(id)))
}

/// Ctrl+S (FR-PRJ-23): a `save` revision if the layers or draft changed since the head.
pub fn save_revision(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    label: Option<String>,
) -> CoreResult<RevisionEntry> {
    p.require_writable()?;
    // Before reading the head revision file, so the error is `folder_moved`, not OS text
    // with an absolute path (walkthrough D3).
    require_folder(p)?;
    // A draft save waiting after `locked` (FR-PRJ-21) goes first: its failure is the answer,
    // never `unchanged` (the manifest doesn't hold the waiting draft yet).
    if let Some(pending) = p.pending_draft.clone() {
        put_draft_once(ctx, install_id, p, pending.draft, pending.export)?;
        p.pending_draft = None;
    }
    let head = read_revision(p, p.manifest.head_revision)?;
    if head.layers == p.manifest.layers && head.draft == p.manifest.draft {
        return Err(CoreError::new(
            ErrorCode::Unchanged,
            "nothing changed since the last revision",
        ));
    }
    commit(ctx, install_id, p, |ctx, _, m, plan| {
        plan.revisions
            .push(push_revision(ctx, m, RevisionReason::Save, label));
        Ok(())
    })?;
    head_entry(p)
}

/// `project_restore_revision` (FR-PRJ-23, 43): a new `restore` revision with that revision's
/// layers and draft. The New image draft (`draft.generate`, FR-GEN-19) is not part of undo: it
/// keeps its current value, so undoing a pick never resets the tab (screen = disk). Candidate
/// statuses and the trash are unchanged (R3), except that generate candidates follow the
/// restored base image (R97, R84).
pub fn restore_revision(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    rev_id: Ulid,
) -> CoreResult<RevisionEntry> {
    p.require_writable()?;
    // Before reading the head revision file, so the error is `folder_moved`, not OS text
    // with an absolute path (walkthrough D3).
    require_folder(p)?;
    let rev = read_revision(p, rev_id)?;
    for layer in rev
        .layers
        .iter()
        .filter(|l| matches!(l.kind, LayerKind::Patch | LayerKind::Base))
    {
        let file = layer.file.as_deref().unwrap_or_default();
        if !p.path(file).is_file() {
            return Err(CoreError::new(
                ErrorCode::NotFound,
                format!("the layer file {file} of that revision is missing"),
            ));
        }
    }
    commit(ctx, install_id, p, |ctx, p, m, plan| {
        m.layers = rev.layers.clone();
        let generate = m.draft.as_ref().and_then(|d| d.generate.clone());
        m.draft = rev.draft.clone();
        match (m.draft.as_mut(), generate) {
            (Some(d), g) => d.generate = g,
            (None, Some(g)) => {
                let mut d = Draft::empty(ctx.now());
                d.generate = Some(g);
                m.draft = Some(d);
            }
            (None, None) => {}
        }
        store::statuses_follow_base(p, m);
        plan.revisions
            .push(push_revision(ctx, m, RevisionReason::Restore, None));
        Ok(())
    })?;
    head_entry(p)
}

fn head_entry(p: &OpenProject) -> CoreResult<RevisionEntry> {
    p.manifest
        .revisions
        .iter()
        .find(|r| r.id == p.manifest.head_revision)
        .cloned()
        .ok_or_else(|| CoreError::internal("no head revision after the commit"))
}

// ---------------------------------------------------------------- draft

/// The draft fields a caller sets; `updated_at` comes from the clock seam.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftInput {
    #[serde(default)]
    pub final_prompt: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub controls: BTreeMap<String, Value>,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub region: Option<Value>,
    /// Absent = keep the draft's mask (EDT `put_mask` owns it, R7); `null` = clear it.
    #[serde(default, deserialize_with = "present")]
    pub mask_id: Option<Option<Ulid>>,
    /// REF's draft references (#51); `null` = REF's defaults.
    #[serde(default)]
    pub references: Option<Value>,
    /// #55: the blend instruction setting; absent or `null` = on (the default).
    #[serde(default)]
    pub blend_instruction: Option<bool>,
    /// PRM's helper ticks (FR-PRM-05); absent or `null` = the defaults.
    #[serde(default)]
    pub prompt_helpers: Option<Vec<String>>,
    /// The New image tab's `draft.generate` (FR-GEN-19). Absent = keep the saved one (so a region
    /// edit draft save never touches it); `null` = clear it.
    #[serde(default, deserialize_with = "present")]
    pub generate: Option<Option<Value>>,
    /// Set by `project_put_draft` when its input has no `draft` (a New image save): the region
    /// edit fields keep their saved values (FR-GEN-19, "the region-edit draft is untouched").
    #[serde(skip)]
    pub keep_region_edit: bool,
}

impl DraftInput {
    /// A save that sets only `draft.generate` (FR-GEN-19).
    pub fn generate_only(generate: Option<Value>) -> Self {
        DraftInput {
            generate: Some(generate),
            keep_region_edit: true,
            ..Default::default()
        }
    }

    /// This save layered over an earlier one still pending after `locked` (FR-PRJ-21): the parts
    /// this save leaves alone (the region edit fields, `generate`) come from the earlier one, so
    /// a New image save and a region edit save never drop each other.
    fn over(self, older: &DraftInput) -> DraftInput {
        let mut d = self;
        if d.keep_region_edit {
            let generate = d.generate.take();
            d = DraftInput {
                generate,
                ..older.clone()
            };
        }
        if d.generate.is_none() {
            d.generate = older.generate.clone();
        }
        d
    }
}

/// [`present`] for command inputs outside this module.
pub(crate) fn present_field<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    present(d)
}

/// Tells an absent field (`None`, via `default`) from an explicit `null` (`Some(None)`).
fn present<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(d).map(Some)
}

/// `project_put_draft` (FR-PRJ-19): one commit with the draft and, if given, EXP's unsaved export
/// settings (R30: `grid`, `target_px_per_square`, `format`, merged into the `export` record).
/// A `locked` failure keeps the draft in memory, fires `project:save-failed{locked, retrying}`
/// and is retried every 10 s by [`retry_pending`] (FR-PRJ-21); a later save replaces it.
pub fn put_draft(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    draft: DraftInput,
    export_settings: Option<serde_json::Map<String, Value>>,
) -> CoreResult<u64> {
    let draft = match &p.pending_draft {
        Some(older) => draft.over(&older.draft),
        None => draft,
    };
    let keep = PendingDraft {
        draft: draft.clone(),
        export: export_settings.clone(),
    };
    let result = put_draft_once(ctx, install_id, p, draft, export_settings);
    match &result {
        Ok(_) => p.pending_draft = None,
        Err(e) if keeps_draft(e) => {
            p.pending_draft = Some(keep);
            emit_save_failed(ctx, save_failed_reason(e), true);
        }
        // Walk 2 N1: kept until the resolve, which saves it with this machine's version (kept, or
        // the `conflict` revision); closing asks meanwhile. The conflict state shows it.
        Err(e) if e.code == ErrorCode::Conflict => p.pending_draft = Some(keep),
        // Shown by the read-only state already.
        Err(e) if e.code == ErrorCode::ReadOnly => {}
        // Anything else is surfaced too: the UI's debounced draft save doesn't show its errors
        // (walkthrough D3).
        Err(e) => emit_save_failed(ctx, save_failed_reason(e), false),
    }
    result
}

/// Retries what a `locked` failure left behind (FR-PRJ-21): the pending draft. (RGN's runner
/// replays the spool on every tick, R48.) Called every [`SAVE_RETRY_EVERY`]. Returns true while something is still
/// pending. While a conflict is open the draft waits for the resolve. A draft that now fails for
/// another reason (`read_only`, …) is dropped: the UI saves its draft again on the next change.
pub fn retry_pending(ctx: &Context, install_id: Ulid, p: &mut OpenProject) -> bool {
    if p.conflict.is_some() {
        // Waits for the resolve (walk 2 N1).
        return p.pending_draft.is_some();
    }
    if let Some(pending) = p.pending_draft.take() {
        match put_draft_once(
            ctx,
            install_id,
            p,
            pending.draft.clone(),
            pending.export.clone(),
        ) {
            // A moved folder that came back: the banner goes too.
            Ok(_) => p.warnings.retain(|w| w.code != FOLDER_MOVED),
            Err(e) if keeps_draft(&e) => {
                p.pending_draft = Some(pending);
                emit_save_failed(ctx, save_failed_reason(&e), true);
                return true;
            }
            // Found by this retry: waits for the resolve too.
            Err(e) if e.code == ErrorCode::Conflict => {
                p.pending_draft = Some(pending);
                return true;
            }
            Err(e) => {
                tracing::warn!(code = %e.code, "pending draft dropped: {}", e.message);
            }
        }
    }
    false
}

/// Failures after which the draft stays in memory and is retried: a lock (FR-PRJ-21) or a
/// folder moved while open (§11; it commits if the folder comes back).
fn keeps_draft(e: &CoreError) -> bool {
    e.code == ErrorCode::Locked || is_folder_moved(e)
}

/// `project:save-failed` `reason`: the error code, or `folder_moved`.
fn save_failed_reason(e: &CoreError) -> &'static str {
    if is_folder_moved(e) {
        FOLDER_MOVED
    } else {
        e.code.as_str()
    }
}

/// True if closing would lose state that only lives in memory (FR-PRJ-21: closing asks first).
pub fn has_unsaved(p: &OpenProject) -> bool {
    p.pending_draft.is_some()
}

fn put_draft_once(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    draft: DraftInput,
    export_settings: Option<serde_json::Map<String, Value>>,
) -> CoreResult<u64> {
    if let Some(Some(id)) = draft.mask_id
        && !p.manifest.masks.iter().any(|m| m.id == id)
    {
        return Err(
            CoreError::new(ErrorCode::UnknownId, "unknown mask").with_field("draft.mask_id")
        );
    }
    commit(ctx, install_id, p, |ctx, _, m, _| {
        apply_draft(ctx, m, draft, export_settings);
        Ok(())
    })
}

/// Writes a draft (and EXP's unsaved settings) into a manifest, keeping the draft's unknown
/// fields.
pub(crate) fn apply_draft(
    ctx: &Context,
    m: &mut Manifest,
    draft: DraftInput,
    export_settings: Option<serde_json::Map<String, Value>>,
) {
    let extra = m
        .draft
        .as_ref()
        .map(|d| d.extra.clone())
        .unwrap_or_default();
    let kept_mask = m.draft.as_ref().and_then(|d| d.mask_id);
    let generate = match draft.generate {
        Some(g) => g,
        None => m.draft.as_ref().and_then(|d| d.generate.clone()),
    };
    m.draft = Some(if draft.keep_region_edit {
        let mut d = m.draft.clone().unwrap_or_else(|| Draft::empty(ctx.now()));
        d.generate = generate;
        d.updated_at = ctx.now();
        d
    } else {
        Draft {
            final_prompt: draft.final_prompt,
            model_id: draft.model_id,
            controls: draft.controls,
            seed: draft.seed,
            region: draft.region,
            mask_id: draft.mask_id.unwrap_or(kept_mask),
            references: draft.references,
            blend_instruction: draft.blend_instruction,
            prompt_helpers: draft.prompt_helpers,
            generate,
            updated_at: ctx.now(),
            extra,
        }
    });
    if let Some(settings) = export_settings {
        store::merge_export(m, settings);
    }
}

// ---------------------------------------------------------------- relink

/// `project_relink_original` (FR-PRJ-16): accepts a file only if its hash matches, copies it back
/// into `working/` and commits its size and mtime. Clears "Original missing".
pub fn relink_original(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    source: &Path,
) -> CoreResult<()> {
    if p.read_only.is_some() && p.read_only != Some(ReadOnlyReason::OriginalMissing) {
        p.require_writable()?;
    }
    let (Some(target), Some(original)) = (p.original_path(), p.manifest.original.clone()) else {
        return Err(CoreError::invalid(
            "path",
            "this project has no original (it was made by Generate new)",
        ));
    };
    if target.exists() {
        return Err(CoreError::invalid("path", "the original isn't missing"));
    }
    library::hydrate_if_needed(ctx, source)?;
    let hash = fsx::hash_file(source).map_err(|e| CoreError::io("reading the file", &e))?;
    if hash != original.content_hash {
        return Err(CoreError::new(
            ErrorCode::HashMismatch,
            "that file isn't this project's original (different content)",
        ));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CoreError::io(format!("creating {}", parent.display()), &e))?;
    }
    fsx::copy_atomic_new(ctx, source, &target)?;
    let meta = std::fs::metadata(&target).map_err(|e| CoreError::io("reading the original", &e))?;
    let was = p.read_only.take();
    let result = commit(ctx, install_id, p, |_, _, m, _| {
        if let Some(o) = m.original.as_mut() {
            o.size = meta.len();
            o.mtime = mtime_of(&meta);
        }
        Ok(())
    });
    if let Err(e) = result {
        p.read_only = was;
        return Err(e);
    }
    p.warnings.retain(|w| w.code != "original_missing");
    p.decoded = decoded_for(&p.dir, &p.manifest);
    Ok(())
}

// ---------------------------------------------------------------- images (EDT)

/// What an image id names (FR-EDT-31/32 via `resolve_image`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageKind {
    Original,
    Layer,
    Candidate,
    Mask,
}

/// A resolved image of the open project.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedImage {
    pub path: PathBuf,
    pub kind: ImageKind,
}

impl OpenProject {
    /// `resolve_image(image_id)` (R4, #48 (3)): the bottom image (the original by its stable id =
    /// the original layer's id, from creation, or a base image by its layer id, FR-GEN-33), a
    /// layer of the current stack or of any revision, a candidate (its trashed copy while it's in
    /// the trash), or a mask → its file. While a conflict is open, also either version's original
    /// or patch layers (BL-22).
    pub fn resolve_image(&self, image_id: Ulid) -> CoreResult<ResolvedImage> {
        let m = &self.manifest;
        let found = |rel: &str, kind| ResolvedImage {
            path: self.path(rel),
            kind,
        };
        if m.original_image_id() == Some(image_id)
            && let Some(file) = m.base_file()
        {
            return Ok(found(file, ImageKind::Original));
        }
        if let Some(base) = self.refs.base_images.get(&image_id) {
            // A bottom image of an earlier revision (undo across a pick, FR-GEN-29).
            let file = match base {
                Some(f) => Some(f.as_str()),
                None => m.original.as_ref().map(|o| o.file.as_str()),
            };
            if let Some(file) = file {
                return Ok(found(file, ImageKind::Original));
            }
        }
        if let Some(l) = m.layers.iter().find(|l| l.id == image_id) {
            let file = l.file.as_deref().ok_or_else(|| {
                CoreError::new(ErrorCode::Corrupt, "the layer has no file").with_field("image_id")
            })?;
            return Ok(found(file, ImageKind::Layer));
        }
        if let Some(c) = m.candidates.iter().find(|c| c.id == image_id) {
            let rel = match c.trash_id.and_then(|t| m.trash.iter().find(|i| i.id == t)) {
                Some(item) => item.trashed_path(&c.file),
                None => c.file.clone(),
            };
            return Ok(found(&rel, ImageKind::Candidate));
        }
        if let Some(mask) = m.masks.iter().find(|k| k.id == image_id) {
            return Ok(found(&mask.file, ImageKind::Mask));
        }
        if self.refs.layers.contains(&image_id) {
            return Ok(found(&layer_file(image_id), ImageKind::Layer));
        }
        if let Some((rel, original)) = conflict::side_image(self, image_id) {
            let kind = if original {
                ImageKind::Original
            } else {
                ImageKind::Layer
            };
            return Ok(found(&rel, kind));
        }
        Err(CoreError::new(ErrorCode::UnknownId, "unknown image").with_field("image_id"))
    }
}

// ---------------------------------------------------------------- project folders

/// The folders `project_folder_list` takes (FR-PRJ-47, 48).
pub const LISTED_FOLDERS: [&str; 5] = ["working", "references", "structure", "parts", "exports"];

/// One file in a project folder.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FolderFile {
    /// Relative to the folder, `/` separators.
    pub name: String,
    /// Absolute path.
    pub path: PathBuf,
    pub bytes: u64,
    pub mtime: Timestamp,
    /// A supported image by its extension (no file is opened, so nothing is hydrated).
    pub image: bool,
    /// A `pf-tiles` URL for images (EDT `library` key space), set by the command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_url: Option<String>,
    /// `cloud_only` (not hydrated) or `corrupt` (an image whose header won't probe); absent when
    /// fine. Set by the command (the sketch block's badges, FR-GEN-14).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<FileState>,
}

/// A listed file's badge state (FR-GEN-14, FR-REF-01 rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    CloudOnly,
    Corrupt,
}

/// Fills `state` for listed files: cloud-only files are never opened (FR-PRJ-17); a local image
/// is probed from its header only.
pub fn set_file_states(ctx: &Context, files: &mut [FolderFile]) {
    for f in files.iter_mut() {
        f.state = if ctx.fs.is_cloud_only(&f.path) {
            Some(FileState::CloudOnly)
        } else if f.image && pf_imaging::probe_file(&f.path).is_err() {
            Some(FileState::Corrupt)
        } else {
            None
        };
    }
}

fn check_folder(folder: &str, allowed: &[&str]) -> CoreResult<()> {
    if allowed.contains(&folder) {
        Ok(())
    } else {
        Err(CoreError::new(
            ErrorCode::UnknownFolder,
            format!("{folder:?} is not one of {}", allowed.join(", ")),
        )
        .with_field("folder"))
    }
}

/// `project_folder_list` (FR-PRJ-48): files in a project folder, recursively, by name. Reads
/// metadata only and never hydrates (FR-PRJ-17).
pub fn folder_list(p: &OpenProject, folder: &str) -> CoreResult<Vec<FolderFile>> {
    check_folder(folder, &LISTED_FOLDERS)?;
    let base = p.path(folder);
    let mut out = vec![];
    collect_files(&base, "", &mut out);
    out.sort_by_key(|f| f.name.to_lowercase());
    Ok(out)
}

fn collect_files(dir: &Path, prefix: &str, out: &mut Vec<FolderFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains(fsx::TEMP_MARKER) {
            continue;
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if meta.is_dir() {
            collect_files(&entry.path(), &rel, out);
        } else if meta.is_file() {
            out.push(FolderFile {
                image: pf_imaging::has_image_extension(&entry.path()),
                path: entry.path(),
                bytes: meta.len(),
                mtime: mtime_of(&meta),
                name: rel,
                thumb_url: None,
                state: None,
            });
        }
    }
}

/// `project_folder_add` (FR-PRJ-47): copies files into a user-owned folder under free names.
/// Never moves, renames or overwrites anything there. Returns the folder's new listing.
pub fn folder_add(
    ctx: &Context,
    p: &OpenProject,
    folder: &str,
    sources: &[PathBuf],
) -> CoreResult<Vec<FolderFile>> {
    check_folder(folder, &USER_DIRS)?;
    p.require_writable()?;
    let base = p.path(folder);
    std::fs::create_dir_all(&base)
        .map_err(|e| CoreError::io(format!("creating {}", base.display()), &e))?;
    for source in sources {
        if !source.is_file() {
            return Err(CoreError::new(
                ErrorCode::NotFound,
                format!("{} is not a file", source.display()),
            ));
        }
        library::hydrate_if_needed(ctx, source)?;
        let name = folder_name(source);
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
            _ => (name.clone(), String::new()),
        };
        let free = crate::naming::first_free(&stem, |c| {
            std::fs::symlink_metadata(base.join(format!("{c}{ext}"))).is_ok()
        });
        fsx::copy_atomic_new(ctx, source, &base.join(format!("{free}{ext}")))?;
    }
    folder_list(p, folder)
}

#[cfg(test)]
mod gen_tests;
#[cfg(test)]
mod tests;

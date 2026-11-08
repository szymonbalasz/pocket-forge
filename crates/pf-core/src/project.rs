//! The asset project store (PRJ): open/close, the commit protocol, revisions, the draft, relink,
//! project folders and image resolution. The store API other features call (jobs, candidates,
//! masks, layers, export) is in [`store`]; open-time recovery is in `sweep`.
//!
//! A project is a folder `<root>/<Maps|Tokens|Items>/<name>/` with `project.json` (#44). Every
//! mutating call commits before it returns (FR-PRJ-18): new write-once files, then
//! `project.json`, each temp → flush → rename. The manifest rename is the commit point.

use std::collections::{BTreeMap, HashSet};
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
pub mod retention;
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
}

impl RevisionRefs {
    pub(crate) fn add(&mut self, rev: &RevisionFile) {
        for l in &rev.layers {
            self.layers.insert(l.id);
            self.masks.extend(l.mask_id);
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
    /// A draft save that failed with `locked`, retried every 10 s (FR-PRJ-21).
    pub pending_draft: Option<PendingDraft>,
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
pub(crate) fn emit_save_failed(ctx: &Context, reason: ErrorCode, retrying: bool) {
    ctx.emit(
        "project:save-failed",
        serde_json::json!({ "reason": reason.as_str(), "retrying": retrying }),
    );
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
    /// Absolute path of the original (`working/<file>`).
    pub image_path: PathBuf,
    /// The original's stable image id (= `layers[0].id`, #48 (3)).
    pub original_image_id: Ulid,
    pub read_only: Option<ReadOnlyReason>,
    pub warnings: Vec<Warning>,
    pub width: u32,
    pub height: u32,
    pub format: String,
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
    pub fn snapshot(&self) -> ProjectSnapshot {
        let m = &self.manifest;
        ProjectSnapshot {
            project_id: m.project_id,
            name: folder_name(&self.dir),
            kind: m.kind,
            project_dir: self.dir.clone(),
            image_path: self.original_path(),
            original_image_id: m.original_image_id().unwrap_or(m.project_id),
            read_only: self.read_only,
            warnings: self.warnings.clone(),
            width: m.original.width,
            height: m.original.height,
            format: m.original.format.clone(),
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

    /// Absolute path of the original.
    pub fn original_path(&self) -> PathBuf {
        self.path(&self.manifest.original.file)
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
        decoded: DecodedOriginal::new(manifest.project_id, dir.join(&manifest.original.file)),
        last_commit: (!recovered).then_some((manifest.commit.seq, manifest.commit.install_id)),
        dir,
        manifest,
        read_only,
        warnings,
        jobs: BTreeMap::new(),
        refs: RevisionRefs::default(),
        thumbs,
        pending_draft: None,
    };
    check_original(ctx, &mut p)?;
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
    if library::has_conflict_copy(&p.dir) {
        p.warnings.push(Warning::new(
            "conflict",
            "This project was also changed on another machine.",
        ));
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
    {
        p.request_thumbnail(ctx);
    }
    Ok(p)
}

/// FR-PRJ-15/16/17: hydrate, then compare size and mtime, then the hash.
fn check_original(ctx: &Context, p: &mut OpenProject) -> CoreResult<()> {
    let path = p.original_path();
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
    let o = &p.manifest.original;
    if meta.len() != o.size || mtime_of(&meta) != o.mtime || racily_clean(&p.dir, o.mtime) {
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
    p.require_writable()?;
    let mut next = p.manifest.clone();
    let mut plan = CommitPlan::default();
    change(ctx, p, &mut next, &mut plan)?;
    if let Some(kind) = library::kind_of_folder(&p.dir) {
        next.kind = kind;
    }
    store::trash_superseded_masks(ctx, p, &mut next, &plan);
    check_commit_unchanged(p)?;
    if !p.dir.is_dir() {
        // Renamed or moved in Explorer while open (§11): state stays in memory.
        return Err(CoreError::new(
            ErrorCode::NotFound,
            "the project folder was moved or renamed; reopen it from home",
        ));
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
    p.manifest = next;
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
            return Err(CoreError::new(
                ErrorCode::NotFound,
                "the project folder was moved or renamed; reopen it from home",
            ));
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
/// layers and draft. Candidate statuses and the trash are unchanged (R3).
pub fn restore_revision(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    rev_id: Ulid,
) -> CoreResult<RevisionEntry> {
    p.require_writable()?;
    let rev = read_revision(p, rev_id)?;
    for layer in rev.layers.iter().filter(|l| l.kind == LayerKind::Patch) {
        let file = layer.file.as_deref().unwrap_or_default();
        if !p.path(file).is_file() {
            return Err(CoreError::new(
                ErrorCode::NotFound,
                format!("the layer file {file} of that revision is missing"),
            ));
        }
    }
    commit(ctx, install_id, p, |ctx, _, m, plan| {
        m.layers = rev.layers.clone();
        m.draft = rev.draft.clone();
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
    #[serde(default)]
    pub mask_id: Option<Ulid>,
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
    let keep = PendingDraft {
        draft: draft.clone(),
        export: export_settings.clone(),
    };
    let result = put_draft_once(ctx, install_id, p, draft, export_settings);
    match &result {
        Ok(_) => p.pending_draft = None,
        Err(e) if e.code == ErrorCode::Locked => {
            p.pending_draft = Some(keep);
            emit_save_failed(ctx, ErrorCode::Locked, true);
        }
        Err(_) => {}
    }
    result
}

/// Retries what a `locked` failure left behind (FR-PRJ-21): the pending draft, then the spool
/// (FR-PRJ-40). Called every [`SAVE_RETRY_EVERY`]. Returns true while something is still
/// pending. A draft that now fails for another reason (`conflict`, `read_only`) is dropped:
/// the UI saves its draft again on the next change.
pub fn retry_pending(ctx: &Context, install_id: Ulid, p: &mut OpenProject) -> bool {
    if let Some(pending) = p.pending_draft.take() {
        match put_draft_once(
            ctx,
            install_id,
            p,
            pending.draft.clone(),
            pending.export.clone(),
        ) {
            Ok(_) => {}
            Err(e) if e.code == ErrorCode::Locked => {
                p.pending_draft = Some(pending);
                emit_save_failed(ctx, ErrorCode::Locked, true);
                return true;
            }
            Err(e) => {
                tracing::warn!(code = %e.code, "pending draft dropped: {}", e.message);
            }
        }
    }
    false
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
    if let Some(id) = draft.mask_id
        && !p.manifest.masks.iter().any(|m| m.id == id)
    {
        return Err(
            CoreError::new(ErrorCode::UnknownId, "unknown mask").with_field("draft.mask_id")
        );
    }
    commit(ctx, install_id, p, |ctx, _, m, _| {
        let extra = m
            .draft
            .as_ref()
            .map(|d| d.extra.clone())
            .unwrap_or_default();
        m.draft = Some(Draft {
            final_prompt: draft.final_prompt,
            model_id: draft.model_id,
            controls: draft.controls,
            seed: draft.seed,
            region: draft.region,
            mask_id: draft.mask_id,
            updated_at: ctx.now(),
            extra,
        });
        if let Some(settings) = export_settings {
            store::merge_export(m, settings);
        }
        Ok(())
    })
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
    let target = p.original_path();
    if target.exists() {
        return Err(CoreError::invalid("path", "the original isn't missing"));
    }
    library::hydrate_if_needed(ctx, source)?;
    let hash = fsx::hash_file(source).map_err(|e| CoreError::io("reading the file", &e))?;
    if hash != p.manifest.original.content_hash {
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
        m.original.size = meta.len();
        m.original.mtime = mtime_of(&meta);
        Ok(())
    });
    if let Err(e) = result {
        p.read_only = was;
        return Err(e);
    }
    p.warnings.retain(|w| w.code != "original_missing");
    p.decoded = DecodedOriginal::new(p.manifest.project_id, target);
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
    /// `resolve_image(image_id)` (R4, #48 (3)): the original (its stable id = the original
    /// layer's id, from creation), a layer of the current stack or of any revision, a candidate
    /// (its trashed copy while it's in the trash), or a mask → its file.
    pub fn resolve_image(&self, image_id: Ulid) -> CoreResult<ResolvedImage> {
        let m = &self.manifest;
        let found = |rel: &str, kind| ResolvedImage {
            path: self.path(rel),
            kind,
        };
        if m.original_image_id() == Some(image_id) {
            return Ok(found(&m.original.file, ImageKind::Original));
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
mod tests;

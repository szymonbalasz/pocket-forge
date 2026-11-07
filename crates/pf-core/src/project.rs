//! The asset project store: open, the commit protocol, revisions and the draft.
//! Skeleton scope (PRJ slices 1 and 3, minimum). Jobs, candidates, masks, trash, conflicts,
//! rebuild and the decoded original are later PRJ slices.

use std::path::{Path, PathBuf};

use serde::Serialize;
use ulid::Ulid;

use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::library;
use crate::manifest::*;
use crate::settings::Settings;
use crate::time::Timestamp;

/// Why a project is read-only (FR-PRJ-15/16/26, FR-PRJ-41).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadOnlyReason {
    /// "Made by a newer Pocket Forge".
    NewerVersion,
    /// "Original changed".
    OriginalChanged,
    /// "Original missing" (relink is PRJ slice 3).
    OriginalMissing,
}

/// A non-fatal note from `project_open`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Warning {
    pub code: &'static str,
    pub message: String,
}

/// An asset project held in memory while it's open.
#[derive(Debug)]
pub struct OpenProject {
    /// Absolute path of the original.
    pub image_path: PathBuf,
    /// Absolute path of `<file>.pforge/`.
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// False until the first commit creates the folder (FR-PRJ-13).
    pub on_disk: bool,
    /// The `commit{seq, install_id}` last loaded or written (FR-PRJ-35).
    pub last_commit: Option<(u64, Ulid)>,
    pub read_only: Option<ReadOnlyReason>,
    pub warnings: Vec<Warning>,
}

/// What `project_open` returns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectSnapshot {
    pub project_id: Ulid,
    /// File name of the original.
    pub name: String,
    pub image_path: PathBuf,
    pub project_dir: PathBuf,
    /// True once the project exists on disk.
    pub saved: bool,
    pub read_only: Option<ReadOnlyReason>,
    pub warnings: Vec<Warning>,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub layers: Vec<Layer>,
    pub draft: Option<Draft>,
    pub revisions: Vec<RevisionEntry>,
    /// `None` until the first commit.
    pub head_revision: Option<Ulid>,
    pub commit_seq: u64,
}

impl OpenProject {
    pub fn snapshot(&self) -> ProjectSnapshot {
        let m = &self.manifest;
        ProjectSnapshot {
            project_id: m.project_id,
            name: file_name(&self.image_path),
            image_path: self.image_path.clone(),
            project_dir: self.dir.clone(),
            saved: self.on_disk,
            read_only: self.read_only,
            warnings: self.warnings.clone(),
            width: m.original.width,
            height: m.original.height,
            format: m.original.format.clone(),
            layers: m.layers.clone(),
            draft: m.draft.clone(),
            revisions: m.revisions.clone(),
            head_revision: self.on_disk.then_some(m.head_revision),
            commit_seq: self.last_commit.map_or(0, |c| c.0),
        }
    }

    fn manifest_path(&self) -> PathBuf {
        self.dir.join(MANIFEST_FILE)
    }

    fn revision_path(&self, id: Ulid) -> PathBuf {
        self.dir.join(REVISIONS_DIR).join(format!("{id}.json"))
    }

    fn require_writable(&self) -> CoreResult<()> {
        match self.read_only {
            Some(reason) => Err(CoreError::new(
                ErrorCode::ReadOnly,
                "this asset project is read-only",
            )
            .with_detail(serde_json::json!({ "reason": reason }))),
            None => Ok(()),
        }
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `<file>.pforge` next to the image.
pub fn project_dir_for(image: &Path) -> PathBuf {
    let mut name = image.as_os_str().to_owned();
    name.push(PROJECT_SUFFIX);
    PathBuf::from(name)
}

fn mtime_of(meta: &std::fs::Metadata) -> Timestamp {
    meta.modified()
        .map(Timestamp::from_system)
        .unwrap_or(Timestamp(0))
}

/// Opens an image or a `.pforge` folder (FR-PRJ-13/14/15/26). Writes nothing in the project.
pub fn open(ctx: &Context, settings: &Settings, path: &Path) -> CoreResult<OpenProject> {
    let path = std::path::absolute(path)
        .map(|p| library::normalise(&p))
        .map_err(|e| CoreError::io("resolving the path", &e))?;
    let is_project_dir = path.is_dir()
        && path
            .to_string_lossy()
            .to_lowercase()
            .ends_with(PROJECT_SUFFIX);
    let (image_hint, dir) = if is_project_dir {
        let s = path.to_string_lossy();
        (
            PathBuf::from(&s[..s.len() - PROJECT_SUFFIX.len()]),
            path.clone(),
        )
    } else {
        (path.clone(), project_dir_for(&path))
    };
    if library::root_of(ctx, settings, &dir).is_none() {
        return Err(CoreError::new(
            ErrorCode::OutsideLibrary,
            "the image is outside the library; import it first",
        ));
    }
    let manifest_path = dir.join(MANIFEST_FILE);
    if manifest_path.is_file() {
        return open_existing(&dir, &manifest_path, &image_hint);
    }
    let mut warnings = vec![];
    if dir.is_dir() {
        // A crash during the first commit can leave the folder without a manifest.
        fsx::remove_temp_files(&dir);
        if dir.join(REVISIONS_DIR).is_dir() {
            warnings.push(Warning {
                code: "manifest_missing",
                message: "The project manifest is missing; starting from the original. Older revision files are kept.".into(),
            });
        }
    }
    let mut p = open_new(ctx, &image_hint, dir)?;
    p.warnings.extend(warnings);
    Ok(p)
}

fn open_new(ctx: &Context, image: &Path, dir: PathBuf) -> CoreResult<OpenProject> {
    if !image.is_file() {
        return Err(CoreError::new(
            ErrorCode::NotFound,
            format!("{} not found", image.display()),
        ));
    }
    if ctx.fs.is_cloud_only(image) {
        ctx.fs.hydrate(image).map_err(|e| {
            CoreError::new(
                ErrorCode::NotAvailableOffline,
                format!("the image is only in the cloud and can't be downloaded: {e}"),
            )
        })?;
    }
    let info = pf_imaging::probe_file(image).map_err(probe_error)?;
    let meta = std::fs::metadata(image).map_err(|e| CoreError::io("reading the image", &e))?;
    let now = ctx.now();
    let manifest = Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        project_id: ctx.new_id(),
        created_at: now,
        commit: Commit {
            seq: 0,
            install_id: Ulid::nil(),
            at: now,
        },
        original: Original {
            file: format!("../{}", file_name(image)),
            // Hashed at the first commit, so opening stays fast and writes nothing.
            content_hash: String::new(),
            size: meta.len(),
            mtime: mtime_of(&meta),
            width: info.width,
            height: info.height,
            format: info.format.as_str().into(),
            extra: Extra::new(),
        },
        layers: vec![Layer::original(ctx.new_id(), info.width, info.height)],
        masks: vec![],
        draft: None,
        jobs: vec![],
        candidates: vec![],
        revisions: vec![],
        head_revision: ctx.new_id(),
        trash: vec![],
        export: None,
        extra: Extra::new(),
    };
    Ok(OpenProject {
        image_path: image.to_path_buf(),
        dir,
        manifest,
        on_disk: false,
        last_commit: None,
        read_only: None,
        warnings: vec![],
    })
}

fn probe_error(e: pf_imaging::ProbeError) -> CoreError {
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

fn open_existing(dir: &Path, manifest_path: &Path, image_hint: &Path) -> CoreResult<OpenProject> {
    // Leftovers of an interrupted commit (FR-PRJ-20).
    fsx::remove_temp_files(dir);
    let bytes =
        std::fs::read(manifest_path).map_err(|e| CoreError::io("reading manifest.json", &e))?;
    let version = peek_schema_version(&bytes).ok_or_else(|| {
        // Rebuild from revisions (FR-PRJ-24) is a later PRJ slice.
        CoreError::new(ErrorCode::Corrupt, "manifest.json is unreadable")
    })?;
    let mut read_only = None;
    let mut warnings = vec![];
    if version > u64::from(MANIFEST_SCHEMA_VERSION) {
        read_only = Some(ReadOnlyReason::NewerVersion);
        warnings.push(Warning {
            code: "newer_version",
            message: "Made by a newer Pocket Forge. The project opens read-only.".into(),
        });
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| {
        CoreError::new(
            ErrorCode::Corrupt,
            format!("manifest.json is unreadable: {e}"),
        )
    })?;
    let image_path = library::normalise(&dir.join(&manifest.original.file));
    if image_path != *image_hint && !image_hint.as_os_str().is_empty() {
        tracing::debug!(manifest = %image_path.display(), hint = %image_hint.display(), "original path from manifest");
    }
    match std::fs::metadata(&image_path) {
        Err(_) => {
            read_only.get_or_insert(ReadOnlyReason::OriginalMissing);
            warnings.push(Warning {
                code: "original_missing",
                message: "Original missing. The project opens read-only.".into(),
            });
        }
        Ok(meta) => {
            let o = &manifest.original;
            if meta.len() != o.size || mtime_of(&meta) != o.mtime {
                let hash = fsx::hash_file(&image_path)
                    .map_err(|e| CoreError::io("hashing the original", &e))?;
                if hash != o.content_hash {
                    read_only.get_or_insert(ReadOnlyReason::OriginalChanged);
                    warnings.push(Warning {
                        code: "original_changed",
                        message:
                            "Original changed outside Pocket Forge. The project opens read-only."
                                .into(),
                    });
                }
            }
        }
    }
    Ok(OpenProject {
        image_path,
        dir: dir.to_path_buf(),
        last_commit: Some((manifest.commit.seq, manifest.commit.install_id)),
        manifest,
        on_disk: true,
        read_only,
        warnings,
    })
}

/// One commit (FR-PRJ-18): new write-once files first, then the manifest; the manifest rename
/// is the commit point. `change` edits a copy of the manifest and returns the new revision
/// files to write; on any error the in-memory project is unchanged.
pub fn commit(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    change: impl FnOnce(&Context, &mut Manifest) -> CoreResult<Vec<RevisionFile>>,
) -> CoreResult<u64> {
    p.require_writable()?;
    let mut next = p.manifest.clone();
    let mut revisions = vec![];
    if !p.on_disk {
        // First commit: hash the original, create the folder and r0 (FR-PRJ-13).
        let meta = std::fs::metadata(&p.image_path)
            .map_err(|e| CoreError::io("reading the original", &e))?;
        next.original.content_hash =
            fsx::hash_file(&p.image_path).map_err(|e| CoreError::io("hashing the original", &e))?;
        next.original.size = meta.len();
        next.original.mtime = mtime_of(&meta);
        next.created_at = ctx.now();
    }
    revisions.extend(change(ctx, &mut next)?);
    if !p.on_disk && next.revisions.is_empty() {
        let id = next.head_revision;
        revisions.push(push_revision(
            ctx,
            &mut next,
            id,
            RevisionReason::Create,
            None,
        ));
    }

    if p.on_disk {
        check_commit_unchanged(p)?;
    } else {
        std::fs::create_dir_all(p.dir.join(REVISIONS_DIR))
            .map_err(|e| CoreError::io(format!("creating {}", p.dir.display()), &e))?;
    }
    for rev in &revisions {
        fsx::write_json_atomic(ctx, &p.revision_path(rev.id), rev)?;
    }
    let seq = p.last_commit.map_or(0, |c| c.0) + 1;
    next.commit = Commit {
        seq,
        install_id,
        at: ctx.now(),
    };
    fsx::write_json_atomic(ctx, &p.manifest_path(), &next)?;

    p.manifest = next;
    p.on_disk = true;
    p.last_commit = Some((seq, install_id));
    ctx.emit("project:committed", serde_json::json!({ "seq": seq }));
    Ok(seq)
}

/// FR-PRJ-35: the on-disk commit must still be the one last loaded or written.
fn check_commit_unchanged(p: &OpenProject) -> CoreResult<()> {
    let conflict = || CoreError::new(ErrorCode::Conflict, "manifest.json changed on disk");
    let bytes = std::fs::read(p.manifest_path()).map_err(|_| conflict())?;
    let disk: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| conflict())?;
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
fn push_revision(
    ctx: &Context,
    m: &mut Manifest,
    id: Ulid,
    reason: RevisionReason,
    label: Option<String>,
) -> RevisionFile {
    let parent = (!m.revisions.is_empty()).then_some(m.head_revision);
    let rev = RevisionFile {
        schema_version: REVISION_SCHEMA_VERSION,
        id,
        seq: m.revisions.len() as u64,
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

/// Reads a revision file.
pub fn read_revision(p: &OpenProject, id: Ulid) -> CoreResult<RevisionFile> {
    let bytes = std::fs::read(p.revision_path(id))
        .map_err(|e| CoreError::io(format!("reading revision {id}"), &e))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| CoreError::new(ErrorCode::Corrupt, format!("revision {id}: {e}")))
}

/// Ctrl+S (FR-PRJ-23): a `save` revision if the layers or draft changed since the head. The
/// first save of a new project creates it with `r0` instead.
pub fn save_revision(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    label: Option<String>,
) -> CoreResult<RevisionEntry> {
    p.require_writable()?;
    if !p.on_disk {
        commit(ctx, install_id, p, |ctx, m| {
            let id = m.head_revision;
            Ok(vec![push_revision(
                ctx,
                m,
                id,
                RevisionReason::Create,
                label,
            )])
        })?;
    } else {
        let head = read_revision(p, p.manifest.head_revision)?;
        if head.layers == p.manifest.layers && head.draft == p.manifest.draft {
            return Err(CoreError::new(
                ErrorCode::Unchanged,
                "nothing changed since the last revision",
            ));
        }
        commit(ctx, install_id, p, |ctx, m| {
            let id = ctx.new_id();
            Ok(vec![push_revision(ctx, m, id, RevisionReason::Save, label)])
        })?;
    }
    p.manifest
        .revisions
        .last()
        .cloned()
        .ok_or_else(|| CoreError::internal("no revision after save"))
}

/// The draft fields a caller sets; `updated_at` comes from the clock seam.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftInput {
    #[serde(default)]
    pub final_prompt: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub controls: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub region: Option<serde_json::Value>,
    #[serde(default)]
    pub mask_id: Option<Ulid>,
}

/// `project_put_draft` (FR-PRJ-19): one commit. Creates the project on first use.
pub fn put_draft(
    ctx: &Context,
    install_id: Ulid,
    p: &mut OpenProject,
    draft: DraftInput,
) -> CoreResult<u64> {
    commit(ctx, install_id, p, |ctx, m| {
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
        Ok(vec![])
    })
}

#[cfg(test)]
mod tests;

//! Core commands. Each is a unit struct implementing [`Command`]; inputs are validated at the
//! boundary. **Append-only registry** at the bottom: add new commands, never rename one.
//!
//! Every command takes one `input` object and returns JSON. The Tauri layer
//! (`app/src-tauri/src/commands.rs`) adapts each one; the UI calls them through `app/src/api/`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use std::sync::atomic::Ordering;

use crate::core::{Command, Core, Validate, dispatch_json};
use crate::create::{self, CreateItem};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::library::{self, LibraryConfig, ProjectCard};
use crate::manifest::{Kind, RevisionEntry};
use crate::naming;
use crate::project::{self, DraftInput, FolderFile, ProjectSnapshot};
use crate::settings::{self, SettingsWarning};
use crate::tiles;

const MAX_PATH_CHARS: usize = 4096;
const MAX_PATHS: usize = 10_000;
const MAX_LABEL_CHARS: usize = 200;
const MAX_PROMPT_CHARS: usize = 20_000;

fn check_path(field: &str, p: &str) -> CoreResult<()> {
    if p.trim().is_empty() {
        return Err(CoreError::invalid(field, "the path is empty"));
    }
    if p.chars().count() > MAX_PATH_CHARS || p.contains('\0') {
        return Err(CoreError::invalid(field, "the path is not valid"));
    }
    Ok(())
}

/// Commands that take no input accept `{}` (or `null`).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoInput {}

impl Validate for NoInput {}

/// Empty success result (`{}`).
#[derive(Debug, Default, Serialize)]
pub struct Ok_ {}

pub(crate) fn no_project() -> CoreError {
    CoreError::new(ErrorCode::NoProject, "no asset project is open")
}

// ---------------------------------------------------------------- app

pub struct AppInfo;

#[derive(Debug, Serialize)]
pub struct AppInfoOutput {
    pub version: &'static str,
    pub provider_mode: pf_provider::ProviderMode,
    pub data_dir: PathBuf,
    pub library_overridden: bool,
    /// Startup warnings (e.g. settings were reset), shown once.
    pub warnings: Vec<SettingsWarning>,
}

impl Command for AppInfo {
    const NAME: &'static str = "app_info";
    type Input = NoInput;
    type Output = AppInfoOutput;

    fn run(core: &Core, _: NoInput) -> CoreResult<AppInfoOutput> {
        Ok(AppInfoOutput {
            version: env!("CARGO_PKG_VERSION"),
            provider_mode: core.ctx.provider.mode(),
            data_dir: core.ctx.data_dir.clone(),
            library_overridden: core.ctx.library_override.is_some(),
            warnings: core.state().settings_warnings.clone(),
        })
    }
}

// ---------------------------------------------------------------- library

pub struct LibraryGetConfig;

impl Command for LibraryGetConfig {
    const NAME: &'static str = "library_get_config";
    type Input = NoInput;
    type Output = LibraryConfig;

    fn run(core: &Core, _: NoInput) -> CoreResult<LibraryConfig> {
        Ok(library::config(&core.ctx, &core.state().settings))
    }
}

pub struct LibrarySetRoots;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibrarySetRootsInput {
    pub roots: Vec<String>,
}

impl Validate for LibrarySetRootsInput {
    fn validate(&self) -> CoreResult<()> {
        if self.roots.len() > 64 {
            return Err(CoreError::invalid("roots", "too many folders"));
        }
        for (i, r) in self.roots.iter().enumerate() {
            check_path(&format!("roots[{i}]"), r)?;
        }
        Ok(())
    }
}

impl Command for LibrarySetRoots {
    const NAME: &'static str = "library_set_roots";
    type Input = LibrarySetRootsInput;
    type Output = LibraryConfig;

    /// FR-PRJ-01: validates, creates any missing section folders, then saves.
    fn run(core: &Core, input: Self::Input) -> CoreResult<LibraryConfig> {
        let roots: Vec<PathBuf> = input.roots.iter().map(PathBuf::from).collect();
        library::validate_roots(&core.ctx, &roots)?;
        for (i, root) in roots.iter().enumerate() {
            library::ensure_sections(root).map_err(|e| e.with_field(format!("roots[{i}]")))?;
        }
        let mut state = core.state();
        let mut next = state.settings.clone();
        next.library.roots = roots;
        settings::save(&core.ctx, &next)?;
        state.settings = next;
        Ok(library::config(&core.ctx, &state.settings))
    }
}

/// A section by its folder (`maps|tokens|items`, the UI's spelling) or its kind
/// (`map|token|item`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionInput {
    #[serde(alias = "map")]
    Maps,
    #[serde(alias = "token")]
    Tokens,
    #[serde(alias = "item")]
    Items,
}

impl From<SectionInput> for Kind {
    fn from(s: SectionInput) -> Kind {
        match s {
            SectionInput::Maps => Kind::Map,
            SectionInput::Tokens => Kind::Token,
            SectionInput::Items => Kind::Item,
        }
    }
}

pub struct LibraryListProjects;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionOnly {
    pub section: SectionInput,
}

impl Validate for SectionOnly {}

impl Command for LibraryListProjects {
    const NAME: &'static str = "library_list_projects";
    type Input = SectionOnly;
    type Output = Vec<ProjectCard>;

    /// FR-PRJ-49. Doesn't hold the state lock while it reads the folders. Thumbnails are served
    /// through EDT's `library` key space (OQ-PRJ-16).
    fn run(core: &Core, input: SectionOnly) -> CoreResult<Vec<ProjectCard>> {
        let settings = core.state().settings.clone();
        let mut cards = library::list_projects(&core.ctx, &settings, input.section.into())?;
        for card in &mut cards {
            card.thumb_url = card
                .thumb
                .as_deref()
                .and_then(|t| tiles::register_library_image(core, t));
        }
        Ok(cards)
    }
}

// ---------------------------------------------------------------- creation

pub struct ProjectCreate;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectCreateInput {
    pub section: SectionInput,
    pub paths: Vec<String>,
}

impl Validate for ProjectCreateInput {
    fn validate(&self) -> CoreResult<()> {
        if self.paths.is_empty() || self.paths.len() > MAX_PATHS {
            return Err(CoreError::invalid(
                "paths",
                format!("give 1 to {MAX_PATHS} paths"),
            ));
        }
        for (i, p) in self.paths.iter().enumerate() {
            check_path(&format!("paths[{i}]"), p)?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct ProjectCreateOutput {
    pub items: Vec<CreateItem>,
}

impl Command for ProjectCreate {
    const NAME: &'static str = "project_create";
    type Input = ProjectCreateInput;
    type Output = ProjectCreateOutput;

    /// FR-PRJ-03 … 11. Runs without the state lock, so `project_create_cancel` and other
    /// commands answer meanwhile. Cancel → `canceled` with `detail.items`.
    fn run(core: &Core, input: Self::Input) -> CoreResult<ProjectCreateOutput> {
        let _one_at_a_time = core.create_lock.lock().unwrap_or_else(|e| e.into_inner());
        core.create_cancel.store(false, Ordering::Relaxed);
        let settings = core.state().settings.clone();
        let paths: Vec<PathBuf> = input.paths.iter().map(PathBuf::from).collect();
        let items = create::create_projects(
            &core.ctx,
            &settings,
            input.section.into(),
            &paths,
            &core.create_cancel,
            Some(&core.thumbs),
        )?;
        Ok(ProjectCreateOutput { items })
    }
}

pub struct ProjectCreateCancel;

impl Command for ProjectCreateCancel {
    const NAME: &'static str = "project_create_cancel";
    type Input = NoInput;
    type Output = Ok_;

    fn run(core: &Core, _: NoInput) -> CoreResult<Ok_> {
        core.create_cancel.store(true, Ordering::Relaxed);
        Ok(Ok_ {})
    }
}

pub struct ProjectCreateFromClipboard;

/// Largest pasted image accepted, in base64 characters (about 512 MiB of bytes).
const MAX_CLIPBOARD_B64: usize = 700 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectCreateFromClipboardInput {
    pub section: SectionInput,
    /// Encoded PNG, JPEG or WebP bytes, standard base64.
    pub data_b64: String,
}

impl std::fmt::Debug for ProjectCreateFromClipboardInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectCreateFromClipboardInput")
            .field("section", &self.section)
            .field("data_b64", &self.data_b64.len())
            .finish()
    }
}

impl Validate for ProjectCreateFromClipboardInput {
    fn validate(&self) -> CoreResult<()> {
        if self.data_b64.trim().is_empty() {
            return Err(CoreError::invalid("data_b64", "no image data"));
        }
        if self.data_b64.len() > MAX_CLIPBOARD_B64 {
            return Err(
                CoreError::new(ErrorCode::TooLarge, "the pasted image is too large")
                    .with_field("data_b64"),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct PathOutput {
    pub path: PathBuf,
}

impl Command for ProjectCreateFromClipboard {
    const NAME: &'static str = "project_create_from_clipboard";
    type Input = ProjectCreateFromClipboardInput;
    type Output = PathOutput;

    /// FR-PRJ-10.
    fn run(core: &Core, input: Self::Input) -> CoreResult<PathOutput> {
        let bytes = crate::b64::decode(&input.data_b64)
            .ok_or_else(|| CoreError::invalid("data_b64", "not valid base64"))?;
        let _one_at_a_time = core.create_lock.lock().unwrap_or_else(|e| e.into_inner());
        let settings = core.state().settings.clone();
        let mut creator = create::Creator::new(
            &core.ctx,
            &settings,
            input.section.into(),
            Some(&core.thumbs),
        )?;
        let path = creator.create_from_bytes(&bytes)?;
        Ok(PathOutput { path })
    }
}

pub struct ProjectRename;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRenameInput {
    pub path: String,
    pub name: String,
}

impl Validate for ProjectRenameInput {
    fn validate(&self) -> CoreResult<()> {
        check_path("path", &self.path)?;
        if self.name.chars().count() > 4 * naming::MAX_NAME_CHARS {
            return Err(
                CoreError::new(ErrorCode::InvalidName, "the name is too long").with_field("name"),
            );
        }
        Ok(())
    }
}

impl Command for ProjectRename {
    const NAME: &'static str = "project_rename";
    type Input = ProjectRenameInput;
    type Output = PathOutput;

    /// FR-PRJ-45 (a closed project) and FR-PRJ-56 (the open one).
    fn run(core: &Core, input: Self::Input) -> CoreResult<PathOutput> {
        let mut state = core.state();
        let path = PathBuf::from(&input.path);
        let path = if is_open_project(&state, &path) {
            rename_open(core, &mut state, &input.name)?
        } else {
            // R111: a job of the closed project would come back for a folder that is gone.
            library::rename_project_if(&core.ctx, &state.settings, &path, &input.name, |dir| {
                refuse_active_job_closed(core, &state, dir, "Rename")
            })?
        };
        Ok(PathOutput { path })
    }
}

/// True if `path` names the open project's folder. Lexical, so it still answers once the folder
/// was moved (§11).
fn is_open_project(state: &crate::core::State, path: &std::path::Path) -> bool {
    let Ok(path) = std::path::absolute(path) else {
        return false;
    };
    state
        .project
        .as_ref()
        .is_some_and(|p| library::same_folder(&p.dir, &path))
}

/// R97 for FR-PRJ-56, 57: `job_active` while the open project has an active job. The job would
/// come back for a folder that is gone.
fn refuse_active_job_open(state: &crate::core::State, verb: &str) -> CoreResult<()> {
    let Some(p) = state.project.as_ref() else {
        return Ok(());
    };
    let active = p
        .jobs
        .values()
        .find(|j| crate::region_edit::jobs::is_active(j.state))
        .map(|j| j.id)
        .or_else(|| state.rgn.runner.job_in(&p.dir));
    match active {
        Some(id) => Err(job_active(id, verb)),
        None => Ok(()),
    }
}

/// R97 for a closed project (FR-PRJ-45, 57): `job_active` while the runner still drives a job
/// of the project in `dir`, or, after a restart, one of its job files says it is active.
fn refuse_active_job_closed(
    core: &Core,
    state: &crate::core::State,
    dir: &std::path::Path,
    verb: &str,
) -> CoreResult<()> {
    let active = state
        .rgn
        .runner
        .job_in(dir)
        .or_else(|| library::trash::active_job_on_disk(&core.ctx, dir));
    match active {
        Some(id) => Err(job_active(id, verb)),
        None => Ok(()),
    }
}

fn job_active(job_id: Ulid, verb: &str) -> CoreError {
    CoreError::new(
        ErrorCode::JobActive,
        format!("A job is running in this project. {verb} it when the job has finished."),
    )
    .with_detail(serde_json::json!({ "job_id": job_id }))
}

/// Releases the open project's files as a close does (EXP's pending settings and running
/// export, the chat session, the decoded original). The caller decided what happens to a
/// waiting save. `reopens`: the folder is only renamed or moved and the same project opens
/// again (FR-PRJ-56, 57), so EXP's session and the published chat contexts stay. Returns the
/// project and whether its original was being decoded or in memory.
fn release_open(
    core: &Core,
    state: &mut crate::core::State,
    reopens: bool,
) -> Option<(project::OpenProject, bool)> {
    if reopens {
        crate::export::on_project_moving(&core.ctx, state);
    } else {
        crate::export::on_project_close(&core.ctx, state);
    }
    let p = state.project.take()?;
    if reopens {
        core.chat.on_project_moving(p.manifest.project_id);
    } else {
        core.chat.on_project_close(p.manifest.project_id);
    }
    let decoded = p.decoded.is_started();
    p.decoded.release();
    Some((p, decoded))
}

/// Opens `dir` again after [`release_open`] (FR-PRJ-56): the same project, so the served keys
/// stay. `decode` starts the original's decode again, as the UI asked for it once per project
/// already. `was` is the project as it was released: the open adds no job warning it didn't
/// show (a fetched job reads as `resumable` at an open; the UI drops that when it loads the
/// project, which it doesn't do here).
fn reopen(
    core: &Core,
    state: &mut crate::core::State,
    dir: &std::path::Path,
    decode: bool,
    was: &project::OpenProject,
) -> CoreResult<()> {
    let settings = state.settings.clone();
    let opened = project::open(&core.ctx, &settings, dir, Some(core.thumbs.clone()));
    let mut p = match opened {
        Ok(p) => p,
        Err(e) => {
            // Nothing is open any more: the UI reads `project_get` and shows home.
            core.served().reset(None);
            core.chat.notify_project(&core.ctx, None);
            return Err(e);
        }
    };
    crate::region_edit::runner::on_project_open(
        &core.ctx,
        settings.install_id,
        &mut state.rgn,
        &mut p,
    );
    let shown = was.live_warnings();
    p.warnings.retain(|w| {
        !matches!(w.code, "resumable" | "lost") || shown.iter().any(|s| s.code == w.code)
    });
    if decode {
        p.decoded.start(&core.ctx);
    }
    core.chat
        .notify_project(&core.ctx, Some(p.manifest.project_id));
    state.project = Some(p);
    Ok(())
}

/// FR-PRJ-56: renames the open project. Checks, then what waits is saved as a close does (no
/// force), the files are released, the folder is renamed and the project is opened again. A
/// folder rename that fails opens it again under its old name.
fn rename_open(core: &Core, state: &mut crate::core::State, name: &str) -> CoreResult<PathBuf> {
    let p = state.project.as_mut().ok_or_else(no_project)?;
    project::require_folder(p)?;
    let old = p.dir.clone();
    let Some(target) = library::rename_target(&old, name)? else {
        return Ok(old);
    };
    refuse_active_job_open(state, "Rename")?;
    refuse_unsaved(core, state, Leaving::Rename)?;
    let (was, decode) = release_open(core, state, true).ok_or_else(no_project)?;
    match crate::fsx::move_dir(&core.ctx, &old, &target) {
        Ok(()) => {
            core.served().rebase(&old, &target);
            reopen(core, state, &target, decode, &was)?;
            Ok(target)
        }
        Err(e) => {
            if let Err(again) = reopen(core, state, &old, decode, &was) {
                tracing::warn!(error = %again, "the project didn't open again after a failed rename");
            }
            Err(e)
        }
    }
}

// ---------------------------------------------------------------- delete, library trash (#62)

pub struct ProjectDelete;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectDeleteInput {
    /// The project folder.
    pub path: String,
}

impl Validate for ProjectDeleteInput {
    fn validate(&self) -> CoreResult<()> {
        check_path("path", &self.path)
    }
}

#[derive(Debug, Serialize)]
pub struct ItemIdOutput {
    /// The library trash item (`library_trash_restore` takes it).
    pub item_id: Ulid,
}

impl Command for ProjectDelete {
    const NAME: &'static str = "project_delete";
    type Input = ProjectDeleteInput;
    type Output = ItemIdOutput;

    /// FR-PRJ-57: the folder moves, whole, to the library trash. The open project is closed
    /// first; `job_active` while it has an active job (R97), open or closed.
    fn run(core: &Core, input: Self::Input) -> CoreResult<ItemIdOutput> {
        let mut state = core.state();
        let settings = state.settings.clone();
        let loc = library::trash::locate(&core.ctx, &settings, &PathBuf::from(&input.path))?;
        let item_id = if is_open_project(&state, &loc.dir) {
            delete_open(core, &mut state, &loc)?
        } else {
            refuse_active_job_closed(core, &state, &loc.dir, "Delete")?;
            library::trash::delete_project(&core.ctx, &loc)?
        };
        Ok(ItemIdOutput { item_id })
    }
}

/// FR-PRJ-57 on the open project: released as FR-PRJ-56 does; a waiting save is tried once and
/// dropped only once the move has succeeded (the UI asked). A move that fails leaves the
/// project open as it was, with its waiting changes still in memory (R111): it is put back,
/// never read again from the folder.
fn delete_open(
    core: &Core,
    state: &mut crate::core::State,
    loc: &library::trash::ProjectLoc,
) -> CoreResult<Ulid> {
    refuse_active_job_open(state, "Delete")?;
    let install_id = state.settings.install_id;
    if let Some(p) = state.project.as_mut() {
        project::retry_pending(&core.ctx, install_id, p);
    }
    crate::export::unsaved_on_close(&core.ctx, state);
    let (p, decode) = release_open(core, state, true).ok_or_else(no_project)?;
    match library::trash::delete_project(&core.ctx, loc) {
        Ok(id) => {
            // It is gone for good: the rest of a close.
            crate::export::on_project_close(&core.ctx, state);
            core.chat.on_project_close(p.manifest.project_id);
            core.served().reset(None);
            core.chat.notify_project(&core.ctx, None);
            Ok(id)
        }
        Err(e) => {
            if decode {
                p.decoded.start(&core.ctx);
            }
            core.chat
                .notify_project(&core.ctx, Some(p.manifest.project_id));
            state.project = Some(p);
            Err(e)
        }
    }
}

pub struct LibraryTrashList;

impl Command for LibraryTrashList {
    const NAME: &'static str = "library_trash_list";
    type Input = SectionOnly;
    type Output = Vec<library::trash::TrashEntry>;

    /// FR-PRJ-58: Recently deleted of one section, newest first.
    fn run(core: &Core, input: SectionOnly) -> CoreResult<Self::Output> {
        let settings = core.state().settings.clone();
        library::trash::list(&core.ctx, &settings, input.section.into())
    }
}

pub struct LibraryTrashRestore;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryTrashRestoreInput {
    /// The library trash item.
    pub id: Ulid,
}

impl Validate for LibraryTrashRestoreInput {}

impl Command for LibraryTrashRestore {
    const NAME: &'static str = "library_trash_restore";
    type Input = LibraryTrashRestoreInput;
    type Output = PathOutput;

    /// FR-PRJ-58: back to its section in the same root; the restored project folder.
    fn run(core: &Core, input: Self::Input) -> CoreResult<PathOutput> {
        // Like a creation: one at a time, so two never race for a name, and without the state
        // lock (the same lock order as `project_create`).
        let _one_at_a_time = core.create_lock.lock().unwrap_or_else(|e| e.into_inner());
        let settings = core.state().settings.clone();
        let path = library::trash::restore(&core.ctx, &settings, input.id)?;
        Ok(PathOutput { path })
    }
}

// ---------------------------------------------------------------- Generate new (GEN)

pub struct GenProjectCreate;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenProjectCreateInput {
    pub section: SectionInput,
    pub name: String,
}

impl Validate for GenProjectCreateInput {
    fn validate(&self) -> CoreResult<()> {
        if self.name.chars().count() > 4 * naming::MAX_NAME_CHARS {
            return Err(
                CoreError::new(ErrorCode::InvalidName, "the name is too long").with_field("name"),
            );
        }
        Ok(())
    }
}

impl Command for GenProjectCreate {
    const NAME: &'static str = "gen_project_create";
    type Input = GenProjectCreateInput;
    type Output = PathOutput;

    /// FR-GEN-02: a project with no image (`original: null`, `layers: []`, `r0`). The UI then
    /// opens it with `project_open` (FR-GEN-01).
    fn run(core: &Core, input: Self::Input) -> CoreResult<PathOutput> {
        let _one_at_a_time = core.create_lock.lock().unwrap_or_else(|e| e.into_inner());
        let settings = core.state().settings.clone();
        let path = create::create_empty_project(
            &core.ctx,
            &settings,
            input.section.into(),
            &input.name,
            Some(&core.thumbs),
        )?;
        Ok(PathOutput { path })
    }
}

pub struct GenProjectSuggestName;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenProjectSuggestNameInput {
    pub section: SectionInput,
    /// Default: the section's prefill (`New map` / `New token` / `New item`).
    #[serde(default)]
    pub name: Option<String>,
}

impl Validate for GenProjectSuggestNameInput {
    fn validate(&self) -> CoreResult<()> {
        if self
            .name
            .as_ref()
            .is_some_and(|n| n.chars().count() > 4 * naming::MAX_NAME_CHARS)
        {
            return Err(
                CoreError::new(ErrorCode::InvalidName, "the name is too long").with_field("name"),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct NameOutput {
    pub name: String,
}

impl Command for GenProjectSuggestName {
    const NAME: &'static str = "gen_project_suggest_name";
    type Input = GenProjectSuggestNameInput;
    type Output = NameOutput;

    /// FR-GEN-01: the name dialog's prefill under FR-PRJ-44, with ` (2)` on collision.
    fn run(core: &Core, input: Self::Input) -> CoreResult<NameOutput> {
        let settings = core.state().settings.clone();
        Ok(NameOutput {
            name: create::suggest_name(
                &core.ctx,
                &settings,
                input.section.into(),
                input.name.as_deref(),
            ),
        })
    }
}

// ---------------------------------------------------------------- project

pub struct ProjectOpen;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectOpenInput {
    /// The project folder (FR-PRJ-14).
    pub path: String,
}

impl Validate for ProjectOpenInput {
    fn validate(&self) -> CoreResult<()> {
        check_path("path", &self.path)
    }
}

impl Command for ProjectOpen {
    const NAME: &'static str = "project_open";
    type Input = ProjectOpenInput;
    type Output = ProjectSnapshot;

    fn run(core: &Core, input: Self::Input) -> CoreResult<ProjectSnapshot> {
        let mut state = core.state();
        // The open project's waiting save is never dropped by opening another (FR-PRJ-21).
        refuse_unsaved(core, &mut state, Leaving::Close)?;
        let settings = state.settings.clone();
        let mut p = project::open(
            &core.ctx,
            &settings,
            &PathBuf::from(&input.path),
            Some(core.thumbs.clone()),
        )?;
        let install_id = settings.install_id;
        // RGN resumes the project's jobs from their job files (FR-RGN-22).
        crate::region_edit::runner::on_project_open(&core.ctx, install_id, &mut state.rgn, &mut p);
        let snap = p.snapshot();
        crate::export::on_project_close(&core.ctx, &mut state);
        if let Some(old) = state.project.take() {
            // PRM FR-PRM-35: the old project's chat session ends.
            core.chat.on_project_close(old.manifest.project_id);
            old.decoded.release();
        }
        core.served().reset(Some(snap.project_id));
        core.chat.notify_project(&core.ctx, Some(snap.project_id));
        state.project = Some(p);
        Ok(snap)
    }
}

/// Why the open project's waiting save is asked about: the message offers to close anyway only
/// for a close (`force`); a rename has no force and says so (FR-PRJ-56, R111).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leaving {
    Close,
    Rename,
}

/// FR-PRJ-21: closing the open project (to close or to open another) must not lose what only
/// lives in memory. One last try at the waiting draft and EXP's pending settings, then
/// `unsaved` with the reason if something still waits.
fn refuse_unsaved(core: &Core, state: &mut crate::core::State, why: Leaving) -> CoreResult<()> {
    let install_id = state.settings.install_id;
    if let Some(p) = state.project.as_mut() {
        // One last try. Spooled writes are on disk already (FR-PRJ-40); only an in-memory
        // draft makes closing ask.
        project::retry_pending(&core.ctx, install_id, p);
    }
    let export_unsaved = crate::export::unsaved_on_close(&core.ctx, state);
    if let Some(p) = state
        .project
        .as_ref()
        .filter(|p| export_unsaved || project::has_unsaved(p))
    {
        // Says why, without paths (walkthrough D3).
        let moved = p.warnings.iter().any(|w| w.code == project::FOLDER_MOVED);
        let (reason, cause) = if moved {
            (
                project::FOLDER_MOVED,
                "Your latest changes aren't saved: the project folder was moved or renamed.",
            )
        } else if p.conflict.is_some() {
            (
                "conflict",
                "Your latest changes aren't saved: this project was also changed on another machine.",
            )
        } else {
            (
                "locked",
                "Your latest changes aren't saved yet: the file is locked.",
            )
        };
        let then = match why {
            Leaving::Close => "Close anyway?",
            Leaving::Rename => "The name can't change until they are saved.",
        };
        let message = format!("{cause} {then}");
        return Err(CoreError::new(ErrorCode::Unsaved, message)
            .with_detail(serde_json::json!({ "reason": reason })));
    }
    Ok(())
}

pub struct ProjectClose;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectCloseInput {
    /// Close even with a save still pending (FR-PRJ-21: the UI asked first).
    #[serde(default)]
    pub force: bool,
}

impl Validate for ProjectCloseInput {}

impl Command for ProjectClose {
    const NAME: &'static str = "project_close";
    type Input = ProjectCloseInput;
    type Output = Ok_;

    /// Releases the decoded original (R11) and the served images. A save still pending after
    /// `locked` → `unsaved`, unless `force` (FR-PRJ-21). EXP's part of the close: its pending
    /// export settings commit first and count as unsaved if they can't (FR-PRJ-19), and a
    /// running export is canceled once the close goes ahead (OQ-EXP-09: the UI asked).
    fn run(core: &Core, input: ProjectCloseInput) -> CoreResult<Ok_> {
        let mut state = core.state();
        if input.force {
            crate::export::unsaved_on_close(&core.ctx, &mut state);
        } else {
            refuse_unsaved(core, &mut state, Leaving::Close)?;
        }
        if release_open(core, &mut state, false).is_some() {
            core.chat.notify_project(&core.ctx, None);
        }
        core.served().reset(None);
        Ok(Ok_ {})
    }
}

pub struct ProjectGet;

impl Command for ProjectGet {
    const NAME: &'static str = "project_get";
    type Input = NoInput;
    type Output = Option<ProjectSnapshot>;

    fn run(core: &Core, _: NoInput) -> CoreResult<Option<ProjectSnapshot>> {
        Ok(core.state().project.as_ref().map(|p| p.snapshot()))
    }
}

pub struct ProjectPutDraft;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPutDraftInput {
    /// The region edit draft. Absent = keep it (a New image save, FR-GEN-19). A draft that holds
    /// only `generate` (`{draft: {generate: {…}}}`, the New image tab's save) is the same: the
    /// region edit fields keep their saved values and `generate` is merged in.
    #[serde(default, deserialize_with = "draft_or_generate_only")]
    pub draft: Option<DraftInput>,
    /// The New image tab's `draft.generate` (FR-GEN-19): an object, or `null` to clear it.
    /// Absent = keep it. Wins over `draft.generate`.
    #[serde(default, deserialize_with = "crate::project::present_field")]
    pub generate: Option<Option<Value>>,
    /// EXP's unsaved export settings (R30): `grid`, `target_px_per_square`, `format`.
    #[serde(default)]
    pub export: Option<serde_json::Map<String, Value>>,
}

/// `draft` as given, or a generate-only save when its only key is `generate` (FR-GEN-19).
fn draft_or_generate_only<'de, D>(d: D) -> Result<Option<DraftInput>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let v = Option::<Value>::deserialize(d)?;
    let Some(v) = v else { return Ok(None) };
    let only_generate = v
        .as_object()
        .is_some_and(|o| o.len() == 1 && o.contains_key("generate"));
    if only_generate {
        let g = v.get("generate").cloned().unwrap_or(Value::Null);
        let g = if g.is_null() { None } else { Some(g) };
        return Ok(Some(DraftInput::generate_only(g)));
    }
    serde_json::from_value(v)
        .map(Some)
        .map_err(D::Error::custom)
}

/// Largest `draft.generate`, serialised (GEN owns its fields; PRJ only bounds the size).
const MAX_GENERATE_DRAFT_BYTES: usize = 256 * 1024;

fn check_generate_draft(field: &str, g: &Option<Value>) -> CoreResult<()> {
    match g {
        None => Ok(()),
        Some(Value::Object(_)) => {
            let n = serde_json::to_vec(g).map_or(usize::MAX, |b| b.len());
            if n > MAX_GENERATE_DRAFT_BYTES {
                Err(CoreError::invalid(
                    field,
                    "the New image draft is too large",
                ))
            } else {
                Ok(())
            }
        }
        Some(_) => Err(CoreError::invalid(field, "an object or null")),
    }
}

impl ProjectPutDraftInput {
    /// The one draft input the store takes.
    fn into_draft(self) -> (DraftInput, Option<serde_json::Map<String, Value>>) {
        let mut d = self.draft.unwrap_or_else(|| DraftInput {
            keep_region_edit: true,
            ..Default::default()
        });
        if self.generate.is_some() {
            d.generate = self.generate;
        }
        (d, self.export)
    }
}

impl Validate for ProjectPutDraftInput {
    fn validate(&self) -> CoreResult<()> {
        if let Some(g) = &self.generate {
            check_generate_draft("generate", g)?;
        }
        if let Some(d) = &self.draft {
            if let Some(g) = &d.generate {
                check_generate_draft("draft.generate", g)?;
            }
            if d.final_prompt.chars().count() > MAX_PROMPT_CHARS {
                return Err(CoreError::invalid(
                    "draft.final_prompt",
                    "the prompt is too long",
                ));
            }
            if d.model_id
                .as_ref()
                .is_some_and(|m| m.is_empty() || m.len() > 200)
            {
                return Err(CoreError::invalid("draft.model_id", "not a valid model id"));
            }
            crate::prompt::helpers::check_ticks(
                "draft.prompt_helpers",
                d.prompt_helpers.as_deref(),
            )?;
        }
        if let Some(e) = &self.export
            && let Some(k) = e
                .keys()
                .find(|k| !matches!(k.as_str(), "grid" | "target_px_per_square" | "format"))
        {
            return Err(CoreError::invalid(
                format!("export.{k}"),
                "the draft save carries grid, target_px_per_square and format only",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct SeqOutput {
    pub seq: u64,
}

impl Command for ProjectPutDraft {
    const NAME: &'static str = "project_put_draft";
    type Input = ProjectPutDraftInput;
    type Output = SeqOutput;

    fn run(core: &Core, input: Self::Input) -> CoreResult<SeqOutput> {
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let crate::core::State { project, exp, .. } = &mut *state;
        let p = project.as_mut().ok_or_else(no_project)?;
        // R30: the same commit carries EXP's unsaved session settings.
        let from_session = input.export.is_none();
        let (draft, export) = input.into_draft();
        let export = export.or_else(|| crate::export::pending_fields(exp, p));
        let seq = project::put_draft(&core.ctx, install_id, p, draft, export)?;
        if from_session {
            crate::export::clear_pending(exp);
        }
        Ok(SeqOutput { seq })
    }
}

pub struct ProjectSaveRevision;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSaveRevisionInput {
    #[serde(default)]
    pub label: Option<String>,
}

impl Validate for ProjectSaveRevisionInput {
    fn validate(&self) -> CoreResult<()> {
        match &self.label {
            Some(l) if l.trim().is_empty() || l.chars().count() > MAX_LABEL_CHARS => Err(
                CoreError::invalid("label", "a label is 1 to 200 characters"),
            ),
            _ => Ok(()),
        }
    }
}

impl Command for ProjectSaveRevision {
    const NAME: &'static str = "project_save_revision";
    type Input = ProjectSaveRevisionInput;
    type Output = RevisionEntry;

    fn run(core: &Core, input: Self::Input) -> CoreResult<RevisionEntry> {
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let p = state.project.as_mut().ok_or_else(no_project)?;
        project::save_revision(&core.ctx, install_id, p, input.label)
    }
}

pub struct ProjectListRevisions;

impl Command for ProjectListRevisions {
    const NAME: &'static str = "project_list_revisions";
    type Input = NoInput;
    type Output = Vec<RevisionEntry>;

    fn run(core: &Core, _: NoInput) -> CoreResult<Vec<RevisionEntry>> {
        let state = core.state();
        let p = state.project.as_ref().ok_or_else(no_project)?;
        Ok(p.manifest.revisions.clone())
    }
}

pub struct ProjectRestoreRevision;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRestoreRevisionInput {
    pub rev_id: Ulid,
}

impl Validate for ProjectRestoreRevisionInput {}

impl Command for ProjectRestoreRevision {
    const NAME: &'static str = "project_restore_revision";
    type Input = ProjectRestoreRevisionInput;
    type Output = RevisionEntry;

    /// FR-PRJ-23, 43: undo/redo of accept and re-tune.
    fn run(core: &Core, input: Self::Input) -> CoreResult<RevisionEntry> {
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let p = state.project.as_mut().ok_or_else(no_project)?;
        project::restore_revision(&core.ctx, install_id, p, input.rev_id)
    }
}

pub struct ProjectRelinkOriginal;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathInput {
    pub path: String,
}

impl Validate for PathInput {
    fn validate(&self) -> CoreResult<()> {
        check_path("path", &self.path)
    }
}

impl Command for ProjectRelinkOriginal {
    const NAME: &'static str = "project_relink_original";
    type Input = PathInput;
    type Output = Ok_;

    /// FR-PRJ-16.
    fn run(core: &Core, input: PathInput) -> CoreResult<Ok_> {
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let p = state.project.as_mut().ok_or_else(no_project)?;
        project::relink_original(&core.ctx, install_id, p, &PathBuf::from(&input.path))?;
        Ok(Ok_ {})
    }
}

pub struct ProjectPrefetchOriginal;

/// `project_prefetch_original` result: whether the original is decoded already.
#[derive(Debug, Serialize)]
pub struct PrefetchOutput {
    pub ready: bool,
}

impl Command for ProjectPrefetchOriginal {
    const NAME: &'static str = "project_prefetch_original";
    type Input = NoInput;
    type Output = PrefetchOutput;

    /// FR-PRJ-42, R11: starts the background decode; `project:original-ready` follows. When the
    /// original is decoded already, the event is sent again (R43). A decode that already failed
    /// reports its error (`corrupt`, …).
    fn run(core: &Core, _: NoInput) -> CoreResult<PrefetchOutput> {
        let decoded = {
            let state = core.state();
            let p = state.project.as_ref().ok_or_else(no_project)?;
            if p.read_only == Some(project::ReadOnlyReason::OriginalMissing) {
                return Err(CoreError::new(
                    ErrorCode::NotFound,
                    "the original is missing",
                ));
            }
            // FR-GEN-03: a project with no image has nothing to decode (`no_image`).
            let base = p.base_path().ok_or_else(crate::decoded::no_image)?;
            library::hydrate_if_needed(&core.ctx, &base)?;
            p.decoded.clone()
        };
        let was_ready = decoded.is_ready();
        match decoded.get(&core.ctx) {
            Ok(_) => {
                if was_ready {
                    core.ctx.emit(
                        "project:original-ready",
                        serde_json::json!({ "project_id": decoded.project_id() }),
                    );
                }
                Ok(PrefetchOutput { ready: true })
            }
            Err(e) if e.code == ErrorCode::OriginalUnavailable => {
                Ok(PrefetchOutput { ready: false })
            }
            Err(e) => Err(e),
        }
    }
}

pub struct ProjectFolderList;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderInput {
    pub folder: String,
}

impl Validate for FolderInput {}

fn with_thumb_urls(core: &Core, mut files: Vec<FolderFile>) -> Vec<FolderFile> {
    for f in files.iter_mut().filter(|f| f.image) {
        f.thumb_url = tiles::register_library_image(core, &f.path);
    }
    files
}

impl Command for ProjectFolderList {
    const NAME: &'static str = "project_folder_list";
    type Input = FolderInput;
    type Output = Vec<FolderFile>;

    /// FR-PRJ-48: on open, focus and Refresh; metadata only. BL-30: `folder_moved` once the
    /// folder is gone, not an empty list.
    fn run(core: &Core, input: FolderInput) -> CoreResult<Vec<FolderFile>> {
        let files = {
            let mut state = core.state();
            let p = state.project.as_mut().ok_or_else(no_project)?;
            project::require_folder(p)?;
            project::folder_list(p, &input.folder)?
        };
        let mut files = files;
        project::set_file_states(&core.ctx, &mut files);
        Ok(with_thumb_urls(core, files))
    }
}

pub struct ProjectFolderAdd;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectFolderAddInput {
    pub folder: String,
    pub paths: Vec<String>,
}

impl Validate for ProjectFolderAddInput {
    fn validate(&self) -> CoreResult<()> {
        if self.paths.is_empty() || self.paths.len() > MAX_PATHS {
            return Err(CoreError::invalid(
                "paths",
                format!("give 1 to {MAX_PATHS} paths"),
            ));
        }
        for (i, p) in self.paths.iter().enumerate() {
            check_path(&format!("paths[{i}]"), p)?;
        }
        Ok(())
    }
}

impl Command for ProjectFolderAdd {
    const NAME: &'static str = "project_folder_add";
    type Input = ProjectFolderAddInput;
    type Output = Vec<FolderFile>;

    /// FR-PRJ-47: copies into `references/`, `structure/` or `parts/`.
    fn run(core: &Core, input: Self::Input) -> CoreResult<Vec<FolderFile>> {
        let files = {
            let state = core.state();
            let p = state.project.as_ref().ok_or_else(no_project)?;
            let paths: Vec<PathBuf> = input.paths.iter().map(PathBuf::from).collect();
            project::folder_add(&core.ctx, p, &input.folder, &paths)?
        };
        let mut files = files;
        project::set_file_states(&core.ctx, &mut files);
        Ok(with_thumb_urls(core, files))
    }
}

/// `project_base_from_file` (FR-PRJ-55, #61): one image file dropped on the Working section
/// becomes the base image, as one undoable revision (undo = `project_restore_revision(parent)`).
pub struct ProjectBaseFromFile;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectBaseFromFileInput {
    pub path: String,
}

impl Validate for ProjectBaseFromFileInput {
    fn validate(&self) -> CoreResult<()> {
        check_path("path", &self.path)
    }
}

impl Command for ProjectBaseFromFile {
    const NAME: &'static str = "project_base_from_file";
    type Input = ProjectBaseFromFileInput;
    type Output = project::store::BaseCommit;

    fn run(core: &Core, input: Self::Input) -> CoreResult<project::store::BaseCommit> {
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let p = state.project.as_mut().ok_or_else(no_project)?;
        project::require_folder(p)?;
        p.require_writable()?;
        let out =
            project::store::base_from_file(&core.ctx, install_id, p, &PathBuf::from(&input.path))?;
        let project_id = p.manifest.project_id;
        let seq = project::revision_seq(p, out.revision_id);
        drop(state);
        // #61: like an accept or a pick, the chat starts a fresh thread (PRM FR-PRM-47).
        crate::prompt::chat::start_after_commit(
            core,
            project_id,
            crate::prompt::chat::thread::StartReason::Dropped,
            out.revision_id,
            seq,
        );
        Ok(out)
    }
}

// ---------------------------------------------------------------- editor

/// EDT `edt_image_open` (FR-EDT-31/32): a whole image of the open project (the original or a
/// layer file) under `/i/<key>`. Without `max_side` it must be ≤ 4096 per side (`use_tiles`);
/// with it, a cached Lanczos3 PNG copy that fits within `max_side` (the picker uses 256).
pub struct EdtImageOpen;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdtImageOpenInput {
    pub project_id: Ulid,
    pub image_id: Ulid,
    #[serde(default)]
    pub max_side: Option<u32>,
}

impl Validate for EdtImageOpenInput {
    fn validate(&self) -> CoreResult<()> {
        match self.max_side {
            Some(m) if !(1..=tiles::WHOLE_IMAGE_MAX_SIDE).contains(&m) => {
                Err(CoreError::invalid("max_side", "max_side is 1 to 4096 px"))
            }
            _ => Ok(()),
        }
    }
}

pub use crate::tiles::{ImageOpenOutput, TilesOpenOutput};

impl Command for EdtImageOpen {
    const NAME: &'static str = "edt_image_open";
    type Input = EdtImageOpenInput;
    type Output = ImageOpenOutput;

    fn run(core: &Core, input: Self::Input) -> CoreResult<ImageOpenOutput> {
        tiles::image_open(core, input.project_id, input.image_id, input.max_side)
    }
}

// ---------------------------------------------------------------- provider

/// Provider commands live in their own module (provider spec §9).
pub use crate::provider_commands::{
    ProviderBalance, ProviderCredentialsClear, ProviderCredentialsSet, ProviderFakeSetCase,
    ProviderGetCapabilities, ProviderListModels, ProviderSettingsGet, ProviderSettingsSet,
    ProviderStatus, ProviderTestConnection,
};

// ---------------------------------------------------------------- editor: tiles

/// EDT `edt_tiles_open` (FR-EDT-07): the pyramid descriptor at once; missing levels build in
/// the background, smallest first (`edt:tiles-progress` / `edt:tiles-ready` / `edt:tiles-failed`).
pub struct EdtTilesOpen;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdtTilesOpenInput {
    pub project_id: Ulid,
    pub image_id: Ulid,
    /// Deletes the cached pyramid and builds it again (the tile error badge, FR-EDT-12).
    #[serde(default)]
    pub rebuild: bool,
}

impl Validate for EdtTilesOpenInput {}

impl Command for EdtTilesOpen {
    const NAME: &'static str = "edt_tiles_open";
    type Input = EdtTilesOpenInput;
    type Output = TilesOpenOutput;

    fn run(core: &Core, input: Self::Input) -> CoreResult<TilesOpenOutput> {
        tiles::tiles_open(core, input.project_id, input.image_id, input.rebuild)
    }
}

// ---------------------------------------------------------------- editor: masks

/// EDT `edt_mask_submit` (FR-EDT-18, 28, 29): the inner-rect mask's w·h bytes (base64) → PRJ
/// `put_mask`, which writes the PNG and points the draft at it in one commit (R7).
pub struct EdtMaskSubmit;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdtMaskSubmitInput {
    pub project_id: Ulid,
    pub rect: crate::manifest::Rect,
    /// `rect.w × rect.h` bytes, row-major, 255 = editable, base64.
    pub data_b64: String,
}

impl Validate for EdtMaskSubmitInput {
    fn validate(&self) -> CoreResult<()> {
        crate::masks::check_rect_size(&self.rect)
    }
}

pub use crate::masks::{MaskReadOutput, MaskRef};

impl Command for EdtMaskSubmit {
    const NAME: &'static str = "edt_mask_submit";
    type Input = EdtMaskSubmitInput;
    type Output = MaskRef;

    fn run(core: &Core, input: Self::Input) -> CoreResult<MaskRef> {
        crate::masks::submit(core, input.project_id, input.rect, &input.data_b64)
    }
}

/// EDT `edt_mask_read` (FR-EDT-18, 19): a stored mask's rect and bytes (base64), byte-identical
/// to the submit.
pub struct EdtMaskRead;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdtMaskReadInput {
    pub project_id: Ulid,
    pub mask_id: Ulid,
}

impl Validate for EdtMaskReadInput {}

impl Command for EdtMaskRead {
    const NAME: &'static str = "edt_mask_read";
    type Input = EdtMaskReadInput;
    type Output = MaskReadOutput;

    fn run(core: &Core, input: Self::Input) -> CoreResult<MaskReadOutput> {
        crate::masks::read(core, input.project_id, input.mask_id)
    }
}

// ---------------------------------------------------------------- region edit

pub use crate::region_edit::candidates::{
    RgnCandidateAccept, RgnCandidateDiscard, RgnCandidatePreview, RgnCandidateStar,
    RgnIterationDiscardAll, RgnIterationDiscardAllUndo, RgnIterationsList, RgnJobDiscardRest,
    RgnPatchRetune,
};
/// Region-edit commands live in their own module (region-edit spec §9).
pub use crate::region_edit::commands::{
    RgnControlsEvaluate, RgnJobEstimate, RgnRegionFit, RgnSpendGet, RgnSpendSet,
};
pub use crate::region_edit::instruction::{RgnBlendInstructionGet, RgnBlendInstructionSet};
pub use crate::region_edit::jobs::{
    RgnJobCancel, RgnJobDismiss, RgnJobFetchAgain, RgnJobRetryDraft, RgnJobSubmit,
};

// ---------------------------------------------------------------- reference packs

/// Reference-pack commands live in their own module (reference-packs spec §9).
pub use crate::references::commands::{
    RefListPacks, RefPackAddImages, RefPackCreate, RefPackDelete, RefPackGet, RefPackRemoveImages,
    RefPackRename, RefPackUpdate, RefResolve, RefTrashList, RefTrashRestore,
};

/// PRJ retention and conflict commands (asset-project §9, slices 5 and 6).
pub use crate::project::commands::{
    ProjectConflictGet, ProjectDiskUsage, ProjectResolveConflict, ProjectTrashList,
    ProjectTrashRestore,
};

// ---------------------------------------------------------------- export

/// Export commands live in their own module (export spec §9).
pub use crate::export::commands::{
    ExportCancel, ExportGetSettings, ExportPlan, ExportSetSettings, ExportStart,
};

// ---------------------------------------------------------------- prompt helper

/// Prompt helper commands live in their own module (prompt-helper spec §9).
pub use crate::prompt::commands::{
    PrmComposePreview, PrmHelpersList, PrmHelpersReset, PrmHelpersSave,
};

/// Prompt chat commands (prompt-helper §9, PRM slice 5).
pub use crate::prompt::chat::{
    PrmChatCancel, PrmChatConsent, PrmChatFakeSetCase, PrmChatInsert, PrmChatLogout,
    PrmChatPreview, PrmChatSend, PrmChatSessionEnd, PrmChatSettingsPut, PrmChatStatus,
    PrmChatThreadGet, PrmChatThreadNew, PrmContextClear, PrmContextPublish,
};

// ---------------------------------------------------------------- new-asset generation

/// GEN slice 3 commands live in their own module (new-asset-generation §9).
pub use crate::generate::commands::{
    GenCandidatePick, GenCandidatePreview, GenControlsEvaluate, GenJobEstimate, GenJobRetryDraft,
    GenJobSubmit, GenSizeFitCmd,
};

// ---------------------------------------------------------------- registry (append-only)

/// A registry row: command name → JSON adapter.
pub type Handler = fn(&Core, Value) -> CoreResult<Value>;

macro_rules! registry {
    ($($cmd:ty),* $(,)?) => {
        /// Every core command by name. **Append-only**: add rows, never rename or remove one.
        pub const REGISTRY: &[(&str, Handler)] = &[
            $((<$cmd as Command>::NAME, dispatch_json::<$cmd> as Handler)),*
        ];
    };
}

registry![
    AppInfo,
    LibraryGetConfig,
    LibrarySetRoots,
    ProjectOpen,
    ProjectClose,
    ProjectGet,
    ProjectPutDraft,
    ProjectSaveRevision,
    ProjectListRevisions,
    EdtImageOpen,
    ProviderStatus,
    EdtTilesOpen,
    ProviderCredentialsSet,
    ProviderCredentialsClear,
    ProviderTestConnection,
    ProviderSettingsGet,
    ProviderSettingsSet,
    ProviderListModels,
    ProviderGetCapabilities,
    ProviderBalance,
    ProviderFakeSetCase,
    RgnRegionFit,
    RgnControlsEvaluate,
    RgnJobEstimate,
    RgnSpendGet,
    RgnSpendSet,
    LibraryListProjects,
    ProjectCreate,
    ProjectCreateCancel,
    ProjectCreateFromClipboard,
    ProjectRename,
    ProjectRestoreRevision,
    ProjectRelinkOriginal,
    ProjectPrefetchOriginal,
    ProjectFolderList,
    ProjectFolderAdd,
    RgnJobSubmit,
    RgnJobCancel,
    RgnJobRetryDraft,
    RgnJobDismiss,
    RgnCandidatePreview,
    RgnCandidateAccept,
    RgnCandidateStar,
    RgnCandidateDiscard,
    RgnPatchRetune,
    RgnIterationsList,
    RefListPacks,
    RefPackGet,
    RefPackCreate,
    RefPackRename,
    RefPackUpdate,
    RefPackAddImages,
    RefPackRemoveImages,
    RefPackDelete,
    RefTrashList,
    RefTrashRestore,
    RefResolve,
    ProjectTrashList,
    ProjectTrashRestore,
    ProjectDiskUsage,
    ProjectConflictGet,
    ProjectResolveConflict,
    ExportGetSettings,
    ExportSetSettings,
    ExportPlan,
    ExportStart,
    ExportCancel,
    EdtMaskSubmit,
    EdtMaskRead,
    RgnJobDiscardRest,
    RgnBlendInstructionGet,
    RgnBlendInstructionSet,
    PrmHelpersList,
    PrmHelpersSave,
    PrmHelpersReset,
    PrmComposePreview,
    GenProjectCreate,
    GenProjectSuggestName,
    GenSizeFitCmd,
    GenControlsEvaluate,
    GenJobEstimate,
    GenJobSubmit,
    GenJobRetryDraft,
    GenCandidatePreview,
    GenCandidatePick,
    PrmChatStatus,
    PrmChatConsent,
    PrmChatSettingsPut,
    PrmChatLogout,
    PrmChatSessionEnd,
    PrmContextPublish,
    PrmContextClear,
    PrmChatThreadGet,
    PrmChatSend,
    PrmChatCancel,
    PrmChatInsert,
    PrmChatFakeSetCase,
    PrmChatPreview,
    RgnJobFetchAgain,
    RgnIterationDiscardAll,
    RgnIterationDiscardAllUndo,
    ProjectBaseFromFile,
    PrmChatThreadNew,
    ProjectDelete,
    LibraryTrashList,
    LibraryTrashRestore,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::time::Timestamp;
    use serde_json::json;

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(1_800_000_000_000));
        ctx.library_override = Some(dir.path().join("lib"));
        let core = Core::new(ctx).unwrap();
        (dir, core)
    }

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&vec![50; (w * h * 3) as usize])
            .unwrap();
        wr.finish().unwrap();
        out
    }

    #[test]
    fn registry_names_are_unique_snake_case() {
        let mut names: Vec<_> = REGISTRY.iter().map(|(n, _)| *n).collect();
        assert!(
            names
                .iter()
                .all(|n| n.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
        );
        let len = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), len);
    }

    #[test]
    fn unknown_command_and_bad_input_are_normalised_errors() {
        let (_d, core) = core();
        assert_eq!(
            core.dispatch("nope", json!({})).unwrap_err().code,
            ErrorCode::NotFound
        );
        let e = core
            .dispatch("project_create", json!({"section": "maps", "paths": "x"}))
            .unwrap_err();
        assert_eq!(
            (e.code, e.field.as_deref()),
            (ErrorCode::InvalidInput, Some("input"))
        );
        let e = core
            .dispatch("project_create", json!({"section": "maps", "paths": [""]}))
            .unwrap_err();
        assert_eq!(e.field.as_deref(), Some("paths[0]"));
        let e = core
            .dispatch(
                "project_create",
                json!({"section": "dungeons", "paths": ["a"]}),
            )
            .unwrap_err();
        assert_eq!(e.field.as_deref(), Some("input"));
        for retired in ["import_paths", "recent_list", "import_clipboard_image"] {
            assert_eq!(
                core.dispatch(retired, json!({})).unwrap_err().code,
                ErrorCode::NotFound
            );
        }
        let e = core
            .dispatch("project_save_revision", json!({}))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NoProject);
        let e = core.dispatch("app_info", json!({"extra": 1})).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn fr_prj_55_base_from_file_validates_the_path_at_the_boundary() {
        let (_dir, core) = core();
        let bad = [
            json!({"path": "  "}),
            json!({"path": "a\u{0}b"}),
            json!({}),
            json!({"path": "a.png", "x": 1}),
        ];
        for input in bad {
            let e = core
                .dispatch("project_base_from_file", input.clone())
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidInput, "{input}");
        }
        let e = core
            .dispatch("project_base_from_file", json!({"path": "a.png"}))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NoProject);
    }

    #[test]
    fn create_list_open_image_serve_save_reopen_end_to_end() {
        let (dir, core) = core();
        let src = dir.path().join("outside.png");
        std::fs::write(&src, png_bytes(64, 48)).unwrap();

        let out = core
            .dispatch(
                "project_create",
                json!({"section": "maps", "paths": [src.to_string_lossy()]}),
            )
            .unwrap();
        assert_eq!(out["items"][0]["status"], "created");
        let path = out["items"][0]["path"].as_str().unwrap().to_owned();
        assert!(path.ends_with("outside"));

        let cards = core
            .dispatch("library_list_projects", json!({"section": "maps"}))
            .unwrap();
        assert_eq!(cards[0]["name"], "outside");
        assert_eq!(cards[0]["path"].as_str().unwrap(), path);
        let thumb = cards[0]["thumb_url"].as_str().unwrap();
        assert!(thumb.contains("/i/l-"), "{thumb}");
        let res = core.serve_scheme(&thumb[thumb.find("/i/").unwrap()..]);
        assert_eq!((res.status, res.content_type), (200, Some("image/jpeg")));

        let snap = core
            .dispatch("project_open", json!({"path": path}))
            .unwrap();
        assert_eq!(snap["kind"], "map");
        assert_eq!(snap["original_image_id"], snap["layers"][0]["id"]);
        assert!(snap["committed_at"].is_string());
        assert_eq!(
            (snap["width"].as_u64(), snap["height"].as_u64()),
            (Some(64), Some(48))
        );

        let img = core
            .dispatch(
                "edt_image_open",
                json!({"project_id": snap["project_id"], "image_id": snap["layers"][0]["id"]}),
            )
            .unwrap();
        let url = img["url"].as_str().unwrap();
        let scheme_path = &url[url.find("/i/").unwrap()..];
        let res = core.serve_scheme(scheme_path);
        assert_eq!((res.status, res.content_type), (200, Some("image/png")));

        core.dispatch(
            "project_put_draft",
            json!({"draft": {"final_prompt": "moss"}}),
        )
        .unwrap();
        let rev = core.dispatch("project_save_revision", json!({})).unwrap();
        assert_eq!(rev["reason"], "save");
        core.dispatch("project_close", json!({})).unwrap();
        assert_eq!(
            core.serve_scheme(scheme_path).status,
            404,
            "keys unregistered on close"
        );

        assert_eq!(
            core.serve_scheme(&thumb[thumb.find("/i/").unwrap()..])
                .status,
            200,
            "library keys survive close"
        );
        let again = core
            .dispatch("project_open", json!({"path": path}))
            .unwrap();
        assert_eq!(again["project_id"], snap["project_id"]);
        assert_eq!(again["head_revision"], rev["id"]);
    }

    #[test]
    fn fr_prj_01_set_roots_validates_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::deterministic(dir.path().join("data"), Timestamp(0));
        let core = Core::new(ctx).unwrap();
        let cfg = core.dispatch("library_get_config", json!({})).unwrap();
        assert_eq!(cfg["configured"], false);
        let e = core
            .dispatch(
                "library_set_roots",
                json!({"roots": [dir.path().join("missing")]}),
            )
            .unwrap_err();
        assert_eq!(e.field.as_deref(), Some("roots[0]"));
        let lib = dir.path().join("lib");
        std::fs::create_dir(&lib).unwrap();
        let cfg = core
            .dispatch("library_set_roots", json!({"roots": [lib]}))
            .unwrap();
        assert_eq!(cfg["configured"], true);
        let saved: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("data/settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            saved["library"]["roots"][0].as_str().unwrap(),
            lib.to_string_lossy()
        );
    }
}

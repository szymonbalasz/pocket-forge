//! EXP core commands (export spec §9, ruling R43): inputs nested `{settings}` /
//! `{settings, replace}`; `export_plan.path` is project-relative, `export:done.path` absolute.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ulid::Ulid;

use super::run::{self, Dest, ExportJob, Input};
use super::settings::{self, ExportFormat, ExportSettings, Plan};
use super::{Running, Scope, flush_pending, set_settings, settings_output, spawn_debounce};
use crate::commands::{NoInput, Ok_, no_project};
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::job::JobState;
use crate::library;
use crate::manifest::Kind;
use crate::project::{OpenProject, folder_name, probe_error, store};
use crate::settings::Settings;

/// `{settings}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub settings: ExportSettings,
}

impl Validate for SettingsInput {}

pub struct ExportGetSettings;

impl Command for ExportGetSettings {
    const NAME: &'static str = "export_get_settings";
    type Input = NoInput;
    type Output = Value;

    /// FR-EXP-26: the settings plus `source_px_per_square` and `target_px_per_square`.
    fn run(core: &Core, _: NoInput) -> CoreResult<Value> {
        settings_output(&mut core.state())
    }
}

pub struct ExportSetSettings;

impl Command for ExportSetSettings {
    const NAME: &'static str = "export_set_settings";
    type Input = SettingsInput;
    type Output = Value;

    /// FR-EXP-25: session state; the next draft save commits it (R30).
    fn run(core: &Core, input: SettingsInput) -> CoreResult<Value> {
        let mut state = core.state();
        let needs_timer = set_settings(&core.ctx, &mut state, input.settings)?;
        let out = settings_output(&mut state)?;
        drop(state);
        if needs_timer && !core.ctx.inline_background {
            spawn_debounce(core.ctx.clone(), core.state_handle());
        }
        Ok(out)
    }
}

pub struct ExportPlan;

impl Command for ExportPlan {
    const NAME: &'static str = "export_plan";
    type Input = SettingsInput;
    type Output = Plan;

    /// FR-EXP-24: pure; only stats the destination.
    fn run(core: &Core, input: SettingsInput) -> CoreResult<Plan> {
        let state = core.state();
        let p = state.project.as_ref().ok_or_else(no_project)?;
        // FR-GEN-03: Export is disabled with `no_image` until the first pick.
        p.require_image()?;
        settings::check_destination(&p.dir)?;
        let (w, h) = p.manifest.image_size().unwrap_or((0, 0));
        settings::plan(&input.settings, &folder_name(&p.dir), &p.dir, w, h)
    }
}

pub struct ExportStart;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportStartInput {
    pub settings: ExportSettings,
    #[serde(default)]
    pub replace: bool,
    /// FR-EXP-30: Export as from the canvas menu: this format, once; the settings don't change.
    #[serde(default)]
    pub format_override: Option<ExportFormat>,
    /// FR-EXP-31: Save as: an absolute path from the save dialog; its extension picks the format.
    #[serde(default)]
    pub dest: Option<String>,
}

impl Validate for ExportStartInput {
    fn validate(&self) -> CoreResult<()> {
        match &self.dest {
            Some(d) => crate::commands::check_path("dest", d),
            None => Ok(()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ExportStartOutput {
    pub export_id: Ulid,
}

fn busy(message: &str) -> CoreError {
    CoreError::new(ErrorCode::Busy, message)
}

fn exists_error(name: &str, existing: Value) -> CoreError {
    CoreError::new(ErrorCode::Exists, format!("{name} exists")).with_detail(existing)
}

/// Runs a job already pushed onto `exp.running` (the state lock is released): inline in tests,
/// else on its own thread.
fn start_job(core: &Core, job: ExportJob) -> CoreResult<ExportStartOutput> {
    let id = job.id;
    if core.ctx.inline_background {
        run::run(&core.ctx, &core.state_handle(), job);
    } else {
        let (ctx, st) = (core.ctx.clone(), core.state_handle());
        let spawned = std::thread::Builder::new()
            .name("pf-export".into())
            .spawn(move || run::run(&ctx, &st, job));
        if let Err(e) = spawned {
            core.state().exp.finish(id);
            return Err(CoreError::internal(format!("can't start the export: {e}")));
        }
    }
    Ok(ExportStartOutput { export_id: id })
}

fn format_mismatch(field: &str) -> CoreError {
    CoreError::new(
        ErrorCode::InvalidSettings,
        "the Save as extension and the format differ",
    )
    .with_field(field)
}

impl Command for ExportStart {
    const NAME: &'static str = "export_start";
    type Input = ExportStartInput;
    type Output = ExportStartOutput;

    /// Checks (read-only, busy, original, settings, format, destination), commits the settings
    /// (R6) and starts the background export. Nothing is written when a check fails. With
    /// `format_override` (FR-EXP-30) or `dest` (FR-EXP-31) the settings are neither committed nor
    /// changed; a `dest` export writes nothing to the project at all.
    fn run(core: &Core, input: ExportStartInput) -> CoreResult<ExportStartOutput> {
        let ctx = &core.ctx;
        let save_as = input
            .dest
            .as_deref()
            .map(settings::save_as_target)
            .transpose()?;
        if let (Some((_, f)), Some(o)) = (&save_as, input.format_override)
            && *f != o
        {
            return Err(format_mismatch("format_override"));
        }
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let crate::core::State { project, exp, .. } = &mut *state;
        let p = project.as_mut().ok_or_else(no_project)?;
        exp.bind(p);
        // FR-EXP-29, R12: fail fast, write nothing. A Save as writes nothing to the project, so
        // it works on a read-only one (FR-EXP-31).
        if save_as.is_none() {
            p.require_writable()?;
        }
        let project_id = p.manifest.project_id;
        if exp.is_running(project_id) {
            return Err(busy("an export of this project is already running"));
        }
        // R31: a region job fetching its candidates shares the decoded original.
        if p.jobs.values().any(|j| j.state == JobState::Succeeded) {
            return Err(busy("a region job is fetching its candidates"));
        }
        // FR-EXP-28, R11; `no_image` with no image yet (FR-GEN-03). `layers[0]`'s pixels, alpha
        // over the flatten colour (FR-EXP-04 amended, FR-GEN-12, 33).
        p.require_image()?;
        let original = p.decoded.get(ctx)?;
        let (w, h) = p.manifest.image_size().unwrap_or((0, 0));
        let s = input.settings;
        let sizing = s.sizing(w, h)?;
        let format = save_as
            .as_ref()
            .map(|(_, f)| *f)
            .or(input.format_override)
            .unwrap_or(s.format);
        if let Some(reason) = settings::unavailable(format, &sizing) {
            return Err(
                CoreError::new(ErrorCode::TooLarge, reason).with_detail(json!({
                    "format": format,
                    "limit": pf_imaging::WEBP_MAX_SIDE,
                })),
            );
        }
        let one_off = save_as.is_some() || input.format_override.is_some();
        let dest = match save_as {
            Some((path, _)) => {
                settings::check_save_dir(&path)?;
                // The dialog already asked about replacing (FR-EXP-31).
                Dest::SaveAs(path)
            }
            None => {
                settings::check_destination(&p.dir)?;
                let rel_path =
                    settings::rel_path(&folder_name(&p.dir), s.target_px_per_square, format);
                let path = settings::abs_path(&p.dir, &rel_path);
                if !input.replace
                    && let Some(existing) = settings::existing(&path)
                {
                    return Err(exists_error(&rel_path, existing));
                }
                Dest::Exports {
                    path,
                    rel_path,
                    record: true,
                }
            }
        };
        if !one_off {
            // R6, FR-EXP-25: the settings commit; PRJ's read_only / locked / conflict stop here.
            if ExportSettings::from_record(p.manifest.export.as_ref()).as_ref() != Some(&s) {
                store::put_export(ctx, install_id, p, s.record_fields())?;
            }
            exp.session = Some(s.clone());
            exp.pending_since = None;
        }

        let id = ctx.new_id();
        let cancel = Arc::new(AtomicBool::new(false));
        let scope = Scope::Project(project_id);
        exp.running.push(Running {
            id,
            scope,
            cancel: cancel.clone(),
        });
        let job = ExportJob {
            id,
            scope,
            input: Input::Stack {
                project_dir: p.dir.clone(),
                rescale: sizing.rescales(),
                original,
                layers: p.manifest.layers.clone(),
                head_revision: p.manifest.head_revision,
            },
            dest,
            format,
            width: sizing.width,
            height: sizing.height,
            cancel,
            seams: exp.seams.clone(),
        };
        drop(state);
        start_job(core, job)
    }
}

/// Whose file an `export_file` source is.
enum Owner {
    /// A file in a project folder (`<root>/<section>/<project>/<rel>`).
    Project { dir: PathBuf, rel: String },
    /// A pack image, or any other file in a root: no project, no `exports/`.
    Other,
}

/// FR-EXP-32: where a resolved source lives.
fn owner_of(ctx: &crate::context::Context, settings: &Settings, source: &Path) -> Owner {
    for root in library::effective_roots(ctx, settings) {
        let root = std::fs::canonicalize(&root)
            .map(library::plain_path)
            .unwrap_or(root);
        let Ok(rel) = source.strip_prefix(&root) else {
            continue;
        };
        let parts: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if parts.len() >= 3 && Kind::from_section_dir(&parts[0]).is_some() {
            return Owner::Project {
                dir: root.join(&parts[0]).join(&parts[1]),
                rel: parts[2..].join("/"),
            };
        }
        return Owner::Other;
    }
    Owner::Other
}

/// The open project's folder, resolved as sources are (links, `..`).
fn open_dir(p: &OpenProject) -> PathBuf {
    std::fs::canonicalize(&p.dir)
        .map(library::plain_path)
        .unwrap_or_else(|_| p.dir.clone())
}

pub struct ExportFile;

/// `export_file{source, format, dest, replace}` (FR-EXP-32). `dest` = `"exports"` or an absolute
/// Save as path; with a path its extension picks the format and `format` may be left out.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportFileInput {
    pub source: String,
    #[serde(default)]
    pub format: Option<ExportFormat>,
    pub dest: String,
    #[serde(default)]
    pub replace: bool,
}

impl Validate for ExportFileInput {
    fn validate(&self) -> CoreResult<()> {
        crate::commands::check_path("source", &self.source)?;
        crate::commands::check_path("dest", &self.dest)
    }
}

/// `export_file`'s `dest` for the project's `exports/`.
pub const DEST_EXPORTS: &str = "exports";

impl Command for ExportFile {
    const NAME: &'static str = "export_file";
    type Input = ExportFileInput;
    type Output = ExportStartOutput;

    /// One image file to `exports/` or a Save as path: a byte copy for the same format, else
    /// decoded and encoded at its own size. Nothing is committed to the manifest. Every check
    /// (source, destination, `exists`, `busy`, offline, the image checks) runs before an
    /// `export_id` is returned.
    fn run(core: &Core, input: ExportFileInput) -> CoreResult<ExportStartOutput> {
        let ctx = &core.ctx;
        let save_as = if input.dest == DEST_EXPORTS {
            None
        } else {
            Some(settings::save_as_target(&input.dest)?)
        };
        let format = match (&save_as, input.format) {
            (Some((_, f)), Some(g)) if *f != g => return Err(format_mismatch("format")),
            (Some((_, f)), _) => *f,
            (None, Some(f)) => f,
            (None, None) => {
                return Err(CoreError::new(ErrorCode::InvalidSettings, "pick a format")
                    .with_field("format"));
            }
        };
        let lib_settings = core.state().settings.clone();
        let source = library::resolve_in_library(ctx, &lib_settings, Path::new(&input.source))?;
        if !source.is_file() {
            return Err(CoreError::new(
                ErrorCode::NotFound,
                format!("{} is not a file", source.display()),
            )
            .with_field("source"));
        }
        let owner = owner_of(ctx, &lib_settings, &source);
        let checks = |state: &mut crate::core::State| -> CoreResult<(Scope, Dest)> {
            let crate::core::State { project, exp, .. } = state;
            let (scope, project) = match &owner {
                Owner::Project { dir, .. } => {
                    // The menus are on the open project's files (FR-PRJ-59).
                    let p = project
                        .as_ref()
                        .filter(|p| library::same_folder(&open_dir(p), dir))
                        .ok_or_else(|| {
                            no_project().with_detail(json!({ "reason": "not the open project" }))
                        })?;
                    (Scope::Project(p.manifest.project_id), Some(p))
                }
                Owner::Other => (Scope::Pack, None),
            };
            let dest = match (&save_as, project, &owner) {
                (Some((path, _)), _, _) => {
                    settings::check_save_dir(path)?;
                    Dest::SaveAs(path.clone())
                }
                (None, Some(p), Owner::Project { rel, .. }) => {
                    p.require_writable()?;
                    settings::check_destination(&p.dir)?;
                    let name = settings::file_export_name(&folder_name(&p.dir), rel, format);
                    let rel_path = format!("{}/{name}", crate::manifest::EXPORTS_DIR);
                    let path = settings::abs_path(&p.dir, &rel_path);
                    if !input.replace
                        && let Some(existing) = settings::existing(&path)
                    {
                        return Err(exists_error(&rel_path, existing));
                    }
                    Dest::Exports {
                        path,
                        rel_path,
                        record: false,
                    }
                }
                _ => {
                    return Err(CoreError::new(
                        ErrorCode::InvalidSettings,
                        "a pack image has no exports folder; use Save as",
                    )
                    .with_field("dest"));
                }
            };
            if exp.holds(scope) {
                return Err(busy(match scope {
                    Scope::Pack => "a pack image is already being saved",
                    Scope::Project(_) => "an export of this project is already running",
                }));
            }
            Ok((scope, dest))
        };
        checks(&mut core.state())?;
        // FR-PRJ-17: a file the user exports is hydrated; offline → not_available_offline.
        library::hydrate_if_needed(ctx, &source)?;
        // FR-PRJ-06's checks, by content: `unsupported`, `corrupt`, `animated`, `too_large`.
        let info = pf_imaging::probe_file_complete(&source).map_err(probe_error)?;
        if let Some(reason) =
            pf_imaging::format_unavailable(format.image_format(), info.width, info.height)
        {
            return Err(
                CoreError::new(ErrorCode::TooLarge, reason).with_detail(json!({
                    "format": format,
                    "limit": pf_imaging::WEBP_MAX_SIDE,
                })),
            );
        }
        let mut state = core.state();
        // Again under the lock: the hydrate and probe ran without it.
        let (scope, dest) = checks(&mut state)?;
        let id = ctx.new_id();
        let cancel = Arc::new(AtomicBool::new(false));
        state.exp.running.push(Running {
            id,
            scope,
            cancel: cancel.clone(),
        });
        let job = ExportJob {
            id,
            scope,
            input: Input::File {
                path: source,
                copy: info.format == format.image_format(),
            },
            dest,
            format,
            width: info.width,
            height: info.height,
            cancel,
            seams: state.exp.seams.clone(),
        };
        drop(state);
        start_job(core, job)
    }
}

pub struct ExportSaveDefault;

/// `export_save_default{source?, format?}`: where the save dialog opens (FR-EXP-31, 32).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportSaveDefaultInput {
    /// A file (FR-EXP-32); absent = the canvas (FR-EXP-31).
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub format: Option<ExportFormat>,
}

impl Validate for ExportSaveDefaultInput {
    fn validate(&self) -> CoreResult<()> {
        match &self.source {
            Some(s) => crate::commands::check_path("source", s),
            None => Ok(()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ExportSaveDefaultOutput {
    /// The dialog's default path: the folder (if any) joined with the default name.
    pub path: PathBuf,
}

impl Command for ExportSaveDefault {
    const NAME: &'static str = "export_save_default";
    type Input = ExportSaveDefaultInput;
    type Output = ExportSaveDefaultOutput;

    /// The default name (FR-EXP-14 with the panel's format on the canvas; FR-EXP-32's names for
    /// a file, a pack image its own stem) in the last Save as folder, else the project's
    /// `exports/`. Reads no file content, so nothing is hydrated.
    fn run(core: &Core, input: ExportSaveDefaultInput) -> CoreResult<ExportSaveDefaultOutput> {
        let ctx = &core.ctx;
        let mut state = core.state();
        let last = state
            .settings
            .export
            .last_save_dir
            .clone()
            .filter(|d| d.is_dir());
        let (name, project_dir) = match &input.source {
            None => {
                let crate::core::State { project, exp, .. } = &mut *state;
                let p = project.as_ref().ok_or_else(no_project)?;
                exp.bind(p);
                let s = super::current_settings(exp, p);
                let format = input.format.unwrap_or(s.format);
                (
                    settings::file_name(&folder_name(&p.dir), s.target_px_per_square, format),
                    Some(p.dir.clone()),
                )
            }
            Some(src) => {
                let source = library::resolve_in_library(ctx, &state.settings, Path::new(src))?;
                let format = input
                    .format
                    .or_else(|| settings::format_of_extension(&source))
                    .unwrap_or(ExportFormat::Png);
                match owner_of(ctx, &state.settings, &source) {
                    Owner::Project { dir, rel } => (
                        settings::file_export_name(&folder_name(&dir), &rel, format),
                        Some(dir),
                    ),
                    Owner::Other => (
                        settings::plain_export_name(&folder_name(&source), format),
                        None,
                    ),
                }
            }
        };
        let exports = project_dir.map(|d| {
            let e = d.join(crate::manifest::EXPORTS_DIR);
            if e.is_dir() { e } else { d }
        });
        let path = match last.or(exports) {
            Some(dir) => dir.join(name),
            None => PathBuf::from(name),
        };
        Ok(ExportSaveDefaultOutput { path })
    }
}

pub struct ExportCancel;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportCancelInput {
    pub export_id: Ulid,
}

impl Validate for ExportCancelInput {}

impl Command for ExportCancel {
    const NAME: &'static str = "export_cancel";
    type Input = ExportCancelInput;
    type Output = Ok_;

    /// FR-EXP-20: `export:canceled` follows; `unknown_id` once the export has finished.
    fn run(core: &Core, input: ExportCancelInput) -> CoreResult<Ok_> {
        if core.state().exp.cancel(input.export_id) {
            Ok(Ok_ {})
        } else {
            Err(
                CoreError::new(ErrorCode::UnknownId, "no such export (already finished?)")
                    .with_field("export_id"),
            )
        }
    }
}

/// Runs the settings debounce now (tests with a fake clock; the app's timer calls the same).
pub fn flush_settings_if_due(core: &Core) -> CoreResult<bool> {
    flush_pending(&core.ctx, &mut core.state(), false)
}

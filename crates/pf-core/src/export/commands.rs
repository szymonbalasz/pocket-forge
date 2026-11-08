//! EXP core commands (export spec §9, ruling R43): inputs nested `{settings}` /
//! `{settings, replace}`; `export_plan.path` is project-relative, `export:done.path` absolute.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ulid::Ulid;

use super::run::{self, ExportJob};
use super::settings::{self, ExportSettings, Plan};
use super::{Running, flush_pending, set_settings, settings_output, spawn_debounce};
use crate::commands::{NoInput, Ok_, no_project};
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::job::JobState;
use crate::project::{folder_name, store};

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
        settings::check_destination(&p.dir)?;
        let m = &p.manifest;
        settings::plan(
            &input.settings,
            &folder_name(&p.dir),
            &p.dir,
            m.original.width,
            m.original.height,
        )
    }
}

pub struct ExportStart;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportStartInput {
    pub settings: ExportSettings,
    #[serde(default)]
    pub replace: bool,
}

impl Validate for ExportStartInput {}

#[derive(Debug, Serialize)]
pub struct ExportStartOutput {
    pub export_id: Ulid,
}

fn busy(message: &str) -> CoreError {
    CoreError::new(ErrorCode::Busy, message)
}

impl Command for ExportStart {
    const NAME: &'static str = "export_start";
    type Input = ExportStartInput;
    type Output = ExportStartOutput;

    /// Checks (read-only, busy, original, settings, format, destination), commits the settings
    /// (R6) and starts the background export. Nothing is written when a check fails.
    fn run(core: &Core, input: ExportStartInput) -> CoreResult<ExportStartOutput> {
        let ctx = &core.ctx;
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let crate::core::State { project, exp, .. } = &mut *state;
        let p = project.as_mut().ok_or_else(no_project)?;
        exp.bind(p);
        // FR-EXP-29, R12: fail fast, write nothing.
        p.require_writable()?;
        let project_id = p.manifest.project_id;
        if exp.is_running(project_id) {
            return Err(busy("an export of this project is already running"));
        }
        // R31: a region job fetching its candidates shares the decoded original.
        if p.jobs.values().any(|j| j.state == JobState::Succeeded) {
            return Err(busy("a region job is fetching its candidates"));
        }
        // FR-EXP-28, R11.
        let original = p.decoded.get(ctx)?;
        let (w, h) = (p.manifest.original.width, p.manifest.original.height);
        let s = input.settings;
        let sizing = s.sizing(w, h)?;
        if let Some(reason) = settings::unavailable(s.format, &sizing) {
            return Err(
                CoreError::new(ErrorCode::TooLarge, reason).with_detail(json!({
                    "format": s.format,
                    "limit": pf_imaging::WEBP_MAX_SIDE,
                })),
            );
        }
        settings::check_destination(&p.dir)?;
        let rel_path = settings::rel_path(&folder_name(&p.dir), s.target_px_per_square, s.format);
        if !input.replace
            && let Some(existing) = settings::existing(&p.dir.join(&rel_path))
        {
            return Err(
                CoreError::new(ErrorCode::Exists, format!("{rel_path} exists"))
                    .with_detail(existing),
            );
        }
        // R6, FR-EXP-25: the settings commit; PRJ's read_only / locked / conflict stop here.
        if ExportSettings::from_record(p.manifest.export.as_ref()).as_ref() != Some(&s) {
            store::put_export(ctx, install_id, p, s.record_fields())?;
        }
        exp.session = Some(s.clone());
        exp.pending_since = None;

        let id = ctx.new_id();
        let cancel = Arc::new(AtomicBool::new(false));
        exp.running.push(Running {
            id,
            project_id,
            cancel: cancel.clone(),
        });
        let job = ExportJob {
            id,
            project_id,
            project_dir: p.dir.clone(),
            rel_path,
            format: s.format,
            width: sizing.width,
            height: sizing.height,
            rescale: sizing.rescales(),
            original,
            layers: p.manifest.layers.clone(),
            head_revision: p.manifest.head_revision,
            cancel,
            seams: exp.seams.clone(),
        };
        drop(state);
        if core.ctx.inline_background {
            run::run(ctx, &core.state_handle(), job);
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

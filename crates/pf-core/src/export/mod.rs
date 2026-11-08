//! Flattened export (export spec, slice 3): session settings, the plan, and the background
//! export with progress, cancel and the `last` record.
//!
//! - Settings are session state of the open project ([`ExpState`]). They're committed to the
//!   manifest `export` record by PRJ's debounced draft save (≥ 2 s after the last change, R30,
//!   FR-PRJ-19): `project_put_draft` carries them in its commit, a core debounce commits them
//!   through `put_export` when no draft save comes, and close flushes them ([`on_project_close`]).
//! - `export_start` commits them through `put_export` (R6), snapshots the current layer stack and
//!   runs the export in the background ([`run`]); after the rename a second `put_export` commits
//!   `last` (FR-EXP-27).
//!
//! Commands are in [`commands`] and re-exported from [`crate::commands`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use ulid::Ulid;

use crate::context::Context;
use crate::core::State;
use crate::error::{CoreResult, ErrorCode};
use crate::project::{OpenProject, store};

pub mod commands;
mod run;
pub mod settings;
#[cfg(test)]
mod tests;

pub use run::ExportSeams;
pub use settings::{ExportFormat, ExportSettings, Grid, GridMode, Plan};

/// Settings are committed this long after the last change (R30, FR-PRJ-19).
pub const DRAFT_SAVE_AFTER: Duration = Duration::from_secs(2);

/// A running export (FR-EXP-21: one per asset project).
#[derive(Debug, Clone)]
pub(crate) struct Running {
    pub id: Ulid,
    pub project_id: Ulid,
    pub cancel: Arc<AtomicBool>,
}

/// EXP's memory-only state behind the core lock.
#[derive(Debug, Default)]
pub struct ExpState {
    /// The project the session settings belong to.
    project_id: Option<Ulid>,
    /// Settings typed this session; `None` = the manifest record's.
    session: Option<ExportSettings>,
    /// When the session settings last changed while not yet committed (R30).
    pending_since: Option<crate::time::Timestamp>,
    /// The `(source, target)` last sent with `export:settings-changed` (FR-EXP-26).
    last_emitted: Option<(Option<f64>, Option<i64>)>,
    pub(crate) running: Vec<Running>,
    /// A debounce thread is waiting to commit the pending settings.
    debounce_active: bool,
    /// Test seams (encoder, rename, lock retry, cancel point).
    pub seams: Arc<ExportSeams>,
}

impl ExpState {
    /// Follows the open project: a different project starts a fresh session.
    fn bind(&mut self, p: &OpenProject) {
        let id = p.manifest.project_id;
        if self.project_id != Some(id) {
            self.project_id = Some(id);
            self.session = None;
            self.pending_since = None;
            let (w, h) = size(p);
            self.last_emitted = Some(current_settings(self, p).px_per_square(w, h));
        }
    }

    fn reset(&mut self) {
        self.project_id = None;
        self.session = None;
        self.pending_since = None;
        self.last_emitted = None;
    }

    /// Whether an export of this project is running (FR-EXP-21).
    pub(crate) fn is_running(&self, project_id: Ulid) -> bool {
        self.running.iter().any(|r| r.project_id == project_id)
    }

    pub(crate) fn finish(&mut self, id: Ulid) {
        self.running.retain(|r| r.id != id);
    }

    /// Asks a running export to stop; false if there's none with this id (FR-EXP-20).
    pub(crate) fn cancel(&self, id: Ulid) -> bool {
        self.running
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.cancel.store(true, Ordering::SeqCst))
            .is_some()
    }
}

fn size(p: &OpenProject) -> (u32, u32) {
    (p.manifest.original.width, p.manifest.original.height)
}

/// The settings in effect: this session's, else the manifest record's, else the defaults.
fn current_settings(exp: &ExpState, p: &OpenProject) -> ExportSettings {
    exp.session
        .clone()
        .or_else(|| ExportSettings::from_record(p.manifest.export.as_ref()))
        .unwrap_or_else(ExportSettings::defaults)
}

/// What `export_get_settings` / `export_set_settings` return: the settings (when any were ever
/// chosen) plus `source_px_per_square` and `target_px_per_square` (FR-EXP-26).
pub(crate) fn settings_output(state: &mut State) -> CoreResult<Value> {
    let State { project, exp, .. } = state;
    let p = project.as_ref().ok_or_else(crate::commands::no_project)?;
    exp.bind(p);
    let chosen = exp
        .session
        .clone()
        .or_else(|| ExportSettings::from_record(p.manifest.export.as_ref()));
    let (w, h) = size(p);
    let (source, target) = chosen
        .as_ref()
        .map_or((None, None), |s| s.px_per_square(w, h));
    let mut out = match chosen.map(|s| s.record_fields()) {
        Some(m) => m,
        None => serde_json::Map::new(),
    };
    out.insert("source_px_per_square".into(), json!(source));
    out.insert("target_px_per_square".into(), json!(target));
    Ok(Value::Object(out))
}

/// `export_set_settings`: validates against the open image, keeps them as session state, marks
/// them for the next draft save and emits `export:settings-changed` when the two px-per-square
/// values change. Never commits (FR-EXP-25).
pub(crate) fn set_settings(
    ctx: &Context,
    state: &mut State,
    settings: ExportSettings,
) -> CoreResult<bool> {
    let State { project, exp, .. } = state;
    let p = project.as_ref().ok_or_else(crate::commands::no_project)?;
    exp.bind(p);
    let (w, h) = size(p);
    settings.sizing(w, h)?;
    let committed = ExportSettings::from_record(p.manifest.export.as_ref());
    let changed = current_settings(exp, p) != settings;
    exp.session = Some(settings.clone());
    if committed.as_ref() == Some(&settings) {
        exp.pending_since = None;
    } else if changed || exp.pending_since.is_none() {
        exp.pending_since = Some(ctx.now());
    }
    let values = settings.px_per_square(w, h);
    if exp.last_emitted != Some(values) {
        exp.last_emitted = Some(values);
        ctx.emit(
            "export:settings-changed",
            json!({ "source": values.0, "target": values.1 }),
        );
    }
    let needs_timer = exp.pending_since.is_some() && !exp.debounce_active;
    if needs_timer {
        exp.debounce_active = true;
    }
    Ok(needs_timer)
}

/// The pending settings' manifest fields, for the draft save's commit (R30).
pub(crate) fn pending_fields(
    exp: &mut ExpState,
    p: &OpenProject,
) -> Option<serde_json::Map<String, Value>> {
    exp.bind(p);
    exp.pending_since?;
    exp.session.as_ref().map(ExportSettings::record_fields)
}

/// The draft save committed the pending settings.
pub(crate) fn clear_pending(exp: &mut ExpState) {
    exp.pending_since = None;
}

/// Commits the pending settings through `put_export` if they're due (`force`: now). Returns
/// whether a commit happened. A read-only project keeps them in the session only.
pub(crate) fn flush_pending(ctx: &Context, state: &mut State, force: bool) -> CoreResult<bool> {
    let install_id = state.settings.install_id;
    let State { project, exp, .. } = state;
    let Some(p) = project.as_mut() else {
        return Ok(false);
    };
    exp.bind(p);
    let (Some(since), Some(settings)) = (exp.pending_since, exp.session.clone()) else {
        return Ok(false);
    };
    let due = since.plus(DRAFT_SAVE_AFTER) <= ctx.now();
    if !(force || due) || p.read_only.is_some() {
        return Ok(false);
    }
    store::put_export(ctx, install_id, p, settings.record_fields())?;
    exp.pending_since = None;
    Ok(true)
}

/// Waits out the debounce in the background and commits the pending settings once they've been
/// unchanged for [`DRAFT_SAVE_AFTER`] (the app; tests call [`flush_pending`] with a fake clock).
pub(crate) fn spawn_debounce(ctx: Arc<Context>, state: Arc<Mutex<State>>) {
    let run = move || {
        loop {
            std::thread::sleep(DRAFT_SAVE_AFTER / 4);
            let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
            if s.exp.pending_since.is_none() || s.project.is_none() {
                s.exp.debounce_active = false;
                return;
            }
            match flush_pending(&ctx, &mut s, false) {
                Ok(true) => {
                    s.exp.debounce_active = false;
                    return;
                }
                Ok(false) if s.project.as_ref().is_some_and(|p| p.read_only.is_some()) => {
                    s.exp.debounce_active = false;
                    return;
                }
                Ok(false) => {}
                Err(e) => {
                    // PRJ reports save problems; the next change, export or close retries.
                    tracing::warn!(error = %e, "export settings not saved");
                    s.exp.debounce_active = false;
                    return;
                }
            }
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("pf-export-settings".into())
        .spawn(run)
    {
        tracing::warn!(error = %e, "can't start the export settings save");
    }
}

/// PRJ's resolve hook for EXP (walk 2 N1): Keep other drops this session's settings, so the kept
/// version's record is in effect (and is what the next draft save carries); Keep this commits
/// any the conflict held back.
pub(crate) fn on_conflict_resolved(
    ctx: &Context,
    state: &mut State,
    keep: crate::project::conflict::Keep,
) {
    if keep == crate::project::conflict::Keep::This {
        if let Err(e) = flush_pending(ctx, state, true) {
            tracing::warn!(error = %e, "export settings not saved after the resolve");
        }
        return;
    }
    let State { project, exp, .. } = state;
    let Some(p) = project.as_ref() else {
        return;
    };
    exp.bind(p);
    exp.session = None;
    exp.pending_since = None;
    let (w, h) = size(p);
    let values = current_settings(exp, p).px_per_square(w, h);
    if exp.last_emitted != Some(values) {
        exp.last_emitted = Some(values);
        ctx.emit(
            "export:settings-changed",
            json!({ "source": values.0, "target": values.1 }),
        );
    }
}

/// PRJ's `unsaved` check for EXP (FR-PRJ-21): commits pending settings now; true if a `locked`
/// file kept them from saving, so closing asks first. Any other failure is logged and dropped,
/// as PRJ drops a pending draft that fails for another reason.
pub(crate) fn unsaved_on_close(ctx: &Context, state: &mut State) -> bool {
    match flush_pending(ctx, state, true) {
        Ok(_) => false,
        Err(e) if e.code == ErrorCode::Locked => true,
        Err(e) => {
            tracing::warn!(error = %e, "export settings not saved on close");
            state.exp.pending_since = None;
            false
        }
    }
}

/// PRJ's close hook for EXP: commits pending settings (FR-PRJ-19 "and on close"), cancels a
/// running export of the project (OQ-EXP-09: the UI already asked) and ends the session.
pub(crate) fn on_project_close(ctx: &Context, state: &mut State) {
    if let Some(id) = state.project.as_ref().map(|p| p.manifest.project_id) {
        if let Err(e) = flush_pending(ctx, state, true) {
            tracing::warn!(error = %e, "export settings not saved on close");
        }
        for r in state.exp.running.iter().filter(|r| r.project_id == id) {
            r.cancel.store(true, Ordering::SeqCst);
        }
    }
    state.exp.reset();
}

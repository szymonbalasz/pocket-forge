//! PRJ retention and conflict commands (asset-project §9, slices 5 and 6). Registered in
//! [`crate::commands::REGISTRY`].

use serde::Deserialize;
use ulid::Ulid;

use super::conflict::{self, ConflictView, Keep};
use super::retention::{self, DiskUsage, TrashRow};
use crate::commands::{NoInput, Ok_};
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::manifest::RevisionEntry;

fn no_project() -> CoreError {
    CoreError::new(ErrorCode::NoProject, "no asset project is open")
}

/// The 10 s retry tick (FR-PRJ-21): retries a pending draft and mask (R108) of the open project. (RGN's runner
/// replays the spool, R48.) The app calls it every [`super::SAVE_RETRY_EVERY`]; it is a no-op when
/// nothing is pending.
pub fn retry_pending_saves(core: &Core) {
    let mut state = core.state();
    let install_id = state.settings.install_id;
    if let Some(p) = state.project.as_mut() {
        super::retry_pending(&core.ctx, install_id, p);
    }
}

// ---------------------------------------------------------------- slice 5: retention

pub struct ProjectTrashList;

impl Command for ProjectTrashList {
    const NAME: &'static str = "project_trash_list";
    type Input = NoInput;
    type Output = Vec<TrashRow>;

    /// BL-30: `folder_moved` once the folder is gone, not a list of 0 B items.
    fn run(core: &Core, _: NoInput) -> CoreResult<Vec<TrashRow>> {
        let mut state = core.state();
        let p = state.project.as_mut().ok_or_else(no_project)?;
        super::require_folder(p)?;
        Ok(retention::trash_list(p))
    }
}

pub struct ProjectTrashRestore;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashRestoreInput {
    pub item_id: Ulid,
}

impl Validate for TrashRestoreInput {}

impl Command for ProjectTrashRestore {
    const NAME: &'static str = "project_trash_restore";
    type Input = TrashRestoreInput;
    type Output = Ok_;

    /// FR-PRJ-31: the item's files back to their old paths.
    fn run(core: &Core, input: TrashRestoreInput) -> CoreResult<Ok_> {
        let mut state = core.state();
        let install_id = state.settings.install_id;
        let p = state.project.as_mut().ok_or_else(no_project)?;
        retention::trash_restore(&core.ctx, install_id, p, input.item_id)?;
        Ok(Ok_ {})
    }
}

pub struct ProjectDiskUsage;

impl Command for ProjectDiskUsage {
    const NAME: &'static str = "project_disk_usage";
    type Input = NoInput;
    type Output = DiskUsage;

    /// FR-PRJ-33: metadata only, never hydrates. BL-30: `folder_moved` once the folder is gone
    /// (the walk would find nothing and report 0 B).
    fn run(core: &Core, _: NoInput) -> CoreResult<DiskUsage> {
        let mut state = core.state();
        let p = state.project.as_mut().ok_or_else(no_project)?;
        super::require_folder(p)?;
        Ok(retention::disk_usage(p))
    }
}

// ---------------------------------------------------------------- slice 6: conflicts

pub struct ProjectConflictGet;

impl Command for ProjectConflictGet {
    const NAME: &'static str = "project_conflict_get";
    type Input = NoInput;
    type Output = ConflictView;

    /// FR-PRJ-36: both versions; `no_conflict` if there is none.
    fn run(core: &Core, _: NoInput) -> CoreResult<ConflictView> {
        let state = core.state();
        let install_id = state.settings.install_id;
        let p = state.project.as_ref().ok_or_else(no_project)?;
        conflict::conflict_get(p, install_id)
    }
}

pub struct ProjectResolveConflict;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveConflictInput {
    pub keep: Keep,
}

impl Validate for ResolveConflictInput {}

impl Command for ProjectResolveConflict {
    const NAME: &'static str = "project_resolve_conflict";
    type Input = ResolveConflictInput;
    type Output = RevisionEntry;

    /// FR-PRJ-36, #39: keep one version; the other becomes a `conflict` revision.
    fn run(core: &Core, input: ResolveConflictInput) -> CoreResult<RevisionEntry> {
        let rev = {
            let mut state = core.state();
            let install_id = state.settings.install_id;
            let p = state.project.as_mut().ok_or_else(no_project)?;
            let rev = conflict::resolve_conflict(&core.ctx, install_id, p, input.keep)?;
            crate::export::on_conflict_resolved(&core.ctx, &mut state, input.keep);
            rev
        };
        // R48: replay what the conflict held back at once (the runner emits its events).
        crate::region_edit::runner::tick(core, false);
        Ok(rev)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::context::Context;
    use crate::core::Core;
    use crate::error::ErrorCode;
    use crate::test_support::{T0, rename_dir};
    use crate::time::Timestamp;

    fn core_with_project() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(T0));
        ctx.library_override = Some(dir.path().join("lib"));
        let core = Core::new(ctx).unwrap();
        let src = dir.path().join("Crypt.png");
        std::fs::write(&src, crate::test_support::png_bytes(40, 30, 9)).unwrap();
        let out = core
            .dispatch(
                "project_create",
                json!({"section": "maps", "paths": [src.to_string_lossy()]}),
            )
            .unwrap();
        let path = out["items"][0]["path"].as_str().unwrap().to_owned();
        core.dispatch("project_open", json!({ "path": path }))
            .unwrap();
        (dir, core)
    }

    #[test]
    fn fr_prj_33_storage_commands_return_the_panel_shapes() {
        let (_d, core) = core_with_project();
        let u = core.dispatch("project_disk_usage", json!({})).unwrap();
        for k in [
            "original",
            "layers",
            "masks",
            "job_inputs",
            "results",
            "trash",
            "user_folders",
            "exports",
            "other",
        ] {
            assert!(u["bytes"][k].is_u64(), "{k}: {u}");
        }
        assert!(u["bytes"]["original"].as_u64().unwrap() > 0);
        assert_eq!(u["trash_count"], 0);
        assert!(u["next_purge"].is_null());
        assert_eq!(
            core.dispatch("project_trash_list", json!({})).unwrap(),
            json!([])
        );
        let e = core
            .dispatch(
                "project_trash_restore",
                json!({"item_id": ulid::Ulid::nil().to_string()}),
            )
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UnknownId);
        core.dispatch("project_close", json!({})).unwrap();
        assert_eq!(
            core.dispatch("project_disk_usage", json!({}))
                .unwrap_err()
                .code,
            ErrorCode::NoProject
        );
    }

    #[test]
    fn fr_prj_36_conflict_commands_report_no_conflict_and_resolve_a_copy() {
        let (_d, core) = core_with_project();
        let e = core
            .dispatch("project_conflict_get", json!({}))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NoConflict);
        let e = core
            .dispatch("project_resolve_conflict", json!({"keep": "mine"}))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidInput);
        let dir = core.state().project.as_ref().unwrap().dir.clone();
        std::fs::copy(dir.join("project.json"), dir.join("project (2).json")).unwrap();
        core.dispatch("project_open", json!({ "path": dir }))
            .unwrap();
        let v = core.dispatch("project_conflict_get", json!({})).unwrap();
        assert_eq!(v["source"], "copy");
        assert_eq!(v["other"]["file"], "project (2).json");
        assert_eq!(v["this"]["can_keep"], true);
        let rev = core
            .dispatch("project_resolve_conflict", json!({"keep": "other"}))
            .unwrap();
        assert_eq!(rev["reason"], "conflict");
        assert!(!dir.join("project (2).json").exists());
        let snap = core.dispatch("project_get", json!({})).unwrap();
        assert!(
            !snap["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w["code"] == "conflict")
        );
    }

    #[test]
    fn fr_prj_21_edge_close_after_the_folder_moved_asks_then_force_closes() {
        let (d, core) = core_with_project();
        let dir = core.state().project.as_ref().unwrap().dir.clone();
        rename_dir(&dir, &dir.with_file_name("Elsewhere"));
        let e = core
            .dispatch(
                "project_put_draft",
                json!({"draft": {"final_prompt": "kept"}}),
            )
            .unwrap_err();
        assert_eq!(
            (e.code, &e.detail["reason"]),
            (ErrorCode::NotFound, &json!("folder_moved"))
        );
        let snap = core.dispatch("project_get", json!({})).unwrap();
        assert!(
            snap["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w["code"] == "folder_moved")
        );
        let e = core.dispatch("project_close", json!({})).unwrap_err();
        assert_eq!(
            (e.code, &e.detail["reason"]),
            (ErrorCode::Unsaved, &json!("folder_moved"))
        );
        let root = d.path().to_string_lossy().into_owned();
        assert!(!e.message.contains(&root), "{}", e.message);
        assert!(core.state().project.is_some(), "still open");
        core.dispatch("project_close", json!({"force": true}))
            .unwrap();
        assert!(core.state().project.is_none());
    }

    #[test]
    fn bl_30_storage_refuses_with_folder_moved_instead_of_zero_bytes() {
        let (_d, core) = core_with_project();
        let dir = core.state().project.as_ref().unwrap().dir.clone();
        assert!(core.dispatch("project_disk_usage", json!({})).is_ok());
        rename_dir(&dir, &dir.with_file_name("Elsewhere"));
        for (cmd, input) in [
            ("project_disk_usage", json!({})),
            ("project_trash_list", json!({})),
            ("project_folder_list", json!({"folder": "working"})),
        ] {
            let e = core.dispatch(cmd, input).unwrap_err();
            assert_eq!(
                (e.code, &e.detail["reason"]),
                (ErrorCode::NotFound, &json!("folder_moved")),
                "{cmd}"
            );
        }
    }

    /// Walk 2 N1: during a conflict a refused draft makes Close ask (reason `conflict`), and
    /// Keep other drops this session's export settings for the kept version's.
    #[test]
    fn fr_prj_36_fr_prj_21_close_asks_during_a_conflict_and_keep_other_reloads_export_settings() {
        let (_d, core) = core_with_project();
        let dir = core.state().project.as_ref().unwrap().dir.clone();
        let settings = json!({"grid": {"mode": "cells", "cols": 4, "rows": 3}, "target_px_per_square": null, "format": "jpeg"});
        core.dispatch("export_set_settings", json!({ "settings": settings }))
            .unwrap();
        std::fs::copy(dir.join("project.json"), dir.join("project (2).json")).unwrap();
        let e = core
            .dispatch(
                "project_put_draft",
                json!({"draft": {"final_prompt": "typed during"}}),
            )
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict);
        let e = core.dispatch("project_close", json!({})).unwrap_err();
        assert_eq!(
            (e.code, &e.detail["reason"]),
            (ErrorCode::Unsaved, &json!("conflict"))
        );
        assert!(core.state().project.is_some(), "still open");

        core.dispatch("project_resolve_conflict", json!({"keep": "other"}))
            .unwrap();
        let s = core.dispatch("export_get_settings", json!({})).unwrap();
        assert!(
            s.get("format").is_none(),
            "the kept version has no settings: {s}"
        );
        let snap = core.dispatch("project_get", json!({})).unwrap();
        assert!(snap["draft"].is_null() || snap["draft"]["final_prompt"] != "typed during");
        core.dispatch("project_close", json!({})).unwrap();
        core.dispatch("project_open", json!({ "path": dir }))
            .unwrap();
        let s = core.dispatch("export_get_settings", json!({})).unwrap();
        assert!(s.get("format").is_none(), "nothing wrote them back: {s}");
    }

    #[test]
    fn fr_prj_21_close_retries_first_and_force_closes() {
        let (_d, core) = core_with_project();
        {
            let mut st = core.state();
            st.project.as_mut().unwrap().pending_draft = Some(crate::project::PendingDraft {
                draft: crate::project::DraftInput {
                    final_prompt: "kept".into(),
                    ..Default::default()
                },
                export: None,
            });
        }
        // Not locked any more: close retries, saves, and closes.
        core.dispatch("project_close", json!({})).unwrap();
        assert!(core.state().project.is_none());
        core.dispatch("project_close", json!({"force": true}))
            .unwrap();
    }
}

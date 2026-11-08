//! PRJ retention and conflict commands (asset-project §9, slices 5 and 6). Registered in
//! [`crate::commands::REGISTRY`].

use serde::Deserialize;
use ulid::Ulid;

use super::retention::{self, DiskUsage, TrashRow};
use crate::commands::{NoInput, Ok_};
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult, ErrorCode};

fn no_project() -> CoreError {
    CoreError::new(ErrorCode::NoProject, "no asset project is open")
}

/// The 10 s retry tick (FR-PRJ-21, 40): retries a pending draft and replays the spool of the
/// open project. The app calls it every [`super::SAVE_RETRY_EVERY`]; it is a no-op when
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

    fn run(core: &Core, _: NoInput) -> CoreResult<Vec<TrashRow>> {
        let state = core.state();
        let p = state.project.as_ref().ok_or_else(no_project)?;
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

    /// FR-PRJ-33: metadata only, never hydrates.
    fn run(core: &Core, _: NoInput) -> CoreResult<DiskUsage> {
        let state = core.state();
        let p = state.project.as_ref().ok_or_else(no_project)?;
        Ok(retention::disk_usage(p))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::context::Context;
    use crate::core::Core;
    use crate::error::ErrorCode;
    use crate::test_support::T0;
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

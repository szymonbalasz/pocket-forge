//! The Tauri command registry. **Append-only** (AGENTS §9): add one line per new core command
//! to the list at the bottom. Each entry adapts a `pf-core` command 1:1; no logic lives here.
//!
//! Every command takes a single `input` argument, so the UI calls
//! `invoke(name, { input })` (see `app/src/api/`).

use pf_core::Command;
use pf_core::commands as c;

macro_rules! tauri_commands {
    ($($name:ident => $cmd:ty),* $(,)?) => {
        $(
            #[tauri::command(async)]
            pub fn $name(
                core: tauri::State<'_, pf_core::Core>,
                input: <$cmd as Command>::Input,
            ) -> Result<<$cmd as Command>::Output, pf_core::CoreError> {
                debug_assert_eq!(stringify!($name), <$cmd as Command>::NAME);
                core.run::<$cmd>(input)
            }
        )*

        /// The invoke handler with every command above, plus the app-level ones.
        pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
            tauri::generate_handler![$($name,)* crate::app_ready]
        }

        /// Names of the registered core commands (checked against `pf_core::commands::REGISTRY`).
        #[cfg(test)]
        pub const NAMES: &[&str] = &[$(stringify!($name)),*];
    };
}

tauri_commands! {
    app_info => c::AppInfo,
    library_get_config => c::LibraryGetConfig,
    library_set_roots => c::LibrarySetRoots,
    project_open => c::ProjectOpen,
    project_close => c::ProjectClose,
    project_get => c::ProjectGet,
    project_put_draft => c::ProjectPutDraft,
    project_save_revision => c::ProjectSaveRevision,
    project_list_revisions => c::ProjectListRevisions,
    edt_image_open => c::EdtImageOpen,
    provider_status => c::ProviderStatus,
    edt_tiles_open => c::EdtTilesOpen,
    provider_credentials_set => c::ProviderCredentialsSet,
    provider_credentials_clear => c::ProviderCredentialsClear,
    provider_test_connection => c::ProviderTestConnection,
    provider_settings_get => c::ProviderSettingsGet,
    provider_settings_set => c::ProviderSettingsSet,
    provider_list_models => c::ProviderListModels,
    provider_get_capabilities => c::ProviderGetCapabilities,
    provider_balance => c::ProviderBalance,
    provider_fake_set_case => c::ProviderFakeSetCase,
    rgn_region_fit => c::RgnRegionFit,
    rgn_controls_evaluate => c::RgnControlsEvaluate,
    rgn_job_estimate => c::RgnJobEstimate,
    rgn_spend_get => c::RgnSpendGet,
    rgn_spend_set => c::RgnSpendSet,
    library_list_projects => c::LibraryListProjects,
    project_create => c::ProjectCreate,
    project_create_cancel => c::ProjectCreateCancel,
    project_create_from_clipboard => c::ProjectCreateFromClipboard,
    project_rename => c::ProjectRename,
    project_restore_revision => c::ProjectRestoreRevision,
    project_relink_original => c::ProjectRelinkOriginal,
    project_prefetch_original => c::ProjectPrefetchOriginal,
    project_folder_list => c::ProjectFolderList,
    project_folder_add => c::ProjectFolderAdd,
    rgn_job_submit => c::RgnJobSubmit,
    rgn_job_cancel => c::RgnJobCancel,
    rgn_job_retry_draft => c::RgnJobRetryDraft,
    rgn_job_dismiss => c::RgnJobDismiss,
    rgn_candidate_preview => c::RgnCandidatePreview,
    rgn_candidate_accept => c::RgnCandidateAccept,
    rgn_candidate_star => c::RgnCandidateStar,
    rgn_candidate_discard => c::RgnCandidateDiscard,
    rgn_patch_retune => c::RgnPatchRetune,
    rgn_iterations_list => c::RgnIterationsList,
    ref_list_packs => c::RefListPacks,
    ref_pack_get => c::RefPackGet,
    ref_pack_create => c::RefPackCreate,
    ref_pack_rename => c::RefPackRename,
    ref_pack_update => c::RefPackUpdate,
    ref_pack_add_images => c::RefPackAddImages,
    ref_pack_remove_images => c::RefPackRemoveImages,
    ref_pack_delete => c::RefPackDelete,
    ref_trash_list => c::RefTrashList,
    ref_trash_restore => c::RefTrashRestore,
    ref_resolve => c::RefResolve,
    project_trash_list => c::ProjectTrashList,
    project_trash_restore => c::ProjectTrashRestore,
    project_disk_usage => c::ProjectDiskUsage,
    project_conflict_get => c::ProjectConflictGet,
    project_resolve_conflict => c::ProjectResolveConflict,
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_core_command_is_registered_with_its_own_name() {
        let core: Vec<&str> = pf_core::commands::REGISTRY
            .iter()
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(super::NAMES, core.as_slice());
    }
}

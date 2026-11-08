//! Core commands. Each is a unit struct implementing [`Command`]; inputs are validated at the
//! boundary. **Append-only registry** at the bottom: add new commands, never rename one.
//!
//! Every command takes one `input` object and returns JSON. The Tauri layer
//! (`app/src-tauri/src/commands.rs`) adapts each one; the UI calls them through `app/src/api/`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::core::{Command, Core, Validate, dispatch_json};
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::import::{self, ImportItem};
use crate::library::{self, LibraryConfig};
use crate::manifest::RevisionEntry;
use crate::project::{self, DraftInput, ProjectSnapshot};
use crate::recent::{self, RecentEntry};
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

fn no_project() -> CoreError {
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
    #[serde(default)]
    pub import_dest: Option<String>,
}

impl Validate for LibrarySetRootsInput {
    fn validate(&self) -> CoreResult<()> {
        if self.roots.len() > 64 {
            return Err(CoreError::invalid("roots", "too many folders"));
        }
        for (i, r) in self.roots.iter().enumerate() {
            check_path(&format!("roots[{i}]"), r)?;
        }
        if let Some(d) = &self.import_dest {
            check_path("import_dest", d)?;
        }
        Ok(())
    }
}

impl Command for LibrarySetRoots {
    const NAME: &'static str = "library_set_roots";
    type Input = LibrarySetRootsInput;
    type Output = LibraryConfig;

    fn run(core: &Core, input: Self::Input) -> CoreResult<LibraryConfig> {
        let roots: Vec<PathBuf> = input.roots.iter().map(PathBuf::from).collect();
        library::validate_roots(&core.ctx, &roots)?;
        let import_dest = input.import_dest.map(PathBuf::from);
        if let Some(dest) = &import_dest
            && !roots
                .iter()
                .any(|r| library::normalise(dest).starts_with(library::normalise(r)))
        {
            return Err(CoreError::invalid(
                "import_dest",
                "the import destination must be inside a library folder",
            ));
        }
        let mut state = core.state();
        let mut next = state.settings.clone();
        next.library.roots = roots;
        next.library.import_dest = import_dest;
        settings::save(&core.ctx, &next)?;
        state.settings = next;
        Ok(library::config(&core.ctx, &state.settings))
    }
}

// ---------------------------------------------------------------- import

pub struct ImportPaths;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPathsInput {
    pub paths: Vec<String>,
}

impl Validate for ImportPathsInput {
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
pub struct ImportPathsOutput {
    pub items: Vec<ImportItem>,
}

impl Command for ImportPaths {
    const NAME: &'static str = "import_paths";
    type Input = ImportPathsInput;
    type Output = ImportPathsOutput;

    fn run(core: &Core, input: Self::Input) -> CoreResult<ImportPathsOutput> {
        let paths: Vec<PathBuf> = input.paths.iter().map(PathBuf::from).collect();
        let settings = core.state().settings.clone();
        let items = import::import_paths(&core.ctx, &settings, &paths)?;
        core.ctx
            .emit("import:done", serde_json::json!({ "total": items.len() }));
        Ok(ImportPathsOutput { items })
    }
}

// ---------------------------------------------------------------- project

pub struct ProjectOpen;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectOpenInput {
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
        let p = project::open(&core.ctx, &state.settings, &PathBuf::from(&input.path))?;
        let snap = p.snapshot();
        if let Err(e) = recent::touch(&core.ctx, &p.image_path, &snap.name) {
            tracing::warn!(error = %e, "could not update recent.json");
        }
        core.served().reset(Some(snap.project_id));
        state.project = Some(p);
        Ok(snap)
    }
}

pub struct ProjectClose;

impl Command for ProjectClose {
    const NAME: &'static str = "project_close";
    type Input = NoInput;
    type Output = Ok_;

    fn run(core: &Core, _: NoInput) -> CoreResult<Ok_> {
        let mut state = core.state();
        state.project = None;
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
    pub draft: DraftInput,
}

impl Validate for ProjectPutDraftInput {
    fn validate(&self) -> CoreResult<()> {
        let d = &self.draft;
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
        let p = state.project.as_mut().ok_or_else(no_project)?;
        let seq = project::put_draft(&core.ctx, install_id, p, input.draft)?;
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

pub struct RecentList;

impl Command for RecentList {
    const NAME: &'static str = "recent_list";
    type Input = NoInput;
    type Output = Vec<RecentEntry>;

    fn run(core: &Core, _: NoInput) -> CoreResult<Vec<RecentEntry>> {
        Ok(recent::load(&core.ctx))
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

// ---------------------------------------------------------------- region edit

/// Region-edit commands live in their own module (region-edit spec §9).
pub use crate::region_edit::commands::{
    RgnControlsEvaluate, RgnJobEstimate, RgnRegionFit, RgnSpendGet, RgnSpendSet,
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
    ImportPaths,
    ProjectOpen,
    ProjectClose,
    ProjectGet,
    ProjectPutDraft,
    ProjectSaveRevision,
    ProjectListRevisions,
    RecentList,
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
            .dispatch("import_paths", json!({"paths": "x"}))
            .unwrap_err();
        assert_eq!(
            (e.code, e.field.as_deref()),
            (ErrorCode::InvalidInput, Some("input"))
        );
        let e = core
            .dispatch("import_paths", json!({"paths": [""]}))
            .unwrap_err();
        assert_eq!(e.field.as_deref(), Some("paths[0]"));
        let e = core
            .dispatch("project_save_revision", json!({}))
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::NoProject);
        let e = core.dispatch("app_info", json!({"extra": 1})).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn import_open_image_serve_save_reopen_end_to_end() {
        let (dir, core) = core();
        let src = dir.path().join("outside.png");
        std::fs::write(&src, png_bytes(64, 48)).unwrap();

        let out = core
            .dispatch("import_paths", json!({"paths": [src.to_string_lossy()]}))
            .unwrap();
        assert_eq!(out["items"][0]["status"], "imported");
        let path = out["items"][0]["path"].as_str().unwrap().to_owned();

        let snap = core
            .dispatch("project_open", json!({"path": path}))
            .unwrap();
        assert_eq!(snap["saved"], false);
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

        let rev = core.dispatch("project_save_revision", json!({})).unwrap();
        assert_eq!(rev["reason"], "create");
        core.dispatch("project_close", json!({})).unwrap();
        assert_eq!(
            core.serve_scheme(scheme_path).status,
            404,
            "keys unregistered on close"
        );

        let recent = core.dispatch("recent_list", json!({})).unwrap();
        assert_eq!(recent[0]["path"].as_str().unwrap(), path);
        let again = core
            .dispatch("project_open", json!({"path": path}))
            .unwrap();
        assert_eq!(again["saved"], true);
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

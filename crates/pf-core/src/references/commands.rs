//! Reference-pack commands (REF §9). Inputs are validated at the boundary (FR-REF-29); a failed
//! command leaves packs and the project unchanged. Pack commands never take the state lock for
//! longer than it takes to copy the settings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pf_provider::ReferenceRole;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ulid::Ulid;

use super::packs::{self, AddResult, CreateOutput, PackCard, PackView};
use super::resolve::{self, DraftReferences, Resolved};
use super::trash::{self, TrashEntry};
use super::{MAX_NOTES_CHARS, require_pack};
use crate::commands::NoInput;
use crate::core::{Command, Core, Validate};
use crate::error::{CoreError, CoreResult};
use crate::settings::Settings;
use crate::tiles;

const MAX_PATH_CHARS: usize = 4096;
const MAX_PATHS: usize = 10_000;
const MAX_NAME_INPUT_CHARS: usize = 255;

fn check_path(field: &str, p: &str) -> CoreResult<()> {
    if p.trim().is_empty() {
        return Err(CoreError::invalid(field, "the path is empty"));
    }
    if p.chars().count() > MAX_PATH_CHARS || p.contains('\0') {
        return Err(CoreError::invalid(field, "the path is not valid"));
    }
    Ok(())
}

fn check_paths(field: &str, paths: &[String]) -> CoreResult<()> {
    if paths.len() > MAX_PATHS {
        return Err(CoreError::invalid(field, "too many files"));
    }
    for (i, p) in paths.iter().enumerate() {
        check_path(&format!("{field}[{i}]"), p)?;
    }
    Ok(())
}

fn check_name_input(name: &str) -> CoreResult<()> {
    if name.is_empty() || name.chars().count() > MAX_NAME_INPUT_CHARS || name.contains('\0') {
        return Err(CoreError::invalid("name", "the name is empty or too long"));
    }
    Ok(())
}

/// A file name inside a pack: one path segment.
fn check_file_name(field: &str, f: &str) -> CoreResult<()> {
    if f.is_empty()
        || f.chars().count() > MAX_NAME_INPUT_CHARS
        || f.contains(['/', '\\', '\0'])
        || f == "."
        || f == ".."
    {
        return Err(CoreError::invalid(field, "not a file name in the pack"));
    }
    Ok(())
}

fn settings(core: &Core) -> Settings {
    core.state().settings.clone()
}

fn changed(core: &Core) {
    core.ctx.emit("ref:packs-changed", json!({}));
}

/// Image thumbnails in the pack view and the References block: ≤ 256 px from EDT's `library`
/// key space (FR-REF-14, R61). The packs-home mosaic keeps the 512 px library key.
pub const IMAGE_THUMB_MAX_SIDE: u32 = 256;

fn image_thumb(core: &Core, path: &Path) -> Option<String> {
    tiles::register_library_image_sized(core, path, IMAGE_THUMB_MAX_SIDE)
}

fn with_urls(core: &Core, mut v: PackView) -> PackView {
    for img in &mut v.images {
        img.thumb_url = image_thumb(core, &img.path);
    }
    v
}

fn to_paths(v: &[String]) -> Vec<PathBuf> {
    v.iter().map(PathBuf::from).collect()
}

/// `{pack_id}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackIdInput {
    pub pack_id: Ulid,
}

impl Validate for PackIdInput {}

/// `{item_id}`: the References trash item a removal made.
#[derive(Debug, Serialize)]
pub struct TrashItemOutput {
    pub item_id: Ulid,
}

// ---------------------------------------------------------------- ref_list_packs

pub struct RefListPacks;

impl Command for RefListPacks {
    const NAME: &'static str = "ref_list_packs";
    type Input = NoInput;
    type Output = Vec<PackCard>;

    /// FR-REF-04 … 06, 13, 28.
    fn run(core: &Core, _: NoInput) -> CoreResult<Vec<PackCard>> {
        let mut cards = packs::list_packs(&core.ctx, &settings(core));
        for c in &mut cards {
            c.thumb_url = c
                .thumb
                .as_deref()
                .and_then(|t| tiles::register_library_image(core, t));
        }
        Ok(cards)
    }
}

// ---------------------------------------------------------------- ref_pack_get

pub struct RefPackGet;

impl Command for RefPackGet {
    const NAME: &'static str = "ref_pack_get";
    type Input = PackIdInput;
    type Output = PackView;

    fn run(core: &Core, input: PackIdInput) -> CoreResult<PackView> {
        let loc = require_pack(&core.ctx, &settings(core), input.pack_id)?;
        Ok(with_urls(core, packs::view(&core.ctx, &loc)))
    }
}

// ---------------------------------------------------------------- ref_pack_create

pub struct RefPackCreate;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefPackCreateInput {
    pub name: String,
    #[serde(default)]
    pub paths: Vec<String>,
    /// A dropped folder: a taken name gets ` (2)` instead of `name_taken` (FR-REF-07).
    #[serde(default)]
    pub suffix_if_taken: bool,
}

impl Validate for RefPackCreateInput {
    fn validate(&self) -> CoreResult<()> {
        check_name_input(&self.name)?;
        check_paths("paths", &self.paths)
    }
}

impl Command for RefPackCreate {
    const NAME: &'static str = "ref_pack_create";
    type Input = RefPackCreateInput;
    type Output = CreateOutput;

    fn run(core: &Core, input: RefPackCreateInput) -> CoreResult<CreateOutput> {
        let mut out = packs::create(
            &core.ctx,
            &settings(core),
            &input.name,
            &to_paths(&input.paths),
            input.suffix_if_taken,
        )?;
        out.pack = with_urls(core, out.pack);
        changed(core);
        Ok(out)
    }
}

// ---------------------------------------------------------------- ref_pack_rename

pub struct RefPackRename;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefPackRenameInput {
    pub pack_id: Ulid,
    pub name: String,
}

impl Validate for RefPackRenameInput {
    fn validate(&self) -> CoreResult<()> {
        check_name_input(&self.name)
    }
}

impl Command for RefPackRename {
    const NAME: &'static str = "ref_pack_rename";
    type Input = RefPackRenameInput;
    type Output = PackView;

    /// FR-REF-10.
    fn run(core: &Core, input: RefPackRenameInput) -> CoreResult<PackView> {
        let loc = require_pack(&core.ctx, &settings(core), input.pack_id)?;
        let loc = packs::rename(&loc, &input.name)?;
        changed(core);
        Ok(with_urls(core, packs::view(&core.ctx, &loc)))
    }
}

// ---------------------------------------------------------------- ref_pack_update

pub struct RefPackUpdate;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefPackUpdateInput {
    pub pack_id: Ulid,
    #[serde(default)]
    pub notes: Option<String>,
    /// File name → role, or `null` to clear it.
    #[serde(default)]
    pub roles: BTreeMap<String, Option<ReferenceRole>>,
}

impl Validate for RefPackUpdateInput {
    fn validate(&self) -> CoreResult<()> {
        if let Some(n) = &self.notes
            && n.chars().count() > MAX_NOTES_CHARS
        {
            return Err(CoreError::invalid(
                "notes",
                format!("notes are at most {MAX_NOTES_CHARS} characters"),
            ));
        }
        if self.roles.len() > MAX_PATHS {
            return Err(CoreError::invalid("roles", "too many roles"));
        }
        for f in self.roles.keys() {
            check_file_name(&format!("roles.{f}"), f)?;
        }
        Ok(())
    }
}

impl Command for RefPackUpdate {
    const NAME: &'static str = "ref_pack_update";
    type Input = RefPackUpdateInput;
    type Output = PackView;

    /// FR-REF-11.
    fn run(core: &Core, input: RefPackUpdateInput) -> CoreResult<PackView> {
        let loc = require_pack(&core.ctx, &settings(core), input.pack_id)?;
        let loc = packs::update(&core.ctx, &loc, input.notes.as_deref(), &input.roles)?;
        changed(core);
        Ok(with_urls(core, packs::view(&core.ctx, &loc)))
    }
}

// ---------------------------------------------------------------- ref_pack_add_images

pub struct RefPackAddImages;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefPackAddImagesInput {
    pub pack_id: Ulid,
    pub paths: Vec<String>,
}

impl Validate for RefPackAddImagesInput {
    fn validate(&self) -> CoreResult<()> {
        if self.paths.is_empty() {
            return Err(CoreError::invalid("paths", "choose at least one file"));
        }
        check_paths("paths", &self.paths)
    }
}

/// The per-source results and the pack as it is now.
#[derive(Debug, Serialize)]
pub struct AddImagesOutput {
    pub items: Vec<AddResult>,
    pub pack: PackView,
}

impl Command for RefPackAddImages {
    const NAME: &'static str = "ref_pack_add_images";
    type Input = RefPackAddImagesInput;
    type Output = AddImagesOutput;

    /// FR-REF-08.
    fn run(core: &Core, input: RefPackAddImagesInput) -> CoreResult<AddImagesOutput> {
        let loc = require_pack(&core.ctx, &settings(core), input.pack_id)?;
        let items = packs::add_images(&core.ctx, &loc, &to_paths(&input.paths));
        changed(core);
        Ok(AddImagesOutput {
            items,
            pack: with_urls(core, packs::view(&core.ctx, &loc)),
        })
    }
}

// ---------------------------------------------------------------- remove, delete

pub struct RefPackRemoveImages;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefPackRemoveImagesInput {
    pub pack_id: Ulid,
    pub files: Vec<String>,
}

impl Validate for RefPackRemoveImagesInput {
    fn validate(&self) -> CoreResult<()> {
        if self.files.is_empty() || self.files.len() > MAX_PATHS {
            return Err(CoreError::invalid("files", "choose 1 to 10000 images"));
        }
        for (i, f) in self.files.iter().enumerate() {
            check_file_name(&format!("files[{i}]"), f)?;
        }
        Ok(())
    }
}

impl Command for RefPackRemoveImages {
    const NAME: &'static str = "ref_pack_remove_images";
    type Input = RefPackRemoveImagesInput;
    type Output = TrashItemOutput;

    /// FR-REF-12.
    fn run(core: &Core, input: RefPackRemoveImagesInput) -> CoreResult<TrashItemOutput> {
        let loc = require_pack(&core.ctx, &settings(core), input.pack_id)?;
        let item_id = trash::remove_images(&core.ctx, &loc, &input.files)?;
        changed(core);
        Ok(TrashItemOutput { item_id })
    }
}

pub struct RefPackDelete;

impl Command for RefPackDelete {
    const NAME: &'static str = "ref_pack_delete";
    type Input = PackIdInput;
    type Output = TrashItemOutput;

    /// FR-REF-12.
    fn run(core: &Core, input: PackIdInput) -> CoreResult<TrashItemOutput> {
        let loc = require_pack(&core.ctx, &settings(core), input.pack_id)?;
        let item_id = trash::delete_pack(&core.ctx, &loc)?;
        changed(core);
        Ok(TrashItemOutput { item_id })
    }
}

// ---------------------------------------------------------------- trash

pub struct RefTrashList;

impl Command for RefTrashList {
    const NAME: &'static str = "ref_trash_list";
    type Input = NoInput;
    type Output = Vec<TrashEntry>;

    fn run(core: &Core, _: NoInput) -> CoreResult<Vec<TrashEntry>> {
        Ok(trash::list(&core.ctx, &settings(core)))
    }
}

pub struct RefTrashRestore;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefTrashRestoreInput {
    pub id: Ulid,
}

impl Validate for RefTrashRestoreInput {}

/// The restored pack folder.
#[derive(Debug, Serialize)]
pub struct RestoreOutput {
    pub path: PathBuf,
}

impl Command for RefTrashRestore {
    const NAME: &'static str = "ref_trash_restore";
    type Input = RefTrashRestoreInput;
    type Output = RestoreOutput;

    /// FR-REF-12.
    fn run(core: &Core, input: RefTrashRestoreInput) -> CoreResult<RestoreOutput> {
        let path = trash::restore(&core.ctx, &settings(core), input.id)?;
        changed(core);
        Ok(RestoreOutput { path })
    }
}

// ---------------------------------------------------------------- ref_resolve

pub struct RefResolve;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefResolveInput {
    pub model_id: String,
    /// The draft's `references`; `null` = REF's defaults (FR-REF-17).
    #[serde(default)]
    pub references: Option<DraftReferences>,
}

const MAX_KEY_CHARS: usize = 600;

impl Validate for RefResolveInput {
    fn validate(&self) -> CoreResult<()> {
        if self.model_id.trim().is_empty() || self.model_id.len() > 200 {
            return Err(CoreError::invalid("model_id", "not a model id"));
        }
        if let Some(r) = &self.references {
            if r.packs.len() > 1000 {
                return Err(CoreError::invalid("references.packs", "too many packs"));
            }
            if r.items.len() > MAX_PATHS {
                return Err(CoreError::invalid("references.items", "too many items"));
            }
            for (i, it) in r.items.iter().enumerate() {
                if it.key.is_empty() || it.key.chars().count() > MAX_KEY_CHARS {
                    return Err(CoreError::invalid(
                        format!("references.items[{i}].key"),
                        "not an item key",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Command for RefResolve {
    const NAME: &'static str = "ref_resolve";
    type Input = RefResolveInput;
    type Output = Resolved;

    /// FR-REF-15 … 19, 27. Runs without the state lock (the capabilities may come from PRV).
    fn run(core: &Core, input: RefResolveInput) -> CoreResult<Resolved> {
        let (dir, read_only, settings) = {
            let st = core.state();
            let p = st.project.as_ref().ok_or_else(|| {
                CoreError::new(
                    crate::error::ErrorCode::NoProject,
                    "no asset project is open",
                )
            })?;
            (p.dir.clone(), p.read_only.is_some(), st.settings.clone())
        };
        let caps = crate::region_edit::commands::capabilities(core, &input.model_id)?;
        let mut out = resolve::resolve(
            &core.ctx,
            &settings,
            &core.refs.probes,
            &dir,
            &caps.slots,
            input.references.as_ref(),
        );
        resolve::mark_duplicates(&mut out.items, |p| core.refs.prepared.content_hash_of(p));
        for item in &mut out.items {
            if let Some(url) = item
                .path
                .as_deref()
                .and_then(|p| image_thumb(core, p))
            {
                item.thumb_key = url
                    .rsplit_once("/i/")
                    .and_then(|(_, k)| k.split_once('.'))
                    .map(|(k, _)| k.to_owned());
                item.thumb_url = Some(url);
            }
        }
        out.read_only = read_only;
        Ok(out)
    }
}

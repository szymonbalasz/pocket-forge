//! Packs: list (with adoption, duplicate ids and conflict copies), get, create, rename, update,
//! add images (FR-REF-01 … 11, 13, 28).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use pf_provider::ReferenceRole;
use serde::Serialize;
use ulid::Ulid;

use super::*;
use crate::create::SkipReason;
use crate::fsx;
use crate::project::{mtime_of, probe_error};

// ---------------------------------------------------------------- list

/// A badge on a pack card (FR-REF-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackBadge {
    /// No `pack.json` could be written yet (FR-REF-04): can't be attached.
    NotReady,
    /// `pack.json` is only in the cloud and can't be fetched (offline).
    NotAvailableOffline,
    /// A `pack.json` conflict copy that couldn't be resolved yet (FR-REF-28).
    Conflict,
}

/// Something a listing did that the user is told about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackNotice {
    /// `adopted` | `new_id` | `conflict_resolved` | `pack_json_replaced`.
    pub code: &'static str,
    pub message: String,
}

/// One pack card (FR-REF-06).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackCard {
    /// The pack id, or the folder path when the pack has no usable `pack.json` (UI record id).
    pub id: String,
    pub pack_id: Option<Ulid>,
    /// The folder name (FR-REF-02).
    pub name: String,
    /// Absolute pack folder.
    pub path: PathBuf,
    pub image_count: usize,
    /// The first line of the notes.
    pub notes_first_line: String,
    /// Absolute thumbnail, when it exists (internal; the UI gets `thumb_url`).
    #[serde(skip)]
    pub thumb: Option<PathBuf>,
    /// A `pf-tiles` URL in EDT's `library` key space, set by the command.
    pub thumb_url: Option<String>,
    pub badges: Vec<PackBadge>,
    pub notices: Vec<PackNotice>,
}

/// `ref_list_packs` (FR-REF-04 … 06, 13, 28): every pack in every available root, sorted by
/// name. Reads only each pack's `pack.json`, its thumbnail and a directory listing. Adopts
/// folders without `pack.json`, gives a duplicate id a new one and resolves `pack.json`
/// conflict copies. Thumbnails that are missing or stale are queued.
pub fn list_packs(ctx: &Context, settings: &Settings) -> Vec<PackCard> {
    let _w = write_lock();
    let mut seen = HashSet::new();
    let mut cards = vec![];
    for root in available_roots(ctx, settings) {
        for (name, dir) in pack_dirs(&root) {
            cards.push(load_card(ctx, &root, &dir, name, &mut seen));
        }
    }
    sort_names(&mut cards, |c| &c.name);
    cards
}

fn load_card(
    ctx: &Context,
    root: &Path,
    dir: &Path,
    name: String,
    seen: &mut HashSet<Ulid>,
) -> PackCard {
    let images = image_files(dir);
    let mut card = PackCard {
        id: dir.to_string_lossy().into_owned(),
        pack_id: None,
        name,
        path: dir.to_path_buf(),
        image_count: images.len(),
        notes_first_line: String::new(),
        thumb: None,
        thumb_url: None,
        badges: vec![],
        notices: vec![],
    };
    match resolve_conflicts(ctx, root, dir, &card.name) {
        Ok(Some(n)) => card.notices.push(n),
        Ok(None) => {}
        Err(e) => {
            tracing::info!(pack = %dir.display(), error = %e, "pack.json conflict not resolved");
            card.badges.push(PackBadge::Conflict);
        }
    }
    let file = dir.join(PACK_FILE);
    let mut json = if std::fs::symlink_metadata(&file).is_ok() {
        if library::hydrate_if_needed(ctx, &file).is_err() {
            card.badges.push(PackBadge::NotAvailableOffline);
            return card;
        }
        match read_pack_json(&file) {
            Ok(j) => Some(j),
            Err(e) if e.code == ErrorCode::Corrupt => {
                // An unreadable pack.json goes to the trash (restorable) and the pack is
                // adopted again with a new id; drafts and jobs fall back to the name.
                match trash::trash_files(ctx, root, None, &card.name, dir, &[PACK_FILE.into()]) {
                    Ok(_) => {
                        card.notices.push(PackNotice {
                            code: "pack_json_replaced",
                            message: format!(
                                "{}'s pack.json was unreadable; it is in Recently deleted",
                                card.name
                            ),
                        });
                        None
                    }
                    Err(_) => {
                        card.badges.push(PackBadge::NotReady);
                        return card;
                    }
                }
            }
            Err(_) => {
                card.badges.push(PackBadge::NotReady);
                return card;
            }
        }
    } else {
        None
    };
    let mut write = false;
    if json.is_none() {
        // FR-REF-04: adopt the folder.
        json = Some(PackJson::new(ctx));
        write = true;
        card.notices.push(PackNotice {
            code: "adopted",
            message: format!("{} is now a reference pack", card.name),
        });
    }
    let Some(mut json) = json else {
        return card;
    };
    if seen.contains(&json.pack_id) {
        // FR-REF-05: a copied folder gets its own id.
        json.pack_id = ctx.new_id();
        json.updated_at = ctx.now();
        write = true;
        card.notices.push(PackNotice {
            code: "new_id",
            message: format!(
                "{} was a copy of another pack; it got its own id",
                card.name
            ),
        });
    }
    if write && fsx::write_json_atomic(ctx, &file, &json).is_err() {
        card.badges.push(PackBadge::NotReady);
        return card;
    }
    seen.insert(json.pack_id);
    card.id = json.pack_id.to_string();
    card.pack_id = Some(json.pack_id);
    card.notes_first_line = json.notes.lines().next().unwrap_or_default().to_owned();
    let thumb_path = dir.join(PACK_APP_DIR).join(thumb::PACK_THUMB_FILE);
    if thumb::needs_rebuild(&thumb_path, &images) {
        thumb::request(ctx, dir, json.pack_id);
    }
    if thumb_path.is_file() && library::hydrate_if_needed(ctx, &thumb_path).is_ok() {
        card.thumb = Some(thumb_path);
    }
    card
}

/// FR-REF-28 (OQ-REF-05 A): of `pack.json` and its conflict copies, the one with the later
/// `updated_at` is kept as `pack.json` (a tie keeps `pack.json`); the others go to the
/// References trash. Also finishes an interrupted resolution (no `pack.json`, copies left).
fn resolve_conflicts(
    ctx: &Context,
    root: &Path,
    dir: &Path,
    pack_name: &str,
) -> CoreResult<Option<PackNotice>> {
    let mut copies: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| naming::is_conflict_copy(n, PACK_FILE))
        .collect();
    if copies.is_empty() {
        return Ok(None);
    }
    copies.sort();
    let main_exists = std::fs::symlink_metadata(dir.join(PACK_FILE)).is_ok();
    let mut names: Vec<String> = vec![];
    if main_exists {
        names.push(PACK_FILE.into());
    }
    names.extend(copies);
    let mut best: Option<(usize, PackJson)> = None;
    for (i, n) in names.iter().enumerate() {
        let path = dir.join(n);
        library::hydrate_if_needed(ctx, &path)?;
        let Ok(j) = read_pack_json(&path) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|(_, b)| j.updated_at > b.updated_at)
        {
            best = Some((i, j));
        }
    }
    let Some((winner, json)) = best else {
        return Err(CoreError::new(
            ErrorCode::Conflict,
            "no readable pack.json among the conflict copies",
        ));
    };
    let losers: Vec<String> = names
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != winner)
        .map(|(_, n)| n.clone())
        .collect();
    trash::trash_files(ctx, root, Some(json.pack_id), pack_name, dir, &losers)?;
    if names[winner] != PACK_FILE {
        fsx::rename_with_retry(&dir.join(&names[winner]), &dir.join(PACK_FILE))
            .map_err(|e| CoreError::io("keeping the newer pack.json", &e))?;
    }
    Ok(Some(PackNotice {
        code: "conflict_resolved",
        message: format!(
            "{pack_name} had a sync conflict; the newer version was kept and the other is in Recently deleted"
        ),
    }))
}

// ---------------------------------------------------------------- get

/// An image's state in the pack view (§9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageState {
    Ok,
    CloudOnly,
    Corrupt,
    Unsupported,
}

/// One image in the pack view.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackImage {
    pub file: String,
    /// Absolute path.
    pub path: PathBuf,
    pub bytes: u64,
    pub mtime: Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub w: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
    pub role: Option<ReferenceRole>,
    pub state: ImageState,
    /// A `pf-tiles` URL (EDT `library` key space, FR-REF-14), set by the command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_url: Option<String>,
}

/// What `ref_pack_get` and the pack writes return.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackView {
    pub pack_id: Ulid,
    pub name: String,
    pub path: PathBuf,
    pub notes: String,
    pub images: Vec<PackImage>,
}

/// State of a local image from its header and end-of-file check (R33). Cloud-only files are
/// never opened.
pub(crate) fn image_state(ctx: &Context, path: &Path) -> (ImageState, Option<(u32, u32)>) {
    if ctx.fs.is_cloud_only(path) {
        return (ImageState::CloudOnly, None);
    }
    match pf_imaging::probe_file_complete(path) {
        Ok(info) => (ImageState::Ok, Some((info.width, info.height))),
        Err(pf_imaging::ProbeError::Corrupt(_) | pf_imaging::ProbeError::Io(_)) => {
            (ImageState::Corrupt, None)
        }
        Err(_) => (ImageState::Unsupported, None),
    }
}

/// `ref_pack_get`: the pack and its images, sorted by name (FR-REF-01, 09).
pub fn view(ctx: &Context, loc: &PackLoc) -> PackView {
    let images = image_files(&loc.dir)
        .into_iter()
        .map(|(file, meta)| {
            let path = loc.dir.join(&file);
            let (state, size) = image_state(ctx, &path);
            PackImage {
                role: loc.json.role_of(&file),
                file,
                path,
                bytes: meta.len(),
                mtime: mtime_of(&meta),
                w: size.map(|s| s.0),
                h: size.map(|s| s.1),
                state,
                thumb_url: None,
            }
        })
        .collect();
    PackView {
        pack_id: loc.json.pack_id,
        name: loc.name.clone(),
        path: loc.dir.clone(),
        notes: loc.json.notes.clone(),
        images,
    }
}

// ---------------------------------------------------------------- add images

/// Outcome of one source (FR-REF-08).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AddResult {
    /// Copied in as `file`.
    Added { source: PathBuf, file: String },
    /// An image with the same content is already in the pack as `file`.
    AlreadyInPack { source: PathBuf, file: String },
    /// Not added (FR-PRJ-06's reasons); the batch continued.
    Skipped { source: PathBuf, reason: SkipReason },
}

fn skip_reason(e: &CoreError) -> SkipReason {
    match e.code {
        ErrorCode::Unsupported => SkipReason::Unsupported,
        ErrorCode::Animated => SkipReason::Animated,
        ErrorCode::TooLarge => SkipReason::TooLarge,
        ErrorCode::Corrupt => SkipReason::Corrupt,
        ErrorCode::NotFound => SkipReason::NotFound,
        ErrorCode::NotAvailableOffline => SkipReason::NotAvailableOffline,
        _ => SkipReason::Io,
    }
}

/// Folders become their supported images, recursively, sorted by path ("copied flat").
fn expand(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, out);
            } else if p.is_file() && pf_imaging::has_image_extension(&p) {
                out.push(p);
            }
        }
    }
    let mut out = vec![];
    for p in paths {
        if p.is_dir() {
            walk(p, &mut out);
        } else {
            out.push(p.clone());
        }
    }
    out
}

/// The file name a copy gets: the source's, without leading dots (they'd hide it).
fn target_name(source: &Path) -> String {
    let n = crate::project::folder_name(source);
    let n = n.trim_start_matches('.');
    if n.is_empty() {
        "image".into()
    } else {
        n.to_owned()
    }
}

/// Copies sources into `dir` (FR-REF-08): never touches a source, ` (2)` for a taken name,
/// `already_in_pack` for content already there, FR-PRJ-06's skip reasons. Emits
/// `ref:add-progress`. Only local pack images are hashed for the comparison.
pub(crate) fn copy_images(ctx: &Context, dir: &Path, sources: &[PathBuf]) -> Vec<AddResult> {
    let sources = expand(sources);
    // (file, size, hash once computed) of the images already in the pack.
    let mut existing: Vec<(String, u64, Option<String>)> = image_files(dir)
        .into_iter()
        .filter(|(n, _)| !ctx.fs.is_cloud_only(&dir.join(n)))
        .map(|(n, m)| (n, m.len(), None))
        .collect();
    let total = sources.len();
    let mut out = Vec::with_capacity(total);
    for (i, source) in sources.iter().enumerate() {
        out.push(copy_one(ctx, dir, source, &mut existing));
        ctx.emit(
            "ref:add-progress",
            serde_json::json!({ "done": i + 1, "total": total }),
        );
    }
    out
}

fn copy_one(
    ctx: &Context,
    dir: &Path,
    source: &Path,
    existing: &mut Vec<(String, u64, Option<String>)>,
) -> AddResult {
    let skipped = |reason| AddResult::Skipped {
        source: source.to_path_buf(),
        reason,
    };
    if !source.is_file() {
        return skipped(SkipReason::NotFound);
    }
    if let Err(e) = library::hydrate_if_needed(ctx, source) {
        return skipped(skip_reason(&e));
    }
    if let Err(e) = pf_imaging::probe_file_complete(source).map_err(probe_error) {
        return skipped(skip_reason(&e));
    }
    let Ok(size) = std::fs::metadata(source).map(|m| m.len()) else {
        return skipped(SkipReason::Io);
    };
    let Ok(hash) = fsx::hash_file(source) else {
        return skipped(SkipReason::Io);
    };
    for (file, s, h) in existing.iter_mut() {
        if *s != size {
            continue;
        }
        if h.is_none() {
            *h = fsx::hash_file(&dir.join(&*file)).ok();
        }
        if h.as_deref() == Some(hash.as_str()) {
            return AddResult::AlreadyInPack {
                source: source.to_path_buf(),
                file: file.clone(),
            };
        }
    }
    let file = free_file_name(dir, &target_name(source));
    match fsx::copy_atomic_new(ctx, source, &dir.join(&file)) {
        Ok((copied_hash, copied_size)) => {
            existing.push((file.clone(), copied_size, Some(copied_hash)));
            AddResult::Added {
                source: source.to_path_buf(),
                file,
            }
        }
        Err(e) => skipped(skip_reason(&e)),
    }
}

/// `ref_pack_add_images` (FR-REF-08). The pack thumbnail is rebuilt afterwards.
pub fn add_images(ctx: &Context, loc: &PackLoc, sources: &[PathBuf]) -> Vec<AddResult> {
    let _w = write_lock();
    let results = copy_images(ctx, &loc.dir, sources);
    if results.iter().any(|r| matches!(r, AddResult::Added { .. })) {
        thumb::request(ctx, &loc.dir, loc.json.pack_id);
    }
    results
}

// ---------------------------------------------------------------- create

/// What `ref_pack_create` returns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CreateOutput {
    pub pack: PackView,
    pub items: Vec<AddResult>,
}

/// `ref_pack_create` (FR-REF-07): builds `References/.<name>.pf-tmp-<ulid>/` (images,
/// `pack.json`) in the first root, then renames it into place. A taken name → `name_taken`,
/// or with `suffix_if_taken` (a dropped folder) the first free ` (n)` name.
pub fn create(
    ctx: &Context,
    settings: &Settings,
    name: &str,
    sources: &[PathBuf],
    suffix_if_taken: bool,
) -> CoreResult<CreateOutput> {
    let _w = write_lock();
    let roots = library::require_available(ctx, settings)?;
    let root = &roots[0];
    let refs = ensure_references_dir(root)?;
    let name = if suffix_if_taken {
        naming::first_free(&naming::sanitise_name(name), |c| name_taken(&refs, c, None))
    } else {
        check_name(name)?;
        if name_taken(&refs, name, None) {
            return Err(name_taken_error(name));
        }
        name.to_owned()
    };
    let tmp = refs.join(format!(
        ".{name}{}{}",
        fsx::TEMP_MARKER,
        ctx.new_id().to_string().to_lowercase()
    ));
    let result = (|| {
        std::fs::create_dir(&tmp)
            .map_err(|e| CoreError::io(format!("creating {}", tmp.display()), &e))?;
        let items = copy_images(ctx, &tmp, sources);
        let json = PackJson::new(ctx);
        fsx::write_json_atomic(ctx, &tmp.join(PACK_FILE), &json)?;
        let target = refs.join(&name);
        if name_taken(&refs, &name, None) {
            return Err(name_taken_error(&name));
        }
        fsx::rename_with_retry(&tmp, &target)
            .map_err(|e| CoreError::io(format!("creating {}", target.display()), &e))?;
        Ok((target, json, items))
    })();
    let (dir, json, items) = match result {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    let loc = PackLoc {
        root: root.clone(),
        dir,
        name,
        json,
    };
    thumb::request(ctx, &loc.dir, loc.json.pack_id);
    Ok(CreateOutput {
        pack: view(ctx, &loc),
        items,
    })
}

// ---------------------------------------------------------------- rename, update

/// `ref_pack_rename` (FR-REF-10): renames the folder; nothing inside changes.
pub fn rename(loc: &PackLoc, name: &str) -> CoreResult<PackLoc> {
    let _w = write_lock();
    check_name(name)?;
    if loc.name == name {
        return Ok(loc.clone());
    }
    let refs = references_dir(&loc.root);
    if name_taken(&refs, name, Some(&loc.name)) {
        return Err(name_taken_error(name));
    }
    let target = refs.join(name);
    fsx::rename_with_retry(&loc.dir, &target)
        .map_err(|e| CoreError::io(format!("renaming {}", loc.dir.display()), &e))?;
    Ok(PackLoc {
        root: loc.root.clone(),
        dir: target,
        name: name.to_owned(),
        json: loc.json.clone(),
    })
}

/// `ref_pack_update` (FR-REF-11): notes and per-image roles in one `pack.json` write. A role
/// for a file that isn't in the pack → `invalid_input`.
pub fn update(
    ctx: &Context,
    loc: &PackLoc,
    notes: Option<&str>,
    roles: &std::collections::BTreeMap<String, Option<ReferenceRole>>,
) -> CoreResult<PackLoc> {
    let _w = write_lock();
    // Re-read, so a write made since the lookup isn't lost.
    let mut json = read_pack_json(&loc.dir.join(PACK_FILE))?;
    if json.pack_id != loc.json.pack_id {
        return Err(CoreError::new(
            ErrorCode::Conflict,
            "the pack changed; refresh",
        ));
    }
    let files: HashSet<String> = image_files(&loc.dir).into_iter().map(|(n, _)| n).collect();
    for file in roles.keys() {
        if !files.contains(file) {
            return Err(CoreError::invalid(
                format!("roles.{file}"),
                format!("{file:?} is not an image in this pack"),
            ));
        }
    }
    if let Some(n) = notes {
        json.notes = n.to_owned();
    }
    for (file, role) in roles {
        let entry = json.images.entry(file.clone()).or_default();
        entry.role = *role;
        if entry.role.is_none() && entry.extra.is_empty() {
            json.images.remove(file);
        }
    }
    json.updated_at = ctx.now();
    fsx::write_json_atomic(ctx, &loc.dir.join(PACK_FILE), &json)?;
    Ok(PackLoc {
        json,
        ..loc.clone()
    })
}

/// Checks that each name is an image file of the pack (remove).
pub(crate) fn require_files(loc: &PackLoc, files: &[String]) -> CoreResult<()> {
    let present: HashSet<String> = image_files(&loc.dir).into_iter().map(|(n, _)| n).collect();
    for (i, f) in files.iter().enumerate() {
        if !present.contains(f) {
            return Err(CoreError::invalid(
                format!("files[{i}]"),
                format!("{f:?} is not an image in this pack"),
            ));
        }
    }
    Ok(())
}

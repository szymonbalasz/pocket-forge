//! Library roots, sections and the home grid (FR-PRJ-01, 02, 09, 45, 46, 49).
//!
//! A root holds `Maps/`, `Tokens/`, `Items/` (#44); each project is a folder in one of them.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use ulid::Ulid;

use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::manifest::{Kind, MANIFEST_FILE, MANIFEST_SCHEMA_VERSION, THUMB_FILE};
use crate::naming;
use crate::settings::Settings;
use crate::time::Timestamp;

pub mod trash;

/// Creation temp folders older than this are removed at startup (FR-PRJ-09, R35).
pub const STALE_TEMP_DIR_AGE: Duration = Duration::from_secs(3600);

/// One effective library root.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RootStatus {
    pub path: PathBuf,
    /// False → "Library unavailable" (FR-PRJ-02); nothing is written under it.
    pub available: bool,
}

/// What `library_get_config` returns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LibraryConfig {
    /// The first receives new projects (FR-PRJ-01).
    pub roots: Vec<RootStatus>,
    /// True when `PF_LIBRARY_DIR` replaces the configured roots for this run.
    pub overridden: bool,
    /// False on first run, before a root is chosen.
    pub configured: bool,
}

/// The roots in effect: `PF_LIBRARY_DIR` if set, else the configured ones.
pub fn effective_roots(ctx: &Context, settings: &Settings) -> Vec<PathBuf> {
    match &ctx.library_override {
        Some(dir) => vec![dir.clone()],
        None => settings.library.roots.clone(),
    }
}

/// Prepares the library at startup: creates the `PF_LIBRARY_DIR` folder and its sections if
/// they're missing (dev and test runs only point it inside the worktree). Configured roots are
/// never created.
pub fn prepare_override(ctx: &Context) -> CoreResult<()> {
    if let Some(dir) = &ctx.library_override {
        std::fs::create_dir_all(dir)
            .map_err(|e| CoreError::io(format!("creating {}", dir.display()), &e))?;
        ensure_sections(dir)?;
    }
    Ok(())
}

/// Creates any missing `Maps/`, `Tokens/`, `Items/` in an existing root (FR-PRJ-01), and a
/// missing `References/` (FR-PRJ-51; not a section).
pub fn ensure_sections(root: &Path) -> CoreResult<()> {
    if !root.is_dir() {
        return Err(unavailable(root));
    }
    for kind in Kind::ALL {
        let dir = root.join(kind.section_dir());
        if !dir.is_dir() {
            std::fs::create_dir(&dir)
                .map_err(|e| CoreError::io(format!("creating {}", dir.display()), &e))?;
        }
    }
    crate::references::ensure_references_dir(root)?;
    Ok(())
}

fn unavailable(root: &Path) -> CoreError {
    CoreError::new(
        ErrorCode::LibraryUnavailable,
        format!("the library folder {} is unavailable", root.display()),
    )
}

pub fn config(ctx: &Context, settings: &Settings) -> LibraryConfig {
    let roots: Vec<RootStatus> = effective_roots(ctx, settings)
        .into_iter()
        .map(|path| RootStatus {
            available: path.is_dir(),
            path,
        })
        .collect();
    LibraryConfig {
        overridden: ctx.library_override.is_some(),
        configured: !roots.is_empty(),
        roots,
    }
}

/// Lexically normalised absolute path (no `.`/`..`), without touching the disk.
pub fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Case-insensitive on Windows and macOS, where the filesystems usually are.
pub fn same_or_inside(path: &Path, base: &Path) -> bool {
    let fold = |p: &Path| {
        let s = normalise(p).to_string_lossy().replace('\\', "/");
        if cfg!(any(windows, target_os = "macos")) {
            s.to_lowercase()
        } else {
            s
        }
    };
    let (p, b) = (fold(path), fold(base));
    let b = b.trim_end_matches('/');
    p == b || p.starts_with(&format!("{b}/"))
}

/// The root containing `path`, if any.
pub fn root_of(ctx: &Context, settings: &Settings, path: &Path) -> Option<PathBuf> {
    effective_roots(ctx, settings)
        .into_iter()
        .find(|r| same_or_inside(path, r))
}

/// Validates new roots (FR-PRJ-01): each must exist, be a writable folder, and not be nested in
/// another root or in `<data-dir>`. Each violation is a field-level error (`roots[i]`).
pub fn validate_roots(ctx: &Context, roots: &[PathBuf]) -> CoreResult<()> {
    if roots.is_empty() {
        return Err(CoreError::invalid("roots", "choose at least one folder"));
    }
    for (i, root) in roots.iter().enumerate() {
        let field = format!("roots[{i}]");
        if !root.is_absolute() {
            return Err(CoreError::invalid(
                field,
                "the folder path must be absolute",
            ));
        }
        if !root.is_dir() {
            return Err(CoreError::invalid(field, "the folder does not exist"));
        }
        if !is_writable(ctx, root) {
            return Err(CoreError::invalid(field, "the folder is not writable"));
        }
        if same_or_inside(root, &ctx.data_dir) || same_or_inside(&ctx.data_dir, root) {
            return Err(CoreError::invalid(
                field,
                "the folder overlaps the app data folder",
            ));
        }
        for (j, other) in roots.iter().enumerate() {
            if i != j && (same_or_inside(root, other) || same_or_inside(other, root)) {
                return Err(CoreError::invalid(
                    field,
                    format!("the folder overlaps roots[{j}]"),
                ));
            }
        }
    }
    Ok(())
}

fn is_writable(ctx: &Context, dir: &Path) -> bool {
    let probe = crate::fsx::temp_path(ctx, &dir.join("write-probe"));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Fails with `library_unavailable` unless a root is configured and every root is reachable.
pub fn require_available(ctx: &Context, settings: &Settings) -> CoreResult<Vec<PathBuf>> {
    let roots = effective_roots(ctx, settings);
    if roots.is_empty() {
        return Err(CoreError::new(
            ErrorCode::LibraryUnavailable,
            "no library folder is set",
        ));
    }
    if let Some(missing) = roots.iter().find(|r| !r.is_dir()) {
        return Err(unavailable(missing));
    }
    Ok(roots)
}

/// The section folder that receives new projects of `kind`: in the first root (FR-PRJ-01),
/// created if it's missing.
pub fn target_section(ctx: &Context, settings: &Settings, kind: Kind) -> CoreResult<PathBuf> {
    let roots = require_available(ctx, settings)?;
    ensure_sections(&roots[0])?;
    Ok(roots[0].join(kind.section_dir()))
}

/// The `kind` of the section a project folder sits in, if its parent is a section folder.
pub fn kind_of_folder(project_dir: &Path) -> Option<Kind> {
    project_dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| Kind::from_section_dir(&n.to_string_lossy()))
}

/// The startup sweep (FR-PRJ-09, R35, BL-02): creation temp folders older than an hour in
/// every available section, and temp files the atomic writer left in `<data-dir>` and its
/// spool. Never touches anything without the temp marker.
pub fn sweep_startup(ctx: &Context, settings: &Settings) -> usize {
    let mut removed = fsx::remove_temp_files_shallow(&ctx.data_dir)
        + fsx::remove_temp_files(&ctx.data_dir.join("pending"));
    for root in effective_roots(ctx, settings) {
        for kind in Kind::ALL {
            let section = root.join(kind.section_dir());
            removed += fsx::remove_stale_temp_dirs(&section, STALE_TEMP_DIR_AGE);
            removed += fsx::remove_temp_files_shallow(&section);
        }
    }
    if removed > 0 {
        tracing::info!(removed, "startup sweep removed temp files and folders");
    }
    removed
}

// ---------------------------------------------------------------- home grid

/// A badge on a home card (FR-PRJ-49).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CardBadge {
    ReadOnly,
    Conflict,
    Unreadable,
    NotAvailableOffline,
    /// "No image yet": a Generate new project before its first pick; the card shows a
    /// placeholder instead of a thumbnail (FR-GEN-03).
    NoImage,
}

/// One project card (FR-PRJ-49).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectCard {
    /// The project id, or the folder path when `project.json` can't be read (UI contract).
    pub id: String,
    /// The folder name (FR-PRJ-44).
    pub name: String,
    /// Absolute project folder (what `project_open` takes).
    pub path: PathBuf,
    /// The section's kind (FR-PRJ-46), whatever the manifest says.
    pub kind: Kind,
    /// Absolute `.pforge/thumb.jpg`, when it exists (internal; the UI gets `thumb_url`).
    #[serde(skip)]
    pub thumb: Option<PathBuf>,
    /// A `pf-tiles` URL in EDT's `library` key space (OQ-PRJ-16), set by the command.
    pub thumb_url: Option<String>,
    pub iterations: usize,
    /// `commit.at`.
    pub last_edited: Option<Timestamp>,
    pub badges: Vec<CardBadge>,
}

/// Hydrates a cloud-only file (FR-PRJ-17). Offline → `not_available_offline`.
pub fn hydrate_if_needed(ctx: &Context, path: &Path) -> CoreResult<()> {
    if ctx.fs.is_cloud_only(path) {
        ctx.fs.hydrate(path).map_err(|e| {
            CoreError::new(
                ErrorCode::NotAvailableOffline,
                format!("{} is only in the cloud: {e}", path.display()),
            )
        })?;
    }
    Ok(())
}

/// `library_list_projects(section)` (FR-PRJ-49): every project folder in that section of every
/// root, newest first. Reads only `project.json` and the thumbnail of each.
pub fn list_projects(
    ctx: &Context,
    settings: &Settings,
    kind: Kind,
) -> CoreResult<Vec<ProjectCard>> {
    let roots = require_available(ctx, settings)?;
    let mut cards = vec![];
    for root in roots {
        let section = root.join(kind.section_dir());
        let Ok(entries) = std::fs::read_dir(&section) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.file_type().is_ok_and(|t| t.is_dir()) || name.contains(fsx::TEMP_MARKER) {
                continue;
            }
            if let Some(card) = card_for(ctx, &entry.path(), name, kind) {
                cards.push(card);
            }
        }
    }
    cards.sort_by(|a, b| {
        b.last_edited
            .cmp(&a.last_edited)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(cards)
}

fn card_for(ctx: &Context, dir: &Path, name: String, kind: Kind) -> Option<ProjectCard> {
    let manifest = dir.join(MANIFEST_FILE);
    // A folder without `project.json` is ignored (FR-PRJ-49).
    if std::fs::symlink_metadata(&manifest).is_err() {
        return None;
    }
    let mut card = ProjectCard {
        id: dir.to_string_lossy().into_owned(),
        name,
        path: dir.to_path_buf(),
        kind,
        thumb: None,
        thumb_url: None,
        iterations: 0,
        last_edited: None,
        badges: vec![],
    };
    if has_conflict_copy(dir) {
        card.badges.push(CardBadge::Conflict);
    }
    if hydrate_if_needed(ctx, &manifest).is_err() {
        card.badges.push(CardBadge::NotAvailableOffline);
        return Some(card);
    }
    let parsed = std::fs::read(&manifest)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let Some(v) = parsed else {
        card.badges.push(CardBadge::Unreadable);
        return Some(card);
    };
    let project_id: Option<Ulid> = v["project_id"].as_str().and_then(|s| s.parse().ok());
    if let Some(id) = project_id {
        card.id = id.to_string();
    }
    card.last_edited = v["commit"]["at"].as_str().and_then(|s| s.parse().ok());
    card.iterations = v["jobs"].as_array().map_or(0, |jobs| {
        jobs.iter().filter(|j| j["iteration"].is_u64()).count()
    });
    match v["schema_version"].as_u64() {
        Some(n) if n > u64::from(MANIFEST_SCHEMA_VERSION) => card.badges.push(CardBadge::ReadOnly),
        Some(_) if project_id.is_some() => {}
        _ => card.badges.push(CardBadge::Unreadable),
    }
    // FR-GEN-03: no layers = no image yet (a stale thumbnail from before an undo is ignored).
    if v["layers"].as_array().is_some_and(|l| l.is_empty()) {
        card.badges.push(CardBadge::NoImage);
        return Some(card);
    }
    let thumb = dir.join(THUMB_FILE);
    if thumb.is_file() && hydrate_if_needed(ctx, &thumb).is_ok() {
        card.thumb = Some(thumb);
    }
    Some(card)
}

/// True if the folder holds a sync conflict copy of `project.json` (FR-PRJ-34).
pub fn has_conflict_copy(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| naming::is_conflict_copy(&e.file_name().to_string_lossy(), MANIFEST_FILE))
    })
}

/// Validates a project folder path: inside a root, an existing folder with `project.json`
/// (FR-PRJ-14). Returns the normalised absolute path.
pub fn require_project_folder(
    ctx: &Context,
    settings: &Settings,
    path: &Path,
) -> CoreResult<PathBuf> {
    let path = std::path::absolute(path)
        .map(|p| normalise(&p))
        .map_err(|e| CoreError::io("resolving the path", &e))?;
    if !path.exists() {
        return Err(CoreError::new(
            ErrorCode::NotFound,
            format!("{} not found", path.display()),
        ));
    }
    if root_of(ctx, settings, &path).is_none() {
        return Err(CoreError::new(
            ErrorCode::OutsideLibrary,
            "the folder is outside the library",
        ));
    }
    Ok(path)
}

/// True if `a` and `b` name the same folder (lexically; case-insensitive where the filesystem
/// usually is).
pub fn same_folder(a: &Path, b: &Path) -> bool {
    same_or_inside(a, b) && same_or_inside(b, a)
}

/// `project_rename` (FR-PRJ-45): renames a closed project's folder. Nothing inside changes.
/// The open project is renamed by the command (FR-PRJ-56).
pub fn rename_project(
    ctx: &Context,
    settings: &Settings,
    path: &Path,
    name: &str,
) -> CoreResult<PathBuf> {
    let dir = require_project_folder(ctx, settings, path)?;
    if !dir.join(MANIFEST_FILE).is_file() {
        return Err(not_a_project(&dir));
    }
    match rename_target(&dir, name)? {
        Some(target) => {
            fsx::move_dir(ctx, &dir, &target)?;
            Ok(target)
        }
        None => Ok(dir),
    }
}

/// The FR-PRJ-44 checks of a rename (FR-PRJ-45, 56): `invalid_name`, `name_taken`. Returns the
/// new folder, or `None` when `name` is the current one. Nothing is written.
pub fn rename_target(dir: &Path, name: &str) -> CoreResult<Option<PathBuf>> {
    if !naming::is_valid_name(name) {
        return Err(CoreError::new(
            ErrorCode::InvalidName,
            format!(
                "names can't contain <>:\"/\\|?* or control characters, start with a dot, end with a dot or space, be a reserved Windows name, or be longer than {} characters",
                naming::MAX_NAME_CHARS
            ),
        )
        .with_field("name"));
    }
    let parent = dir
        .parent()
        .ok_or_else(|| CoreError::internal("a project folder has no parent"))?;
    let current = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if current == name {
        return Ok(None);
    }
    let taken = std::fs::read_dir(parent)
        .map_err(|e| CoreError::io("reading the section folder", &e))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .any(|n| naming::same_name(&n, name) && !naming::same_name(&n, &current));
    if taken {
        return Err(CoreError::new(
            ErrorCode::NameTaken,
            format!("a project named {name:?} already exists here"),
        )
        .with_field("name"));
    }
    Ok(Some(parent.join(name)))
}

pub fn not_a_project(dir: &Path) -> CoreError {
    CoreError::new(
        ErrorCode::NotAProject,
        format!(
            "{} is not an asset project (no project.json)",
            dir.display()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Context) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::deterministic(dir.path().join("data"), Timestamp(0));
        std::fs::create_dir_all(&ctx.data_dir).unwrap();
        (dir, ctx)
    }

    #[test]
    fn fr_prj_01_root_must_exist() {
        let (dir, ctx) = setup();
        let err = validate_roots(&ctx, &[dir.path().join("missing")]).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("roots[0]"));
        assert_eq!(err.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn fr_prj_01_roots_must_not_nest_or_overlap_data_dir() {
        let (dir, ctx) = setup();
        let a = dir.path().join("a");
        std::fs::create_dir_all(a.join("b")).unwrap();
        let err = validate_roots(&ctx, &[a.clone(), a.join("b")]).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("roots[0]"));
        let err = validate_roots(&ctx, &[ctx.data_dir.join("..").join("data")]).unwrap_err();
        assert!(err.message.contains("app data"), "{err:?}");
        let err = validate_roots(&ctx, &[dir.path().to_path_buf()]).unwrap_err();
        assert!(err.message.contains("app data"), "{err:?}");
        validate_roots(&ctx, &[a]).unwrap();
    }

    #[test]
    fn fr_prj_01_relative_paths_are_rejected() {
        let (_dir, ctx) = setup();
        assert!(validate_roots(&ctx, &[PathBuf::from("relative")]).is_err());
        assert!(validate_roots(&ctx, &[]).is_err());
    }

    #[test]
    fn fr_prj_01_sections_are_created_in_a_root() {
        let (dir, _ctx) = setup();
        let lib = dir.path().join("lib");
        std::fs::create_dir(&lib).unwrap();
        std::fs::create_dir(lib.join("Maps")).unwrap();
        std::fs::write(lib.join("Maps/keep.txt"), b"x").unwrap();
        ensure_sections(&lib).unwrap();
        for s in ["Maps", "Tokens", "Items"] {
            assert!(lib.join(s).is_dir(), "{s}");
        }
        assert!(lib.join("Maps/keep.txt").exists());
        assert_eq!(
            ensure_sections(&dir.path().join("gone")).unwrap_err().code,
            ErrorCode::LibraryUnavailable
        );
    }

    #[test]
    fn fr_prj_02_unavailable_root_reports_and_blocks() {
        let (dir, ctx) = setup();
        let mut s = Settings::defaults(&ctx);
        s.library.roots = vec![dir.path().join("gone")];
        let cfg = config(&ctx, &s);
        assert!(!cfg.roots[0].available);
        assert_eq!(
            require_available(&ctx, &s).unwrap_err().code,
            ErrorCode::LibraryUnavailable
        );
        assert_eq!(
            target_section(&ctx, &s, Kind::Map).unwrap_err().code,
            ErrorCode::LibraryUnavailable
        );
        assert!(!dir.path().join("gone").exists(), "nothing written");
    }

    #[test]
    fn override_replaces_roots_and_gets_sections() {
        let (dir, mut ctx) = setup();
        let lib = dir.path().join("lib");
        ctx.library_override = Some(lib.clone());
        prepare_override(&ctx).unwrap();
        let s = Settings::defaults(&ctx);
        let cfg = config(&ctx, &s);
        assert!(cfg.overridden && cfg.configured && cfg.roots[0].available);
        assert!(lib.join("Tokens").is_dir());
        assert_eq!(root_of(&ctx, &s, &lib.join("x/y.png")), Some(lib.clone()));
        assert_eq!(root_of(&ctx, &s, &dir.path().join("libx/y.png")), None);
        assert_eq!(
            target_section(&ctx, &s, Kind::Item).unwrap(),
            lib.join("Items")
        );
    }

    #[test]
    fn fr_prj_09_r35_startup_sweep_removes_only_marked_leftovers() {
        let (dir, mut ctx) = setup();
        let lib = dir.path().join("lib");
        ctx.library_override = Some(lib.clone());
        prepare_override(&ctx).unwrap();
        let s = Settings::defaults(&ctx);
        std::fs::write(ctx.data_dir.join(".settings.json.pf-tmp-01"), b"x").unwrap();
        std::fs::write(ctx.data_dir.join("settings.json"), b"{}").unwrap();
        std::fs::create_dir_all(ctx.data_dir.join("pending/p1")).unwrap();
        std::fs::write(ctx.data_dir.join("pending/p1/.1.json.pf-tmp-02"), b"x").unwrap();
        std::fs::write(lib.join("Maps/.x.png.pf-tmp-03"), b"x").unwrap();
        std::fs::create_dir(lib.join("Maps/.Crypt.pf-tmp-04")).unwrap();
        std::fs::create_dir(lib.join("Maps/Crypt")).unwrap();
        assert_eq!(sweep_startup(&ctx, &s), 3, "the fresh temp folder stays");
        assert!(ctx.data_dir.join("settings.json").exists());
        assert!(lib.join("Maps/.Crypt.pf-tmp-04").is_dir());
        assert!(lib.join("Maps/Crypt").is_dir());
    }
}

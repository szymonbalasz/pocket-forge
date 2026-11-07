//! Library roots and the import destination (FR-PRJ-01, FR-PRJ-02, FR-PRJ-04).

use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::settings::Settings;

/// Default import destination folder under the first root (#40).
pub const IMPORTED_DIR: &str = "Imported";

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
    pub roots: Vec<RootStatus>,
    pub import_dest: Option<PathBuf>,
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

/// Prepares the library at startup: creates the `PF_LIBRARY_DIR` folder if it's missing (dev and
/// test runs only point it inside the worktree). Configured roots are never created.
pub fn prepare_override(ctx: &Context) -> CoreResult<()> {
    if let Some(dir) = &ctx.library_override {
        std::fs::create_dir_all(dir)
            .map_err(|e| CoreError::io(format!("creating {}", dir.display()), &e))?;
    }
    Ok(())
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
        import_dest: import_dest(ctx, settings),
        overridden: ctx.library_override.is_some(),
        configured: !roots.is_empty(),
        roots,
    }
}

/// The import destination: the configured one, else `<first root>/Imported/`.
pub fn import_dest(ctx: &Context, settings: &Settings) -> Option<PathBuf> {
    if ctx.library_override.is_none()
        && let Some(dest) = &settings.library.import_dest
    {
        return Some(dest.clone());
    }
    effective_roots(ctx, settings)
        .first()
        .map(|r| r.join(IMPORTED_DIR))
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
fn same_or_inside(path: &Path, base: &Path) -> bool {
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

/// The root containing `path`, if any (FR-PRJ-05).
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
        return Err(CoreError::new(
            ErrorCode::LibraryUnavailable,
            format!("the library folder {} is unavailable", missing.display()),
        ));
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Timestamp;

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
    }

    #[test]
    fn override_replaces_roots_and_sets_import_dest() {
        let (dir, mut ctx) = setup();
        let lib = dir.path().join("lib");
        ctx.library_override = Some(lib.clone());
        prepare_override(&ctx).unwrap();
        let s = Settings::defaults(&ctx);
        let cfg = config(&ctx, &s);
        assert!(cfg.overridden && cfg.configured && cfg.roots[0].available);
        assert_eq!(cfg.import_dest, Some(lib.join(IMPORTED_DIR)));
        assert_eq!(root_of(&ctx, &s, &lib.join("x/y.png")), Some(lib.clone()));
        assert_eq!(root_of(&ctx, &s, &dir.path().join("libx/y.png")), None);
    }
}

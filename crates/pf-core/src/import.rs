//! Bringing images into the library (FR-PRJ-03 … FR-PRJ-09). Skeleton scope: files.
//! Folder recursion, clipboard data, progress events and cancel are PRJ slice 2.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::library;
use crate::settings::Settings;

/// Outcome of one source path.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ImportItem {
    /// Copied into the import destination.
    Imported { source: PathBuf, path: PathBuf },
    /// Already in the library: the source is inside a root (used in place, FR-PRJ-05), or the
    /// same content is already in the target folder (FR-PRJ-08).
    AlreadyInLibrary { source: PathBuf, path: PathBuf },
    /// Not imported; the batch continues (FR-PRJ-06).
    Skipped { source: PathBuf, reason: SkipReason },
}

impl ImportItem {
    /// The library path to open, if the item is usable.
    pub fn library_path(&self) -> Option<&Path> {
        match self {
            ImportItem::Imported { path, .. } | ImportItem::AlreadyInLibrary { path, .. } => {
                Some(path)
            }
            ImportItem::Skipped { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Unsupported,
    Animated,
    TooLarge,
    Corrupt,
    NotFound,
    Io,
}

/// Imports files. Returns one item per input path, in order.
pub fn import_paths(
    ctx: &Context,
    settings: &Settings,
    paths: &[PathBuf],
) -> CoreResult<Vec<ImportItem>> {
    library::require_available(ctx, settings)?;
    let dest = library::import_dest(ctx, settings).ok_or_else(|| {
        CoreError::new(
            ErrorCode::LibraryUnavailable,
            "no import destination is set",
        )
    })?;
    Ok(paths
        .iter()
        .map(|p| import_one(ctx, settings, &dest, p))
        .collect())
}

fn import_one(ctx: &Context, settings: &Settings, dest: &Path, source: &Path) -> ImportItem {
    let skipped = |reason| ImportItem::Skipped {
        source: source.to_path_buf(),
        reason,
    };
    if !source.exists() {
        return skipped(SkipReason::NotFound);
    }
    if !source.is_file() {
        // Folder import is PRJ slice 2.
        return skipped(SkipReason::Unsupported);
    }
    if let Err(e) = pf_imaging::probe_file(source) {
        use pf_imaging::ProbeError as P;
        return skipped(match e {
            P::Unsupported => SkipReason::Unsupported,
            P::Animated => SkipReason::Animated,
            P::TooLarge { .. } => SkipReason::TooLarge,
            P::Corrupt(_) => SkipReason::Corrupt,
            P::Io(_) => SkipReason::Io,
        });
    }
    if library::root_of(ctx, settings, source).is_some() {
        return ImportItem::AlreadyInLibrary {
            source: source.to_path_buf(),
            path: source.to_path_buf(),
        };
    }
    match copy_into(ctx, dest, source) {
        Ok(item) => item,
        Err(e) => {
            tracing::warn!(source = %source.display(), error = %e, "import copy failed");
            skipped(SkipReason::Io)
        }
    }
}

/// Copies `source` into `dest` under a free name, or reuses a file with the same content.
fn copy_into(ctx: &Context, dest: &Path, source: &Path) -> CoreResult<ImportItem> {
    std::fs::create_dir_all(dest)
        .map_err(|e| CoreError::io(format!("creating {}", dest.display()), &e))?;
    let size = std::fs::metadata(source)
        .map_err(|e| CoreError::io("reading the source", &e))?
        .len();
    let mut source_hash = None;
    let name = source
        .file_name()
        .ok_or_else(|| CoreError::invalid("paths", "the path has no file name"))?;
    for n in 1u32.. {
        let candidate = dest.join(suffixed(Path::new(name), n));
        if candidate.exists() {
            let same_size = std::fs::metadata(&candidate).is_ok_and(|m| m.len() == size);
            if same_size {
                let h = match &source_hash {
                    Some(h) => h,
                    None => source_hash.insert(
                        fsx::hash_file(source)
                            .map_err(|e| CoreError::io("hashing the source", &e))?,
                    ),
                };
                if fsx::hash_file(&candidate).is_ok_and(|c| &c == h) {
                    return Ok(ImportItem::AlreadyInLibrary {
                        source: source.to_path_buf(),
                        path: candidate,
                    });
                }
            }
            continue;
        }
        fsx::copy_atomic_new(ctx, source, &candidate)?;
        return Ok(ImportItem::Imported {
            source: source.to_path_buf(),
            path: candidate,
        });
    }
    Err(CoreError::internal("no free file name"))
}

/// `crypt.png` → `crypt (2).png` for n ≥ 2 (FR-PRJ-08).
pub fn suffixed(name: &Path, n: u32) -> PathBuf {
    if n < 2 {
        return name.to_path_buf();
    }
    let stem = name.file_stem().unwrap_or_default().to_string_lossy();
    match name.extension() {
        Some(ext) => PathBuf::from(format!("{stem} ({n}).{}", ext.to_string_lossy())),
        None => PathBuf::from(format!("{stem} ({n})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Timestamp;

    fn png_bytes(w: u32, h: u32, shade: u8) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&vec![shade; (w * h * 3) as usize])
            .unwrap();
        wr.finish().unwrap();
        out
    }

    struct Env {
        dir: tempfile::TempDir,
        ctx: Context,
        settings: Settings,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = Context::deterministic(dir.path().join("data"), Timestamp(0));
        ctx.library_override = Some(dir.path().join("lib"));
        library::prepare_override(&ctx).unwrap();
        std::fs::create_dir_all(dir.path().join("outside")).unwrap();
        let settings = Settings::defaults(&ctx);
        Env { dir, ctx, settings }
    }

    impl Env {
        fn outside(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let p = self.dir.path().join("outside").join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        }
        fn import(&self, paths: &[PathBuf]) -> Vec<ImportItem> {
            import_paths(&self.ctx, &self.settings, paths).unwrap()
        }
    }

    #[test]
    fn fr_prj_04_copies_outside_source_into_imported_and_leaves_it_unchanged() {
        let e = env();
        let src = e.outside("crypt.png", &png_bytes(8, 8, 10));
        let before = (
            std::fs::metadata(&src).unwrap().modified().unwrap(),
            fsx::hash_file(&src).unwrap(),
        );
        let items = e.import(std::slice::from_ref(&src));
        let ImportItem::Imported { path, .. } = &items[0] else {
            panic!("{items:?}")
        };
        assert_eq!(path, &e.dir.path().join("lib/Imported/crypt.png"));
        assert_eq!(fsx::hash_file(path).unwrap(), before.1);
        let after = (
            std::fs::metadata(&src).unwrap().modified().unwrap(),
            fsx::hash_file(&src).unwrap(),
        );
        assert_eq!(before, after);
    }

    #[test]
    fn fr_prj_05_source_inside_root_is_used_in_place() {
        let e = env();
        let inside = e.dir.path().join("lib/maps/a.png");
        std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
        std::fs::write(&inside, png_bytes(4, 4, 1)).unwrap();
        let items = e.import(std::slice::from_ref(&inside));
        assert_eq!(
            items[0],
            ImportItem::AlreadyInLibrary {
                source: inside.clone(),
                path: inside
            }
        );
        assert!(!e.dir.path().join("lib/Imported").exists());
    }

    #[test]
    fn fr_prj_06_unsupported_files_are_skipped_and_the_batch_continues() {
        let e = env();
        let psd = e.outside("art.psd", b"8BPS\0\x01rest");
        let ok = e.outside("ok.png", &png_bytes(4, 4, 2));
        let missing = e.dir.path().join("outside/none.png");
        let items = e.import(&[psd.clone(), ok, missing.clone()]);
        assert_eq!(
            items[0],
            ImportItem::Skipped {
                source: psd,
                reason: SkipReason::Unsupported
            }
        );
        assert!(matches!(items[1], ImportItem::Imported { .. }));
        assert_eq!(
            items[2],
            ImportItem::Skipped {
                source: missing,
                reason: SkipReason::NotFound
            }
        );
    }

    #[test]
    fn fr_prj_08_name_collision_gets_suffix_and_same_content_is_reused() {
        let e = env();
        let a = e.outside("map.png", &png_bytes(4, 4, 1));
        e.import(std::slice::from_ref(&a));
        // Same name, different content → " (2)".
        std::fs::create_dir_all(e.dir.path().join("outside/other")).unwrap();
        let b = e.dir.path().join("outside/other/map.png");
        std::fs::write(&b, png_bytes(4, 4, 99)).unwrap();
        let items = e.import(std::slice::from_ref(&b));
        assert_eq!(
            items[0].library_path().unwrap(),
            e.dir.path().join("lib/Imported/map (2).png")
        );
        // Same content again → reused.
        let items = e.import(&[a]);
        assert!(
            matches!(&items[0], ImportItem::AlreadyInLibrary { path, .. } if path.ends_with("Imported/map.png")),
            "{items:?}"
        );
    }

    #[test]
    fn fr_prj_09_no_temp_files_remain_after_import() {
        let e = env();
        let a = e.outside("x.png", &png_bytes(4, 4, 1));
        e.import(&[a]);
        let names: Vec<_> = std::fs::read_dir(e.dir.path().join("lib/Imported"))
            .unwrap()
            .map(|d| d.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["x.png"]);
    }

    #[test]
    fn suffix_names() {
        assert_eq!(suffixed(Path::new("a.png"), 1), PathBuf::from("a.png"));
        assert_eq!(suffixed(Path::new("a.png"), 3), PathBuf::from("a (3).png"));
        assert_eq!(suffixed(Path::new("a"), 2), PathBuf::from("a (2)"));
    }
}

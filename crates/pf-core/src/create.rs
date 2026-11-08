//! New from image (FR-PRJ-03 … 11, 44, 50): every source is **copied** into a new project
//! folder `<section>/<name>/working/<source file name>`; the source is never touched. A project
//! is built in `<section>/.<name>.pf-tmp-<ulid>/` and renamed into place, so an interrupted
//! creation never leaves a partial project under a final name (FR-PRJ-09).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde_json::json;
use ulid::Ulid;

use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::library;
use crate::manifest::*;
use crate::naming;
use crate::project::{mtime_of, probe_error};
use crate::settings::Settings;
use crate::thumb::{ThumbJob, Thumbnails};

/// Why a source made no project (FR-PRJ-06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Unsupported,
    Animated,
    TooLarge,
    Corrupt,
    NotFound,
    NotAvailableOffline,
    Io,
}

/// Outcome of one source file (FR-PRJ-03, 08).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CreateItem {
    /// A new project folder.
    Created { source: PathBuf, path: PathBuf },
    /// The same image is already a project's original in this section: that project's folder.
    AlreadyInLibrary { source: PathBuf, path: PathBuf },
    /// Nothing was created; the batch continues.
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

/// FR-PRJ-07: a folder makes one project per supported image found recursively. Files that are
/// neither an image by magic bytes nor by extension are left out; files are sorted by path.
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
            } else if p.is_file() && looks_like_image(&p) {
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

fn looks_like_image(p: &Path) -> bool {
    if pf_imaging::has_image_extension(p) {
        return true;
    }
    let mut head = [0u8; 16];
    std::fs::File::open(p)
        .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
        .is_ok_and(|n| pf_imaging::sniff(&head[..n]).is_some())
}

/// An existing project's original in the section (FR-PRJ-08).
#[derive(Debug, Clone)]
struct Known {
    size: u64,
    hash: String,
    dir: PathBuf,
}

/// The originals of every project in this section of every root. Reads `project.json` only.
fn known_originals(ctx: &Context, settings: &Settings, kind: Kind) -> Vec<Known> {
    let mut known = vec![];
    for root in library::effective_roots(ctx, settings) {
        let section = root.join(kind.section_dir());
        for entry in std::fs::read_dir(&section).into_iter().flatten().flatten() {
            let dir = entry.path();
            let manifest = dir.join(MANIFEST_FILE);
            if entry
                .file_name()
                .to_string_lossy()
                .contains(fsx::TEMP_MARKER)
                || std::fs::symlink_metadata(&manifest).is_err()
                || library::hydrate_if_needed(ctx, &manifest).is_err()
            {
                continue;
            }
            let v: Option<serde_json::Value> = std::fs::read(&manifest)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
            if let Some(v) = v
                && let (Some(size), Some(hash)) = (
                    v["original"]["size"].as_u64(),
                    v["original"]["content_hash"].as_str(),
                )
            {
                known.push(Known {
                    size,
                    hash: hash.to_owned(),
                    dir,
                });
            }
        }
    }
    known
}

/// Names taken in the section, in every root (case-insensitive, FR-PRJ-44).
fn name_taken(ctx: &Context, settings: &Settings, kind: Kind, name: &str) -> bool {
    library::effective_roots(ctx, settings).iter().any(|root| {
        std::fs::read_dir(root.join(kind.section_dir()))
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| naming::same_name(&e.file_name().to_string_lossy(), name))
    })
}

/// A file to put into `working/`: copied from a path, or written from bytes (a paste).
enum Source<'a> {
    File(&'a Path),
    Bytes { name: String, bytes: &'a [u8] },
}

/// Builds one project in a temp folder and renames it into place (FR-PRJ-09). Returns the
/// project folder. On any error the temp folder is removed.
fn build_project(
    ctx: &Context,
    settings: &Settings,
    section: &Path,
    kind: Kind,
    name: &str,
    source: Source<'_>,
    info: pf_imaging::ImageInfo,
) -> CoreResult<(PathBuf, Manifest)> {
    let temp = fsx::temp_path(ctx, &section.join(name));
    let result = (|| {
        for d in PROJECT_DIRS {
            std::fs::create_dir_all(temp.join(d))
                .map_err(|e| CoreError::io(format!("creating {d}"), &e))?;
        }
        let (file_name, hash, size) = match &source {
            Source::File(path) => {
                let file_name = crate::project::folder_name(path);
                let (hash, size) =
                    fsx::copy_atomic_new(ctx, path, &temp.join(WORKING_DIR).join(&file_name))?;
                (file_name, hash, size)
            }
            Source::Bytes { name, bytes } => {
                let target = temp.join(WORKING_DIR).join(name);
                fsx::write_atomic(ctx, &target, bytes)?;
                let hash = fsx::hash_file(&target)
                    .map_err(|e| CoreError::io("hashing the pasted image", &e))?;
                (name.clone(), hash, bytes.len() as u64)
            }
        };
        let rel = format!("{WORKING_DIR}/{file_name}");
        let meta = std::fs::metadata(temp.join(&rel))
            .map_err(|e| CoreError::io("reading the copy", &e))?;
        let now = ctx.now();
        let original_layer = Layer::original(ctx.new_id(), info.width, info.height);
        let r0 = RevisionFile {
            schema_version: REVISION_SCHEMA_VERSION,
            id: ctx.new_id(),
            seq: 0,
            created_at: now,
            reason: RevisionReason::Create,
            label: None,
            parent: None,
            layers: vec![original_layer.clone()],
            draft: None,
            extra: Extra::new(),
        };
        let manifest = Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            project_id: ctx.new_id(),
            kind,
            created_at: now,
            commit: Commit {
                seq: 1,
                install_id: settings.install_id,
                at: now,
            },
            original: Original {
                file: rel,
                content_hash: hash,
                size,
                mtime: mtime_of(&meta),
                width: info.width,
                height: info.height,
                format: info.format.as_str().into(),
                extra: Extra::new(),
            },
            layers: vec![original_layer],
            masks: vec![],
            draft: None,
            jobs: vec![],
            candidates: vec![],
            revisions: vec![r0.entry()],
            head_revision: r0.id,
            trash: vec![],
            export: None,
            extra: Extra::new(),
        };
        fsx::write_json_atomic(ctx, &temp.join(revision_file(r0.id)), &r0)?;
        fsx::write_json_atomic(ctx, &temp.join(MANIFEST_FILE), &manifest)?;
        let target = section.join(name);
        if std::fs::symlink_metadata(&target).is_ok() {
            return Err(CoreError::new(
                ErrorCode::NameTaken,
                format!("{name:?} appeared during the creation"),
            ));
        }
        fsx::rename_with_retry(&temp, &target)
            .map_err(|e| CoreError::io(format!("renaming into {}", target.display()), &e))?;
        fsx::set_hidden(&target.join(PFORGE_DIR));
        Ok((target, manifest))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&temp);
    }
    result
}

/// What a creation run shares: settings, the target section, the dedupe index.
pub struct Creator<'a> {
    ctx: &'a Context,
    settings: &'a Settings,
    kind: Kind,
    section: PathBuf,
    known: Vec<Known>,
    thumbs: Option<&'a Thumbnails>,
}

impl<'a> Creator<'a> {
    /// Fails with `library_unavailable` unless the first root is reachable. Creates any missing
    /// section folders there (FR-PRJ-01).
    pub fn new(
        ctx: &'a Context,
        settings: &'a Settings,
        kind: Kind,
        thumbs: Option<&'a Thumbnails>,
    ) -> CoreResult<Self> {
        let section = library::target_section(ctx, settings, kind)?;
        Ok(Creator {
            known: known_originals(ctx, settings, kind),
            ctx,
            settings,
            kind,
            section,
            thumbs,
        })
    }

    fn free_name(&self, raw: &str) -> String {
        let base = naming::sanitise_name(raw);
        naming::first_free(&base, |c| name_taken(self.ctx, self.settings, self.kind, c))
    }

    fn finish(&mut self, dir: &Path, m: &Manifest) {
        self.known.push(Known {
            size: m.original.size,
            hash: m.original.content_hash.clone(),
            dir: dir.to_path_buf(),
        });
        if let Some(t) = self.thumbs {
            t.request(ThumbJob::new(self.ctx, dir, m));
        }
    }

    /// One source file → one item. Never fails the batch.
    pub fn create_one(&mut self, source: &Path) -> CreateItem {
        let skipped = |reason| CreateItem::Skipped {
            source: source.to_path_buf(),
            reason,
        };
        if !source.is_file() {
            return skipped(SkipReason::NotFound);
        }
        if let Err(e) = library::hydrate_if_needed(self.ctx, source) {
            return skipped(skip_reason(&e));
        }
        let info = match pf_imaging::probe_file_complete(source) {
            Ok(i) => i,
            Err(e) => return skipped(skip_reason(&probe_error(e))),
        };
        let Ok(size) = std::fs::metadata(source).map(|m| m.len()) else {
            return skipped(SkipReason::Io);
        };
        if self.known.iter().any(|k| k.size == size) {
            let Ok(hash) = fsx::hash_file(source) else {
                return skipped(SkipReason::Io);
            };
            if let Some(k) = self.known.iter().find(|k| k.size == size && k.hash == hash) {
                return CreateItem::AlreadyInLibrary {
                    source: source.to_path_buf(),
                    path: k.dir.clone(),
                };
            }
        }
        let stem = source
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = self.free_name(&stem);
        match build_project(
            self.ctx,
            self.settings,
            &self.section,
            self.kind,
            &name,
            Source::File(source),
            info,
        ) {
            Ok((dir, m)) => {
                self.finish(&dir, &m);
                CreateItem::Created {
                    source: source.to_path_buf(),
                    path: dir,
                }
            }
            Err(e) => {
                tracing::warn!(source = %source.display(), error = %e, "project creation failed");
                skipped(skip_reason(&e))
            }
        }
    }

    /// FR-PRJ-10: pasted image data → project `pasted-<UTC yyyymmdd-hhmmss>` with a lossless PNG.
    pub fn create_from_bytes(&mut self, bytes: &[u8]) -> CoreResult<PathBuf> {
        let info = pf_imaging::probe_bytes(bytes).map_err(probe_error)?;
        let png;
        let png_bytes = if info.format == pf_imaging::ImageFormat::Png {
            bytes
        } else {
            let img = image::load_from_memory(bytes).map_err(|e| {
                CoreError::new(
                    ErrorCode::Corrupt,
                    format!("the pasted image won't decode: {e}"),
                )
            })?;
            let mut out = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .map_err(|e| CoreError::internal(format!("encoding the pasted image: {e}")))?;
            png = out;
            &png
        };
        let info = pf_imaging::ImageInfo {
            format: pf_imaging::ImageFormat::Png,
            ..info
        };
        let stamp = format!("pasted-{}", self.ctx.now().file_stamp());
        let name = self.free_name(&stamp);
        let (dir, m) = build_project(
            self.ctx,
            self.settings,
            &self.section,
            self.kind,
            &name,
            Source::Bytes {
                name: format!("{stamp}.png"),
                bytes: png_bytes,
            },
            info,
        )?;
        self.finish(&dir, &m);
        Ok(dir)
    }
}

/// `project_create` (FR-PRJ-03, 07, 11): one item per source file, in order. Emits
/// `import:progress{done, total}` per file and `import:done` at the end. A cancel stops before the
/// next file with `canceled` (`detail.items` = the finished items; those projects stay).
pub fn create_projects(
    ctx: &Context,
    settings: &Settings,
    kind: Kind,
    paths: &[PathBuf],
    cancel: &AtomicBool,
    thumbs: Option<&Thumbnails>,
) -> CoreResult<Vec<CreateItem>> {
    let mut creator = Creator::new(ctx, settings, kind, thumbs)?;
    let sources = expand(paths);
    let total = sources.len();
    let mut items = Vec::with_capacity(total);
    for (i, source) in sources.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            ctx.emit(
                "import:done",
                json!({ "total": total, "done": i, "canceled": true }),
            );
            return Err(CoreError::new(
                ErrorCode::Canceled,
                format!("canceled after {i} of {total}"),
            )
            .with_detail(json!({ "items": items })));
        }
        items.push(creator.create_one(source));
        ctx.emit("import:progress", json!({ "done": i + 1, "total": total }));
    }
    ctx.emit(
        "import:done",
        json!({ "total": total, "done": total, "canceled": false }),
    );
    Ok(items)
}

/// For tests and the clipboard command: the id of a fresh project from its folder.
pub fn project_id_of(dir: &Path) -> Option<Ulid> {
    let bytes = std::fs::read(dir.join(MANIFEST_FILE)).ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes).ok()?["project_id"]
        .as_str()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests;

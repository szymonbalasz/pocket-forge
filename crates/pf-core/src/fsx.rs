//! Crash-safe file writes: temp → flush → rename (FR-PRJ-09, FR-PRJ-18, manifest-schema §1).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::context::{Context, FaultPoint};
use crate::error::{CoreError, CoreResult, is_locked};

/// Marker every temp file name contains. Files with it are deleted on open (FR-PRJ-20).
pub const TEMP_MARKER: &str = ".pf-tmp-";

/// How long a locked file is retried before `locked` (FR-PRJ-21).
const LOCK_RETRY_FOR: Duration = Duration::from_secs(3);

/// The temp sibling for `target`: `.<name>.pf-tmp-<ulid>` in the same folder.
pub fn temp_path(ctx: &Context, target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    target.with_file_name(format!(
        ".{name}{TEMP_MARKER}{}",
        ctx.new_id().to_string().to_lowercase()
    ))
}

/// Writes `bytes` to `target` atomically. A crash at any point leaves either the old file or
/// the new one under `target`, plus at most a temp file.
pub fn write_atomic(ctx: &Context, target: &Path, bytes: &[u8]) -> CoreResult<()> {
    let tmp = temp_path(ctx, target);
    let result = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        ctx.faults.hit(FaultPoint::AfterTemp, target)?;
        rename_with_retry(&tmp, target)?;
        ctx.faults.hit(FaultPoint::AfterRename, target)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(CoreError::io(format!("writing {}", target.display()), &e));
    }
    Ok(())
}

/// Writes a JSON value pretty-printed (2 spaces, LF, trailing newline), atomically.
pub fn write_json_atomic<T: Serialize>(ctx: &Context, target: &Path, value: &T) -> CoreResult<()> {
    write_atomic(ctx, target, &to_json_bytes(value)?)
}

/// The JSON file form (manifest-schema §1): UTF-8, LF, 2-space indent, declared field order.
pub fn to_json_bytes<T: Serialize>(value: &T) -> CoreResult<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| CoreError::internal(format!("serialising JSON: {e}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Copies `source` to `target` through a temp file, never touching the source. Fails if
/// `target` exists. Returns the content hash and size.
pub fn copy_atomic_new(ctx: &Context, source: &Path, target: &Path) -> CoreResult<(String, u64)> {
    let tmp = temp_path(ctx, target);
    let result = (|| -> io::Result<(String, u64)> {
        let mut src = File::open(source)?;
        let mut dst = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut size = 0u64;
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            dst.write_all(&buf[..n])?;
            size += n as u64;
        }
        dst.sync_all()?;
        drop(dst);
        ctx.faults.hit(FaultPoint::AfterTemp, target)?;
        if target.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "target appeared during the copy",
            ));
        }
        rename_with_retry(&tmp, target)?;
        ctx.faults.hit(FaultPoint::AfterRename, target)?;
        Ok((format_hash(&hasher.finalize()), size))
    })();
    result.map_err(|e| {
        let _ = fs::remove_file(&tmp);
        CoreError::io(format!("copying to {}", target.display()), &e)
    })
}

fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let start = std::time::Instant::now();
    let mut delay = Duration::from_millis(25);
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if is_locked(&e) && start.elapsed() < LOCK_RETRY_FOR => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(500));
            }
            Err(e) => return Err(e),
        }
    }
}

/// `sha256:<hex>` of a file, streamed (#38).
pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format_hash(&hasher.finalize()))
}

fn format_hash(digest: &[u8]) -> String {
    let mut s = String::with_capacity(7 + digest.len() * 2);
    s.push_str("sha256:");
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Deletes temp files (names containing [`TEMP_MARKER`]) in `dir` and its subfolders.
/// Returns how many were removed.
pub fn remove_temp_files(dir: &Path) -> usize {
    let mut removed = 0;
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            removed += remove_temp_files(&path);
        } else if entry.file_name().to_string_lossy().contains(TEMP_MARKER)
            && fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{FaultAction, Faults};
    use crate::time::Timestamp;

    fn ctx(dir: &Path) -> Context {
        Context::deterministic(dir.to_path_buf(), Timestamp(0))
    }

    #[test]
    fn fr_prj_18_write_atomic_replaces_content() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let target = dir.path().join("manifest.json");
        write_atomic(&c, &target, b"one").unwrap();
        write_atomic(&c, &target, b"two").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"two");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "no temp left");
    }

    #[test]
    fn fr_prj_20_fault_after_temp_keeps_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = ctx(dir.path());
        let target = dir.path().join("manifest.json");
        write_atomic(&c, &target, b"before").unwrap();
        c.faults = Faults::parse("after_temp", FaultAction::Error).unwrap();
        assert!(write_atomic(&c, &target, b"after").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"before");
        assert_eq!(remove_temp_files(dir.path()), 0, "temp cleaned on error");
    }

    #[test]
    fn fr_prj_20_remove_temp_files_recurses() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("revisions")).unwrap();
        fs::write(dir.path().join(".manifest.json.pf-tmp-abc"), b"x").unwrap();
        fs::write(dir.path().join("revisions/.r.json.pf-tmp-def"), b"x").unwrap();
        fs::write(dir.path().join("revisions/r.json"), b"x").unwrap();
        assert_eq!(remove_temp_files(dir.path()), 2);
        assert!(dir.path().join("revisions/r.json").exists());
    }

    #[test]
    fn fr_prj_09_copy_atomic_new_never_overwrites_and_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let c = ctx(dir.path());
        let src = dir.path().join("src.bin");
        fs::write(&src, b"abc").unwrap();
        let dst = dir.path().join("dst.bin");
        let (hash, size) = copy_atomic_new(&c, &src, &dst).unwrap();
        assert_eq!(size, 3);
        assert_eq!(
            hash,
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(hash, hash_file(&dst).unwrap());
        assert!(copy_atomic_new(&c, &src, &dst).is_err());
    }

    #[test]
    fn json_is_pretty_lf_with_trailing_newline() {
        let bytes = to_json_bytes(&serde_json::json!({"a": [1]})).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "{\n  \"a\": [\n    1\n  ]\n}\n"
        );
    }
}

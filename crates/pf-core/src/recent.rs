//! `<data-dir>/recent.json`: recently opened asset projects (local only, absolute paths).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::context::Context;
use crate::error::CoreResult;
use crate::fsx;
use crate::time::Timestamp;

pub const RECENT_MAX: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentEntry {
    /// Absolute path of the original.
    pub path: PathBuf,
    pub name: String,
    pub opened_at: Timestamp,
}

fn recent_path(data_dir: &Path) -> PathBuf {
    data_dir.join("recent.json")
}

/// The list, newest first. A missing or unreadable file is an empty list (it's a convenience).
pub fn load(ctx: &Context) -> Vec<RecentEntry> {
    std::fs::read(recent_path(&ctx.data_dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Moves `path` to the front, keeping at most [`RECENT_MAX`] entries.
pub fn touch(ctx: &Context, path: &Path, name: &str) -> CoreResult<Vec<RecentEntry>> {
    let mut list = load(ctx);
    list.retain(|e| e.path != path);
    list.insert(
        0,
        RecentEntry {
            path: path.to_path_buf(),
            name: name.to_owned(),
            opened_at: ctx.now(),
        },
    );
    list.truncate(RECENT_MAX);
    fsx::write_json_atomic(ctx, &recent_path(&ctx.data_dir), &list)?;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_dedupes_orders_newest_first_and_caps() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::deterministic(dir.path().to_path_buf(), Timestamp(0));
        for i in 0..25 {
            touch(
                &ctx,
                Path::new(&format!("/lib/{i}.png")),
                &format!("{i}.png"),
            )
            .unwrap();
        }
        touch(&ctx, Path::new("/lib/3.png"), "3.png").unwrap();
        let list = load(&ctx);
        assert_eq!(list.len(), RECENT_MAX);
        assert_eq!(list[0].name, "3.png");
        assert_eq!(list[1].name, "24.png");
        assert_eq!(list.iter().filter(|e| e.name == "3.png").count(), 1);
    }

    #[test]
    fn corrupt_file_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::deterministic(dir.path().to_path_buf(), Timestamp(0));
        std::fs::write(recent_path(dir.path()), b"[oops").unwrap();
        assert!(load(&ctx).is_empty());
    }
}

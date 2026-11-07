//! The model list + capabilities cache (FR-PRV-25): `<data-dir>/cache/provider/models.json`,
//! a rebuildable local cache. Written temp + rename. A corrupt file is deleted (and the next
//! online call refetches); a locked one is skipped with a warning (spec §11).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Capabilities, ModelSummary};

const CACHE_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CacheFile {
    pub v: u32,
    pub models: Vec<ModelSummary>,
    pub capabilities: BTreeMap<String, Capabilities>,
}

/// The cache, or none (tests that don't need one).
#[derive(Debug, Clone, Default)]
pub struct ModelCache {
    path: Option<PathBuf>,
}

impl ModelCache {
    /// The cache under `<dir>` (normally `<data-dir>/cache/provider/`).
    pub fn in_dir(dir: &Path) -> Self {
        ModelCache {
            path: Some(dir.join("models.json")),
        }
    }

    pub fn none() -> Self {
        ModelCache { path: None }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The cached data. A corrupt or foreign-version file is deleted and reads as empty.
    pub fn load(&self) -> Option<CacheFile> {
        let path = self.path.as_ref()?;
        let bytes = std::fs::read(path).ok()?;
        match serde_json::from_slice::<CacheFile>(&bytes) {
            Ok(f) if f.v == CACHE_VERSION => Some(f),
            _ => {
                tracing::warn!(path = %path.display(), "provider cache is corrupt; deleting it");
                if let Err(e) = std::fs::remove_file(path) {
                    tracing::warn!(error = %e, "could not delete the corrupt provider cache");
                }
                None
            }
        }
    }

    /// Writes the cache (temp + rename). Failures only warn: the cache is rebuildable.
    pub fn store(&self, file: &CacheFile) {
        let Some(path) = &self.path else { return };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
            let bytes = serde_json::to_vec_pretty(&CacheFile {
                v: CACHE_VERSION,
                ..file.clone()
            })
            .map_err(std::io::Error::other)?;
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, path).inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            })
        };
        if let Err(e) = write() {
            tracing::warn!(path = %path.display(), error = %e, "provider cache not written (locked?); skipping");
        }
    }

    /// Updates one model's capabilities.
    pub fn put_capabilities(&self, caps: &Capabilities) {
        if self.path.is_none() {
            return;
        }
        let mut f = self.load().unwrap_or_default();
        f.capabilities.insert(caps.model_id.clone(), caps.clone());
        self.store(&f);
    }
}

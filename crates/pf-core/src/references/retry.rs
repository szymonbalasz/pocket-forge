//! Retry (FR-REF-26, FR-RGN-44): a job's `references[]` turned back into the draft selection.
//! Its packs are picked again, its items are `on` with their roles and every other available
//! item is `off`. Each item is found by `pack_id` (falling back to the pack name) and file name.
//! Different bytes mark it `changed since NNN` (still `on`; the current bytes are sent); a file
//! that can't be found stays in the draft, `off`, and resolves as `missing`.

use std::collections::HashSet;
use std::path::Path;

use serde_json::json;

use super::prepare::{JobReference, sha256};
use super::resolve::{
    self, CHANGED, DraftItem, DraftPack, DraftReferences, ItemState, ProbeCache, RETRY_OF, Source,
};
use super::*;

/// The draft `references` for a Retry of a job that sent `rows`. `label` is the job's
/// iteration `NNN`. `hash_of` gives a file's current content hash (a session cache hit or a
/// read); it is only asked for local files, so nothing is hydrated. `None` when the model has
/// no reference slots and the job sent none (the block stays hidden).
#[allow(clippy::too_many_arguments)]
pub fn retry_references(
    ctx: &Context,
    settings: &Settings,
    probes: &ProbeCache,
    project_dir: &Path,
    has_slots: bool,
    rows: &[JobReference],
    label: &str,
    hash_of: impl Fn(&Path) -> Option<String>,
) -> Option<DraftReferences> {
    if !has_slots && rows.is_empty() {
        return None;
    }
    // The job's packs, in first-use order.
    let mut packs: Vec<DraftPack> = vec![];
    for r in rows {
        if let (Source::Pack, Some(id)) = (r.source, r.pack_id)
            && !packs.iter().any(|p| p.pack_id == id)
        {
            packs.push(DraftPack {
                pack_id: id,
                name: r.pack_name.clone().unwrap_or_default(),
                extra: Extra::new(),
            });
        }
    }
    let picked = resolve::find_picked(ctx, settings, &packs);
    let available = resolve::available_items(ctx, probes, project_dir, &picked);

    let mut items = vec![];
    let mut sent = HashSet::new();
    for r in rows {
        let file = r.path.rsplit('/').next().unwrap_or(&r.path);
        let key = match (r.source, r.pack_id) {
            (Source::Project, _) => resolve::project_key(file),
            (Source::Pack, Some(id)) => {
                let now = picked
                    .iter()
                    .find(|p| p.draft.pack_id == id)
                    .and_then(|p| p.found.as_ref())
                    .map_or(id, |l| l.json.pack_id);
                resolve::pack_key(now, file)
            }
            (Source::Pack, None) => continue,
        };
        if !sent.insert(key.clone()) {
            continue;
        }
        let found = available.iter().find(|a| a.key == key);
        let changed = found.is_some_and(|a| {
            a.state == ItemState::Ok && hash_of(&a.path).is_some_and(|h| h != r.content_hash)
        });
        let mut extra = Extra::new();
        extra.insert(RETRY_OF.into(), json!(label));
        if changed {
            extra.insert(CHANGED.into(), json!(true));
        }
        items.push(DraftItem {
            key,
            on: found.is_some(),
            role: r.role,
            extra,
        });
    }
    for a in &available {
        if !sent.contains(&a.key) {
            items.push(DraftItem {
                key: a.key.clone(),
                on: false,
                role: None,
                extra: Extra::new(),
            });
        }
    }
    let packs = picked
        .into_iter()
        .map(|p| match p.found {
            Some(loc) => DraftPack {
                pack_id: loc.json.pack_id,
                name: loc.name,
                extra: p.draft.extra,
            },
            None => p.draft,
        })
        .collect();
    Some(DraftReferences {
        packs,
        items,
        extra: Extra::new(),
    })
}

/// A local file's content hash: the session cache's, else read and hashed. Cloud-only files
/// are never read here (FR-REF-26 must not hydrate).
pub fn current_hash(core: &crate::Core, path: &Path) -> Option<String> {
    if core.ctx.fs.is_cloud_only(path) {
        return None;
    }
    core.refs
        .prepared
        .content_hash_of(path)
        .or_else(|| std::fs::read(path).ok().map(|b| sha256(&b)))
}

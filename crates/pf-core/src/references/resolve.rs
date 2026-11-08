//! `ref_resolve` (FR-REF-15 … 19): reconciles the draft's `references` with the available
//! items and the model's slots. Reads directory listings and `pack.json` files only; image
//! headers are probed once per (path, size, mtime) and remembered for the session.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use pf_provider::ReferenceRole;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use super::packs::ImageState;
use super::*;

// ---------------------------------------------------------------- slots

/// A model's reference slot with its limits (FR-PRV-31, R49): `name`, `role`, `min`–`max`,
/// `cost_affecting`, `array`.
pub use pf_provider::Slot;

/// A slot as the block names it (walk R7): its role ("Style"), else "References". Messages
/// shown to the user use this, never the provider's input name (`style_refs`).
pub fn slot_label(slot: &Slot) -> String {
    match slot.role {
        Some(ReferenceRole::Style) => "Style",
        Some(ReferenceRole::Structure) => "Structure",
        Some(ReferenceRole::Identity) => "Identity",
        None => "References",
    }
    .to_owned()
}

/// The slot an item with `role` goes to: the slot with that role, else the first.
fn slot_index(slots: &[Slot], role: Option<ReferenceRole>) -> usize {
    role.and_then(|r| slots.iter().position(|s| s.role == Some(r)))
        .unwrap_or(0)
}

/// The item's role: kept if the model has a slot for it, else its pack role if the model has a
/// slot for that, else the first slot's role (FR-REF-17).
fn pick_role(
    slots: &[Slot],
    kept: Option<ReferenceRole>,
    pack_role: Option<ReferenceRole>,
) -> Option<ReferenceRole> {
    let has = |r: ReferenceRole| slots.iter().any(|s| s.role == Some(r));
    kept.filter(|r| has(*r))
        .or(pack_role.filter(|r| has(*r)))
        .or_else(|| slots.first().and_then(|s| s.role))
}

// ---------------------------------------------------------------- draft shape

/// A picked pack in the draft (schema §4.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftPack {
    pub pack_id: Ulid,
    pub name: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// One item's state in the draft (schema §4.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftItem {
    pub key: String,
    pub on: bool,
    #[serde(default)]
    pub role: Option<ReferenceRole>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Set by Retry (FR-REF-26, schema §4.5): the iteration (`NNN`) an item was restored from.
pub const RETRY_OF: &str = "retry_of";
/// Set by Retry when the file's bytes differ from what the job sent (FR-REF-26).
pub const CHANGED: &str = "changed";

impl DraftItem {
    /// The iteration this item was restored from by Retry, if any.
    pub fn retry_of(&self) -> Option<&str> {
        self.extra.get(RETRY_OF).and_then(serde_json::Value::as_str)
    }

    /// `NNN` when Retry found the file changed since that job (badge `changed_since:<NNN>`).
    pub fn changed_since(&self) -> Option<&str> {
        let changed = self.extra.get(CHANGED).and_then(serde_json::Value::as_bool);
        if changed == Some(true) {
            self.retry_of()
        } else {
            None
        }
    }
}

/// Draft `references` (schema §4.5): `{packs, items}`; `null` = REF's defaults.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DraftReferences {
    #[serde(default)]
    pub packs: Vec<DraftPack>,
    #[serde(default)]
    pub items: Vec<DraftItem>,
    #[serde(flatten)]
    pub extra: Extra,
}

// ---------------------------------------------------------------- available items

/// Where an item comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Project,
    Pack,
}

/// An item's state for the block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Ok,
    CloudOnly,
    Corrupt,
    Missing,
}

/// A file that could be sent (FR-REF-16).
#[derive(Debug, Clone, PartialEq)]
pub struct Available {
    pub key: String,
    pub source: Source,
    pub pack_id: Option<Ulid>,
    pub pack_name: Option<String>,
    pub file: String,
    /// Absolute path.
    pub path: PathBuf,
    /// The job-record path: `references/<file>` or `References/<pack>/<file>` (FR-REF-25).
    pub rel_path: String,
    /// The pack's default role for the file.
    pub pack_role: Option<ReferenceRole>,
    pub state: ItemState,
}

pub fn project_key(file: &str) -> String {
    format!("project:{file}")
}

pub fn pack_key(pack_id: Ulid, file: &str) -> String {
    format!("pack:{pack_id}:{file}")
}

/// Header probes remembered per (path, size, mtime), so a repeated resolve reads no image
/// bytes (§12).
#[derive(Debug, Default)]
pub struct ProbeCache(Mutex<HashMap<(PathBuf, u64, i64), ItemState>>);

const PROBE_CACHE_MAX: usize = 20_000;

impl ProbeCache {
    fn state(&self, ctx: &Context, path: &Path, meta: &std::fs::Metadata) -> ItemState {
        if ctx.fs.is_cloud_only(path) {
            return ItemState::CloudOnly;
        }
        let key = (
            path.to_path_buf(),
            meta.len(),
            crate::project::mtime_of(meta).millis(),
        );
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = m.get(&key) {
            return *s;
        }
        let s = match packs::image_state(ctx, path).0 {
            ImageState::Ok => ItemState::Ok,
            ImageState::CloudOnly => ItemState::CloudOnly,
            ImageState::Corrupt | ImageState::Unsupported => ItemState::Corrupt,
        };
        if m.len() >= PROBE_CACHE_MAX {
            m.clear();
        }
        m.insert(key, s);
        s
    }
}

/// A picked pack as found now.
#[derive(Debug, Clone)]
pub struct PickedPack {
    pub draft: DraftPack,
    /// `None` when neither the id nor the name is found (deleted, root unavailable).
    pub found: Option<PackLoc>,
}

/// Finds each picked pack by `pack_id`, falling back to its name (FR-REF-26). One pass over
/// the packs' `pack.json` files, whatever the number of picks.
pub fn find_picked(ctx: &Context, settings: &Settings, packs: &[DraftPack]) -> Vec<PickedPack> {
    if packs.is_empty() {
        return vec![];
    }
    let mut all: Vec<PackLoc> = vec![];
    for root in available_roots(ctx, settings) {
        for (name, dir) in pack_dirs(&root) {
            let file = dir.join(PACK_FILE);
            if crate::library::hydrate_if_needed(ctx, &file).is_err() {
                continue;
            }
            if let Ok(json) = read_pack_json(&file) {
                all.push(PackLoc {
                    root: root.clone(),
                    dir,
                    name,
                    json,
                });
            }
        }
    }
    packs
        .iter()
        .map(|p| PickedPack {
            draft: p.clone(),
            found: all
                .iter()
                .find(|l| l.json.pack_id == p.pack_id)
                .or_else(|| all.iter().find(|l| naming::same_name(&l.name, &p.name)))
                .cloned(),
        })
        .collect()
}

/// FR-REF-16: the project's `references/` images by name, then each found picked pack in pick
/// order with its images by name.
pub fn available_items(
    ctx: &Context,
    probes: &ProbeCache,
    project_dir: &Path,
    picked: &[PickedPack],
) -> Vec<Available> {
    let mut out = vec![];
    let refs = project_dir.join("references");
    for (file, meta) in image_files(&refs) {
        let path = refs.join(&file);
        out.push(Available {
            key: project_key(&file),
            source: Source::Project,
            pack_id: None,
            pack_name: None,
            rel_path: format!("references/{file}"),
            pack_role: None,
            state: probes.state(ctx, &path, &meta),
            path,
            file,
        });
    }
    let mut seen = HashSet::new();
    for p in picked {
        let Some(loc) = &p.found else { continue };
        if !seen.insert(loc.json.pack_id) {
            continue;
        }
        for (file, meta) in image_files(&loc.dir) {
            let path = loc.dir.join(&file);
            out.push(Available {
                key: pack_key(loc.json.pack_id, &file),
                source: Source::Pack,
                pack_id: Some(loc.json.pack_id),
                pack_name: Some(loc.name.clone()),
                rel_path: loc.rel_path(&file),
                pack_role: loc.json.role_of(&file),
                state: probes.state(ctx, &path, &meta),
                path,
                file,
            });
        }
    }
    out
}

// ---------------------------------------------------------------- reconcile

/// A slot in the block header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SlotOut {
    pub name: String,
    pub role: Option<ReferenceRole>,
    pub min: u32,
    pub max: u32,
    pub cost_affecting: bool,
    /// Items ticked for this slot.
    pub on: u32,
}

/// One item in the block.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemOut {
    pub key: String,
    pub source: Source,
    pub pack_id: Option<Ulid>,
    pub pack_name: Option<String>,
    pub file: String,
    pub role: Option<ReferenceRole>,
    /// The slot it is sent in.
    pub slot: String,
    pub on: bool,
    pub state: ItemState,
    /// `cloud`, `missing`, `corrupt`, `duplicate`, `changed_since:<NNN>` (FR-REF-26).
    pub badges: Vec<String>,
    /// False for missing and corrupt items (they can't be ticked).
    pub checkable: bool,
    /// EDT `library` key and URL for the thumbnail, set by the command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_url: Option<String>,
    /// Absolute path (internal).
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

/// A notice shown under the block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notice {
    /// `dropped` | `limit_lowered` | `pack_unavailable` | `duplicate`.
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// What `ref_resolve` returns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resolved {
    /// False when the model has no slots: the block is hidden and nothing is sent (FR-REF-15).
    pub visible: bool,
    pub slots: Vec<SlotOut>,
    pub items: Vec<ItemOut>,
    pub notices: Vec<Notice>,
    /// The reconciled draft value, for PRJ's debounced draft save (FR-REF-20).
    pub references: DraftReferences,
    /// Every slot is at or over its `min` (submit allowed, FR-REF-18).
    pub ready: bool,
    /// Read-only project (FR-REF-27): shown but disabled.
    pub read_only: bool,
    /// Items the draft has `on` that can't be sent now (the file is gone or corrupt), with their
    /// state. `prepare` fails on these (FR-REF-21); the block shows them (notice or badge) and
    /// the reconciled draft turns them off. Items of a picked pack that can't be found are not
    /// here: they stay `on` in the draft for when it comes back, shown `missing` under the
    /// persistent `pack_unavailable` notice (Appendix A step 9).
    #[serde(skip)]
    pub lost_on: Vec<(String, ItemState)>,
}

/// FR-REF-17 … 19, pure. `prev` = the draft's `references` (`None` = defaults).
pub fn reconcile(
    slots: &[Slot],
    available: &[Available],
    picked: &[PickedPack],
    prev: Option<&DraftReferences>,
) -> Resolved {
    let mut notices = vec![];
    let mut lost_on: Vec<(String, ItemState)> = vec![];
    if slots.is_empty() {
        return Resolved {
            visible: false,
            slots: vec![],
            items: vec![],
            notices,
            references: prev.cloned().unwrap_or_default(),
            ready: true,
            read_only: false,
            lost_on: vec![],
        };
    }
    // Keys of a pack found by name under a new id are carried over (FR-REF-26).
    let mut rekey: HashMap<String, String> = HashMap::new();
    for p in picked {
        if let Some(loc) = &p.found
            && loc.json.pack_id != p.draft.pack_id
        {
            rekey.insert(
                format!("pack:{}:", p.draft.pack_id),
                format!("pack:{}:", loc.json.pack_id),
            );
        }
    }
    let map_key = |k: &str| -> String {
        rekey
            .iter()
            .find_map(|(old, new)| {
                k.strip_prefix(old.as_str())
                    .map(|rest| format!("{new}{rest}"))
            })
            .unwrap_or_else(|| k.to_owned())
    };
    let prev_items: HashMap<String, &DraftItem> = prev
        .map(|p| p.items.iter().map(|i| (map_key(&i.key), i)).collect())
        .unwrap_or_default();

    struct Row {
        out: ItemOut,
        slot: usize,
        kept: Option<bool>,
    }
    let mut rows: Vec<Row> = available
        .iter()
        .map(|a| {
            let prev = prev_items.get(&a.key);
            let role = pick_role(slots, prev.and_then(|p| p.role), a.pack_role);
            let slot = slot_index(slots, role);
            let checkable = matches!(a.state, ItemState::Ok | ItemState::CloudOnly);
            let mut badges = vec![];
            match a.state {
                ItemState::CloudOnly => badges.push("cloud".to_owned()),
                ItemState::Corrupt => badges.push("corrupt".to_owned()),
                ItemState::Missing => badges.push("missing".to_owned()),
                ItemState::Ok => {}
            }
            if let Some(n) = prev.and_then(|p| p.changed_since()) {
                badges.push(format!("changed_since:{n}"));
            }
            Row {
                out: ItemOut {
                    key: a.key.clone(),
                    source: a.source,
                    pack_id: a.pack_id,
                    pack_name: a.pack_name.clone(),
                    file: a.file.clone(),
                    role,
                    slot: slots[slot].name.clone(),
                    on: false,
                    state: a.state,
                    badges,
                    checkable,
                    thumb_key: None,
                    thumb_url: None,
                    path: Some(a.path.clone()),
                },
                slot,
                kept: prev.map(|p| p.on && checkable),
            }
        })
        .collect();
    // A ticked item that can't be ticked any more (now corrupt) is lost, not silently off.
    for r in &rows {
        if r.kept == Some(false) && prev_items.get(&r.out.key).is_some_and(|p| p.on) {
            lost_on.push((r.out.key.clone(), r.out.state));
        }
    }
    // Kept ticks first; over a (lowered) limit the last ones in list order go off.
    let mut count = vec![0u32; slots.len()];
    for r in rows.iter_mut() {
        if r.kept == Some(true) {
            if count[r.slot] < slots[r.slot].max {
                count[r.slot] += 1;
                r.out.on = true;
            } else {
                notices.push(Notice {
                    code: "limit_lowered",
                    message: format!(
                        "{} was unticked: {} takes at most {}",
                        r.out.file,
                        slot_label(&slots[r.slot]),
                        slots[r.slot].max
                    ),
                    key: Some(r.out.key.clone()),
                });
            }
        }
    }
    // New items are ticked while their slot has room.
    for r in rows.iter_mut() {
        if r.kept.is_none() && r.out.checkable && count[r.slot] < slots[r.slot].max {
            count[r.slot] += 1;
            r.out.on = true;
        }
    }
    let available_keys: HashSet<&str> = available.iter().map(|a| a.key.as_str()).collect();
    // Items of a picked pack that can't be found stay in the draft and show as missing.
    let mut missing_rows = vec![];
    let lost_packs: Vec<&PickedPack> = picked.iter().filter(|p| p.found.is_none()).collect();
    for p in &lost_packs {
        notices.push(Notice {
            code: "pack_unavailable",
            message: format!("The pack {} can't be found", p.draft.name),
            key: None,
        });
    }
    let mut kept_draft_items = vec![];
    if let Some(prev) = prev {
        for item in &prev.items {
            let key = map_key(&item.key);
            if available_keys.contains(key.as_str()) {
                continue;
            }
            let lost = lost_packs
                .iter()
                .find(|p| key.starts_with(&format!("pack:{}:", p.draft.pack_id)));
            if let Some(p) = lost {
                let file = key
                    .strip_prefix(&format!("pack:{}:", p.draft.pack_id))
                    .unwrap_or_default()
                    .to_owned();
                let slot = slot_index(slots, item.role);
                missing_rows.push(ItemOut {
                    key: key.clone(),
                    source: Source::Pack,
                    pack_id: Some(p.draft.pack_id),
                    pack_name: Some(p.draft.name.clone()),
                    file,
                    role: item.role,
                    slot: slots[slot].name.clone(),
                    on: false,
                    state: ItemState::Missing,
                    badges: vec!["missing".to_owned()],
                    checkable: false,
                    thumb_key: None,
                    thumb_url: None,
                    path: None,
                });
                kept_draft_items.push(DraftItem {
                    key,
                    ..item.clone()
                });
            } else if item.retry_of().is_some() {
                if item.on {
                    lost_on.push((key.clone(), ItemState::Missing));
                }
                // Restored by Retry (FR-REF-26): a file that can't be found shows as missing.
                let (source, pack_id, pack_name, file) = match key.strip_prefix("project:") {
                    Some(f) => (Source::Project, None, None, f.to_owned()),
                    None => {
                        let found = picked
                            .iter()
                            .filter_map(|p| p.found.as_ref())
                            .find(|l| key.starts_with(&format!("pack:{}:", l.json.pack_id)));
                        (
                            Source::Pack,
                            found.map(|l| l.json.pack_id),
                            found.map(|l| l.name.clone()),
                            key_file(&key).to_owned(),
                        )
                    }
                };
                let slot = slot_index(slots, item.role);
                missing_rows.push(ItemOut {
                    key: key.clone(),
                    source,
                    pack_id,
                    pack_name,
                    file,
                    role: item.role,
                    slot: slots[slot].name.clone(),
                    on: false,
                    state: ItemState::Missing,
                    badges: vec!["missing".to_owned()],
                    checkable: false,
                    thumb_key: None,
                    thumb_url: None,
                    path: None,
                });
                kept_draft_items.push(DraftItem {
                    key,
                    on: false,
                    ..item.clone()
                });
            } else {
                if item.on {
                    lost_on.push((key.clone(), ItemState::Missing));
                }
                notices.push(Notice {
                    code: "dropped",
                    message: format!("{} is no longer available", key_file(&key)),
                    key: Some(key),
                });
            }
        }
    }
    let slots_out: Vec<SlotOut> = slots
        .iter()
        .zip(&count)
        .map(|(s, &on)| SlotOut {
            name: s.name.clone(),
            role: s.role,
            min: s.min,
            max: s.max,
            cost_affecting: s.cost_affecting,
            on,
        })
        .collect();
    let ready = slots_out.iter().all(|s| s.on >= s.min);
    let mut items: Vec<ItemOut> = rows.into_iter().map(|r| r.out).collect();
    // R53: the draft keeps only roles the user chose (or Retry restored); `null` = the default,
    // re-derived per model (FR-REF-17), so a model change can apply a pack's role.
    let mut draft_items: Vec<DraftItem> = items
        .iter()
        .map(|i| {
            let prev = prev_items.get(&i.key);
            DraftItem {
                key: i.key.clone(),
                on: i.on,
                role: prev.and_then(|p| p.role),
                extra: prev.map(|p| p.extra.clone()).unwrap_or_default(),
            }
        })
        .collect();
    draft_items.extend(kept_draft_items);
    items.extend(missing_rows);
    let packs = picked
        .iter()
        .map(|p| match &p.found {
            Some(loc) => DraftPack {
                pack_id: loc.json.pack_id,
                name: loc.name.clone(),
                extra: p.draft.extra.clone(),
            },
            None => p.draft.clone(),
        })
        .collect();
    Resolved {
        visible: true,
        slots: slots_out,
        items,
        notices,
        references: DraftReferences {
            packs,
            items: draft_items,
            extra: prev.map(|p| p.extra.clone()).unwrap_or_default(),
        },
        ready,
        read_only: false,
        lost_on,
    }
}

fn key_file(key: &str) -> &str {
    key.rsplit_once(':').map_or(key, |(_, f)| f)
}

/// Marks items whose prepared content hash (from the session cache) equals an earlier item's
/// as `duplicate` (FR-REF-21). Reads nothing from disk.
pub fn mark_duplicates(items: &mut [ItemOut], hash_of: impl Fn(&Path) -> Option<String>) {
    let mut seen: HashSet<String> = HashSet::new();
    for item in items.iter_mut() {
        let Some(h) = item.path.as_deref().and_then(&hash_of) else {
            continue;
        };
        if !seen.insert(h) {
            item.badges.push("duplicate".to_owned());
        }
    }
}

/// Everything `ref_resolve` and `prepare_for_send` share: slots, the picked packs and the
/// available items for the open project.
pub fn resolve(
    ctx: &Context,
    settings: &Settings,
    probes: &ProbeCache,
    project_dir: &Path,
    slots: &[Slot],
    prev: Option<&DraftReferences>,
) -> Resolved {
    let picked = find_picked(ctx, settings, prev.map_or(&[][..], |p| &p.packs));
    let available = if slots.is_empty() {
        vec![]
    } else {
        available_items(ctx, probes, project_dir, &picked)
    };
    reconcile(slots, &available, &picked, prev)
}

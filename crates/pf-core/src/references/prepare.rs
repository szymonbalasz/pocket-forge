//! Prepare for send (FR-REF-21 … 25): the Rust API RGN calls in estimate and submit. For each
//! ticked item: hydrate it if it is cloud-only, read it, hash it, apply the sent-bytes rule,
//! dedupe identical bytes and check the slot limits. Nothing is dropped silently; any item that
//! can't be read fails the whole call before a provider call is made.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use pf_provider::{Capabilities, ReferenceImage, ReferenceRole};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use super::resolve::{self, DraftReferences, ItemState, Notice, ProbeCache, Slot, Source};
use super::*;

/// Long-side cap for sent references (OQ-REF-04 A).
pub const SENT_MAX_SIDE: u32 = 2048;
/// Prepared-bytes cache cap (FR-REF-23).
pub const CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
/// How long a locked file is retried before `locked` (§11).
const LOCK_RETRY_FOR: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------- job record row

/// `sent{w, h, bytes, hash, resized}` of a job reference (FR-REF-25).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentInfo {
    pub w: u32,
    pub h: u32,
    pub bytes: u64,
    pub hash: String,
    pub resized: bool,
}

/// One row of the job's `references[]` (schema §4.6, FR-REF-25), in request order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobReference {
    pub source: Source,
    pub pack_id: Option<Ulid>,
    pub pack_name: Option<String>,
    /// `references/<file>` or `References/<pack>/<file>`.
    pub path: String,
    /// `sha256:` of the file as read.
    pub content_hash: String,
    pub slot: String,
    pub role: Option<ReferenceRole>,
    pub sent: SentInfo,
}

/// A reference ready for `JobRequest.references` ([`SentReference::to_provider`]) plus its
/// job row.
#[derive(Debug, Clone, PartialEq)]
pub struct SentReference {
    pub key: String,
    pub slot: String,
    pub role: Option<ReferenceRole>,
    /// The bytes as sent (PNG, JPEG or WebP).
    pub bytes: Arc<Vec<u8>>,
    /// `sha256:` of `bytes`.
    pub hash: String,
    pub row: JobReference,
}

/// What `prepare_for_send` returns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prepared {
    pub sent: Vec<SentReference>,
    /// `duplicate` notices for items sent once.
    pub notices: Vec<Notice>,
}

impl SentReference {
    /// The PRV request form (FR-PRV-33, R49).
    pub fn to_provider(&self) -> ReferenceImage {
        ReferenceImage {
            slot: self.slot.clone(),
            role: self.role,
            bytes: self.bytes.as_ref().clone(),
            hash: self.hash.clone(),
        }
    }
}

impl Prepared {
    /// `JobRequest.references`, in request order (FR-PRV-33).
    pub fn provider_references(&self) -> Vec<ReferenceImage> {
        self.sent.iter().map(SentReference::to_provider).collect()
    }

    /// The job's `references[]` (FR-REF-25).
    pub fn rows(&self) -> Vec<JobReference> {
        self.sent.iter().map(|s| s.row.clone()).collect()
    }
}

/// FR-REF-24: the ordered `(slot, sent hash)` list of cost-affecting slots, for the estimate
/// id. References in other slots never change it.
pub fn estimate_inputs(sent: &[SentReference], slots: &[Slot]) -> Vec<(String, String)> {
    sent.iter()
        .filter(|s| slots.iter().any(|l| l.name == s.slot && l.cost_affecting))
        .map(|s| (s.slot.clone(), s.hash.clone()))
        .collect()
}

// ---------------------------------------------------------------- cache

/// Cache key: (path, size, mtime) of the file plus the limits that shape the sent bytes.
type CacheKey = (PathBuf, u64, SystemTime, u32, u64);

#[derive(Debug, Clone)]
struct Entry {
    content_hash: String,
    sent: Arc<Vec<u8>>,
    sent_hash: String,
    w: u32,
    h: u32,
    resized: bool,
    used: u64,
}

#[derive(Debug, Default)]
struct Lru {
    map: HashMap<CacheKey, Entry>,
    bytes: usize,
    tick: u64,
}

/// Prepared references, in memory per session, keyed by (path, size, mtime), ≤ 64 MiB, least
/// recently used out first (FR-REF-23).
#[derive(Debug, Default)]
pub struct PreparedCache {
    lru: Mutex<Lru>,
    pub hits: AtomicU64,
    pub misses: AtomicU64,
}

impl PreparedCache {
    fn get(&self, key: &CacheKey) -> Option<Entry> {
        let mut l = self.lru.lock().unwrap_or_else(|e| e.into_inner());
        l.tick += 1;
        let tick = l.tick;
        let e = l.map.get_mut(key)?;
        e.used = tick;
        Some(e.clone())
    }

    fn put(&self, key: CacheKey, mut entry: Entry) {
        let size = entry.sent.len();
        if size > CACHE_MAX_BYTES {
            return;
        }
        let mut l = self.lru.lock().unwrap_or_else(|e| e.into_inner());
        l.tick += 1;
        entry.used = l.tick;
        if let Some(old) = l.map.insert(key, entry) {
            l.bytes -= old.sent.len();
        }
        l.bytes += size;
        while l.bytes > CACHE_MAX_BYTES {
            let Some(oldest) = l
                .map
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(e) = l.map.remove(&oldest) {
                l.bytes -= e.sent.len();
            }
        }
    }

    /// The cached content hash of a file as it is now (any limits), without reading it.
    pub fn content_hash_of(&self, path: &Path) -> Option<String> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        let mtime = meta.modified().ok()?;
        let l = self.lru.lock().unwrap_or_else(|e| e.into_inner());
        l.map
            .iter()
            .find(|(k, _)| k.0 == path && k.1 == meta.len() && k.2 == mtime)
            .map(|(_, e)| e.content_hash.clone())
    }

    /// Bytes held (tests, diagnostics).
    pub fn bytes(&self) -> usize {
        self.lru.lock().unwrap_or_else(|e| e.into_inner()).bytes
    }
}

/// REF's per-session memory: header probes and prepared references.
#[derive(Debug, Default)]
pub struct RefSession {
    pub probes: ProbeCache,
    pub prepared: PreparedCache,
}

// ---------------------------------------------------------------- errors

/// Why an item can't be sent (FR-REF-21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    Missing,
    NotAvailableOffline,
    Corrupt,
    Locked,
}

fn unavailable(key: &str, reason: UnavailableReason) -> CoreError {
    // The file name, not the key (`pack:<id>:<file>`): the message is shown as is.
    let file = key.rsplit_once(':').map_or(key, |(_, f)| f);
    let why = match reason {
        UnavailableReason::Missing => "it can't be found",
        UnavailableReason::NotAvailableOffline => "it isn't available offline",
        UnavailableReason::Corrupt => "it can't be read",
        UnavailableReason::Locked => "another app is using it",
    };
    CoreError::new(
        ErrorCode::ReferenceUnavailable,
        format!("The reference {file} can't be sent: {why}"),
    )
    .with_detail(serde_json::json!({ "key": key, "reason": reason }))
}

fn required(slot: &Slot) -> CoreError {
    CoreError::new(
        ErrorCode::ReferencesRequired,
        format!(
            "{} needs at least {} reference(s)",
            resolve::slot_label(slot),
            slot.min
        ),
    )
    .with_detail(serde_json::json!({ "slot": slot.name, "min": slot.min }))
}

fn too_many(slot: &Slot) -> CoreError {
    CoreError::new(
        ErrorCode::TooManyReferences,
        format!(
            "{} takes at most {} reference(s)",
            resolve::slot_label(slot),
            slot.max
        ),
    )
    .with_detail(serde_json::json!({ "slot": slot.name, "max": slot.max }))
}

fn io_reason(e: &std::io::Error) -> UnavailableReason {
    if crate::error::is_locked(e) {
        UnavailableReason::Locked
    } else if e.kind() == std::io::ErrorKind::NotFound {
        UnavailableReason::Missing
    } else {
        UnavailableReason::Corrupt
    }
}

/// Reads a file, retrying a locked one for ≤ 3 s.
fn read_with_retry(path: &Path) -> std::io::Result<Vec<u8>> {
    let start = std::time::Instant::now();
    let mut delay = Duration::from_millis(25);
    loop {
        match std::fs::read(path) {
            Err(e) if crate::error::is_locked(&e) && start.elapsed() < LOCK_RETRY_FOR => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(500));
            }
            other => return other,
        }
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(7 + 64);
    s.push_str("sha256:");
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// ---------------------------------------------------------------- prepare

/// The limits a model puts on sent references (FR-REF-22): long side ≤ 2048 and ≤
/// `input_px.max`; bytes ≤ `max_bytes` (0 = no limit).
pub fn send_limits(caps: &Capabilities) -> (u32, u64) {
    let side = match caps.input_px.max {
        0 => SENT_MAX_SIDE,
        m => m.min(SENT_MAX_SIDE),
    };
    let bytes = if caps.max_bytes == 0 {
        u64::MAX
    } else {
        caps.max_bytes
    };
    (side, bytes)
}

/// FR-REF-21 … 23 with explicit inputs (no locks taken). `refs` is the draft's `references`
/// (`None` = defaults); `slots` the model's reference slots.
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    ctx: &Context,
    session: &RefSession,
    settings: &Settings,
    project_dir: &Path,
    slots: &[Slot],
    max_side: u32,
    max_bytes: u64,
    refs: Option<&DraftReferences>,
) -> CoreResult<Prepared> {
    let resolved = resolve::resolve(ctx, settings, &session.probes, project_dir, slots, refs);
    if !resolved.visible {
        return Ok(Prepared::default());
    }
    // A ticked item that is gone (deleted, moved, its pack lost) or now corrupt fails the call:
    // the reconcile turns it off for the block, but it must never be dropped silently here.
    if let Some((key, state)) = resolved.lost_on.first() {
        let reason = match state {
            ItemState::Corrupt => UnavailableReason::Corrupt,
            _ => UnavailableReason::Missing,
        };
        return Err(unavailable(key, reason));
    }
    let on: Vec<_> = resolved.items.iter().filter(|i| i.on).collect();
    // Limits first, so nothing is hydrated or read for a request that can't be sent.
    for s in slots {
        let n = on.iter().filter(|i| i.slot == s.name).count() as u32;
        if n < s.min {
            return Err(required(s));
        }
        if n > s.max {
            return Err(too_many(s));
        }
    }
    let mut out = Prepared::default();
    let mut by_hash: HashMap<String, String> = HashMap::new();
    for item in on {
        let path = item
            .path
            .as_deref()
            .ok_or_else(|| unavailable(&item.key, UnavailableReason::Missing))?;
        if ctx.fs.is_cloud_only(path) && ctx.fs.hydrate(path).is_err() {
            return Err(unavailable(
                &item.key,
                UnavailableReason::NotAvailableOffline,
            ));
        }
        let entry = prepare_one(session, &item.key, path, max_side, max_bytes)?;
        if let Some(first) = by_hash.get(&entry.content_hash) {
            out.notices.push(Notice {
                code: "duplicate",
                message: format!(
                    "{} is the same image as {first}; it is sent once",
                    item.file
                ),
                key: Some(item.key.clone()),
            });
            continue;
        }
        by_hash.insert(entry.content_hash.clone(), item.file.clone());
        let row = JobReference {
            source: item.source,
            pack_id: item.pack_id,
            pack_name: item.pack_name.clone(),
            path: match item.source {
                Source::Project => format!("references/{}", item.file),
                Source::Pack => format!(
                    "{REFERENCES_DIR}/{}/{}",
                    item.pack_name.as_deref().unwrap_or_default(),
                    item.file
                ),
            },
            content_hash: entry.content_hash.clone(),
            slot: item.slot.clone(),
            role: item.role,
            sent: SentInfo {
                w: entry.w,
                h: entry.h,
                bytes: entry.sent.len() as u64,
                hash: entry.sent_hash.clone(),
                resized: entry.resized,
            },
        };
        out.sent.push(SentReference {
            key: item.key.clone(),
            slot: item.slot.clone(),
            role: item.role,
            bytes: entry.sent,
            hash: entry.sent_hash,
            row,
        });
    }
    // Duplicates sent once may leave a slot under its minimum.
    for s in slots {
        let n = out.sent.iter().filter(|r| r.slot == s.name).count() as u32;
        if n < s.min {
            return Err(required(s));
        }
    }
    Ok(out)
}

fn prepare_one(
    session: &RefSession,
    key: &str,
    path: &Path,
    max_side: u32,
    max_bytes: u64,
) -> CoreResult<Entry> {
    let meta = std::fs::metadata(path).map_err(|e| unavailable(key, io_reason(&e)))?;
    let mtime = meta
        .modified()
        .map_err(|e| unavailable(key, io_reason(&e)))?;
    let cache_key = (path.to_path_buf(), meta.len(), mtime, max_side, max_bytes);
    if let Some(e) = session.prepared.get(&cache_key) {
        session.prepared.hits.fetch_add(1, Ordering::Relaxed);
        return Ok(e);
    }
    session.prepared.misses.fetch_add(1, Ordering::Relaxed);
    let bytes = read_with_retry(path).map_err(|e| unavailable(key, io_reason(&e)))?;
    let content_hash = sha256(&bytes);
    let sent = pf_imaging::reference::prepare_reference(&bytes, max_side, max_bytes)
        .map_err(|_| unavailable(key, UnavailableReason::Corrupt))?;
    let entry = Entry {
        sent_hash: if sent.resized {
            sha256(&sent.bytes)
        } else {
            content_hash.clone()
        },
        content_hash,
        w: sent.width,
        h: sent.height,
        resized: sent.resized,
        sent: Arc::new(sent.bytes),
        used: 0,
    };
    // Keyed by what was read: a file that changed during the read is re-read next time.
    session.prepared.put(cache_key, entry.clone());
    Ok(entry)
}

/// `references::prepare_for_send(core, caps, &draft.references)` (§9): the open project's
/// references for `caps`. Takes the state lock briefly; don't call it while holding it.
pub fn prepare_for_send(
    core: &crate::Core,
    caps: &Capabilities,
    refs: Option<&DraftReferences>,
) -> CoreResult<Prepared> {
    let (dir, settings) = {
        let st = core.state();
        let dir = st
            .project
            .as_ref()
            .map(|p| p.dir.clone())
            .ok_or_else(|| CoreError::new(ErrorCode::NoProject, "no asset project is open"))?;
        (dir, st.settings.clone())
    };
    let (side, bytes) = send_limits(caps);
    prepare(
        &core.ctx,
        &core.refs,
        &settings,
        &dir,
        &caps.slots,
        side,
        bytes,
        refs,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(mib: usize) -> Entry {
        Entry {
            content_hash: String::new(),
            sent: Arc::new(vec![0u8; mib * 1024 * 1024]),
            sent_hash: String::new(),
            w: 1,
            h: 1,
            resized: false,
            used: 0,
        }
    }

    fn key(n: u64) -> CacheKey {
        (
            PathBuf::from(format!("{n}.png")),
            n,
            SystemTime::UNIX_EPOCH,
            2048,
            u64::MAX,
        )
    }

    #[test]
    fn fr_ref_23_cache_is_capped_at_64_mib_lru() {
        let c = PreparedCache::default();
        c.put(key(1), entry(20));
        c.put(key(2), entry(20));
        c.put(key(3), entry(20));
        assert!(c.get(&key(1)).is_some(), "1 is now the most recently used");
        c.put(key(4), entry(20));
        assert!(c.bytes() <= CACHE_MAX_BYTES);
        assert!(c.get(&key(2)).is_none(), "least recently used went first");
        assert!(c.get(&key(1)).is_some() && c.get(&key(4)).is_some());
        c.put(key(5), entry(65));
        assert!(
            c.get(&key(5)).is_none(),
            "an entry over the cap is never cached"
        );
        let other_limits = (key(1).0, 1, SystemTime::UNIX_EPOCH, 1024, u64::MAX);
        assert!(
            c.get(&other_limits).is_none(),
            "the limits are part of the key"
        );
    }
}

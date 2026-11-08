//! The context object: every injected seam (AGENTS §9). IDs, clock, randomness, the provider,
//! filesystem attributes, fault injection and the event sink all come from here, so tests and
//! fixtures run with seeded or fake implementations.

use std::collections::HashSet;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use pf_provider::{AnyProvider, CredentialStore, KeyringStore, MemoryStore, ProviderMode};
use serde_json::Value;
use ulid::Ulid;

use crate::time::Timestamp;

// ---------------------------------------------------------------- clock

/// The clock seam.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// Wall-clock UTC.
#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_system(SystemTime::now())
    }
}

/// `PF_FAKE_NOW`: starts at a given time and advances with real time (test builds only).
#[derive(Debug)]
pub struct OffsetClock {
    start: Timestamp,
    started: Instant,
}

impl OffsetClock {
    pub fn starting_at(start: Timestamp) -> Self {
        OffsetClock {
            start,
            started: Instant::now(),
        }
    }
}

impl Clock for OffsetClock {
    fn now(&self) -> Timestamp {
        self.start.plus(self.started.elapsed())
    }
}

/// A clock tests set and advance by hand.
#[derive(Debug)]
pub struct ManualClock(Mutex<Timestamp>);

impl ManualClock {
    pub fn new(t: Timestamp) -> Self {
        ManualClock(Mutex::new(t))
    }

    pub fn set(&self, t: Timestamp) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = t;
    }

    pub fn advance(&self, d: Duration) {
        let mut t = self.0.lock().unwrap_or_else(|e| e.into_inner());
        *t = t.plus(d);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// ---------------------------------------------------------------- randomness

/// The randomness seam. Not cryptographic: IDs and fake seeds only.
pub trait Rng: Send + Sync {
    fn next_u64(&self) -> u64;
}

/// SplitMix64. Seeded in tests; seeded from process entropy in the app.
#[derive(Debug)]
pub struct SeededRng(AtomicU64);

impl SeededRng {
    pub fn new(seed: u64) -> Self {
        SeededRng(AtomicU64::new(seed))
    }

    /// Seeded from the std hasher's per-process random keys, the time and the process id.
    pub fn from_entropy() -> Self {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        h.write_u32(std::process::id());
        SeededRng::new(h.finish())
    }
}

impl Rng for SeededRng {
    fn next_u64(&self) -> u64 {
        let mut z = self
            .0
            .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
            .wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

// ---------------------------------------------------------------- ids

/// The ID seam. IDs are ULIDs (manifest-schema §1).
pub trait IdGen: Send + Sync {
    fn next_id(&self, clock: &dyn Clock, rng: &dyn Rng) -> Ulid;
}

/// ULIDs from the clock and randomness seams.
#[derive(Debug, Default)]
pub struct UlidGen;

impl IdGen for UlidGen {
    fn next_id(&self, clock: &dyn Clock, rng: &dyn Rng) -> Ulid {
        let random = (u128::from(rng.next_u64()) << 64 | u128::from(rng.next_u64())) >> 48;
        Ulid::from_parts(clock.now().millis().max(0) as u64, random)
    }
}

/// Sequential ULIDs (`…0001`, `…0002`, …) for golden tests.
#[derive(Debug, Default)]
pub struct SequentialIds(AtomicU64);

impl IdGen for SequentialIds {
    fn next_id(&self, _clock: &dyn Clock, _rng: &dyn Rng) -> Ulid {
        Ulid::from_parts(0, u128::from(self.0.fetch_add(1, Ordering::Relaxed) + 1))
    }
}

// ---------------------------------------------------------------- filesystem attributes

/// The filesystem-attributes seam (FR-PRJ-17, R13): cloud-only files and hydration.
pub trait FsAttributes: Send + Sync {
    /// True if the file's content is not on this machine (files on demand).
    fn is_cloud_only(&self, path: &Path) -> bool;
    /// Downloads a cloud-only file. Fails when offline.
    fn hydrate(&self, path: &Path) -> std::io::Result<()>;
}

/// The real attributes: Windows cloud-file flags; elsewhere every file is local.
#[derive(Debug, Default)]
pub struct OsFsAttributes;

impl FsAttributes for OsFsAttributes {
    #[cfg(windows)]
    fn is_cloud_only(&self, path: &Path) -> bool {
        use std::os::windows::fs::MetadataExt;
        const OFFLINE: u32 = 0x1000;
        const RECALL_ON_OPEN: u32 = 0x4_0000;
        const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
        // symlink_metadata doesn't open the file, so it never triggers a download.
        std::fs::symlink_metadata(path)
            .map(|m| m.file_attributes() & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0)
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    fn is_cloud_only(&self, _path: &Path) -> bool {
        false
    }

    fn hydrate(&self, path: &Path) -> std::io::Result<()> {
        // Reading the content makes the sync client fetch it.
        let mut f = std::fs::File::open(path)?;
        std::io::copy(&mut f, &mut std::io::sink()).map(|_| ())
    }
}

/// Fake attributes for tests: a set of cloud-only paths and an offline switch.
#[derive(Debug, Default)]
pub struct FakeFsAttributes {
    pub cloud_only: Mutex<HashSet<PathBuf>>,
    pub offline: std::sync::atomic::AtomicBool,
    pub hydrate_calls: AtomicU64,
}

impl FsAttributes for FakeFsAttributes {
    fn is_cloud_only(&self, path: &Path) -> bool {
        self.cloud_only
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(path)
    }

    fn hydrate(&self, path: &Path) -> std::io::Result<()> {
        self.hydrate_calls.fetch_add(1, Ordering::Relaxed);
        if self.offline.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "offline",
            ));
        }
        self.cloud_only
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(path);
        Ok(())
    }
}

impl<T: FsAttributes + ?Sized> FsAttributes for Arc<T> {
    fn is_cloud_only(&self, path: &Path) -> bool {
        (**self).is_cloud_only(path)
    }

    fn hydrate(&self, path: &Path) -> std::io::Result<()> {
        (**self).hydrate(path)
    }
}

// ---------------------------------------------------------------- faults

/// Where a fault can be injected into an atomic write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    /// The temp file is written and flushed; the rename hasn't happened.
    AfterTemp,
    /// The rename has happened.
    AfterRename,
    /// A tile pyramid level is complete and marked (`after_level:<k>`, FR-EDT-10).
    AfterLevel(u32),
}

/// What an injected fault does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultAction {
    /// Kill the process on the spot (`PF_FAULT`, walkthroughs).
    Abort,
    /// Return an I/O error (unit tests).
    Error,
    /// Return a "file locked" error (unit tests, FR-PRJ-21).
    Locked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FaultRule {
    point: FaultPoint,
    /// Only writes to a file with this name; `None` = the first write that reaches the point.
    file: Option<String>,
    action: FaultAction,
}

/// Fault injection (AGENTS §9 `PF_FAULT`). Checks compile to nothing in release builds.
#[derive(Debug, Default, Clone)]
pub struct Faults {
    #[cfg_attr(
        not(debug_assertions),
        expect(dead_code, reason = "`hit` compiles to nothing in release builds")
    )]
    rule: Option<FaultRule>,
}

impl Faults {
    pub fn none() -> Self {
        Faults::default()
    }

    /// Parses `PF_FAULT`: `<point>[:<file name>]`, point = `after_temp` | `after_rename`;
    /// or `after_level:<k>` (stop a tile pyramid build once level `k` is complete).
    pub fn parse(spec: &str, action: FaultAction) -> Result<Self, String> {
        if let Some(k) = spec.trim().strip_prefix("after_level:") {
            let level = k
                .parse()
                .map_err(|_| format!("PF_FAULT: after_level needs a level number, got {k:?}"))?;
            return Ok(Faults {
                rule: Some(FaultRule {
                    point: FaultPoint::AfterLevel(level),
                    file: None,
                    action,
                }),
            });
        }
        let (point, file) = match spec.trim().split_once(':') {
            Some((p, f)) => (p, Some(f.to_owned())),
            None => (spec.trim(), None),
        };
        let point = match point {
            "after_temp" => FaultPoint::AfterTemp,
            "after_rename" => FaultPoint::AfterRename,
            other => return Err(format!("PF_FAULT: unknown fault point {other:?}")),
        };
        Ok(Faults {
            rule: Some(FaultRule {
                point,
                file,
                action,
            }),
        })
    }

    /// Called by the atomic writer at each point.
    #[cfg(debug_assertions)]
    pub fn hit(&self, point: FaultPoint, target: &Path) -> std::io::Result<()> {
        let Some(rule) = &self.rule else {
            return Ok(());
        };
        let name_matches = rule
            .file
            .as_deref()
            .is_none_or(|f| target.file_name().is_some_and(|n| n == f));
        if rule.point != point || !name_matches {
            return Ok(());
        }
        match rule.action {
            FaultAction::Abort => {
                tracing::warn!(?point, target = %target.display(), "PF_FAULT: aborting the process");
                eprintln!("PF_FAULT: aborting at {point:?} for {}", target.display());
                std::process::abort()
            }
            FaultAction::Error => Err(std::io::Error::other(format!(
                "injected fault at {point:?}"
            ))),
            FaultAction::Locked => Err(std::io::Error::new(
                std::io::ErrorKind::ResourceBusy,
                format!("injected lock at {point:?}"),
            )),
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline(always)]
    pub fn hit(&self, _point: FaultPoint, _target: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- events

/// Where core events go (the Tauri layer forwards them to the UI).
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &str, payload: Value);
}

/// Drops events.
#[derive(Debug, Default)]
pub struct NoEvents;

impl EventSink for NoEvents {
    fn emit(&self, _event: &str, _payload: Value) {}
}

/// Records events (tests).
#[derive(Debug, Default)]
pub struct RecordedEvents(pub Mutex<Vec<(String, Value)>>);

impl RecordedEvents {
    pub fn names(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(n, _)| n.clone())
            .collect()
    }
}

impl EventSink for RecordedEvents {
    fn emit(&self, event: &str, payload: Value) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((event.to_owned(), payload));
    }
}

impl<T: EventSink + ?Sized> EventSink for std::sync::Arc<T> {
    fn emit(&self, event: &str, payload: Value) {
        (**self).emit(event, payload)
    }
}

// ---------------------------------------------------------------- context

/// Every seam, plus the per-run paths.
pub struct Context {
    pub clock: Box<dyn Clock>,
    pub rng: Box<dyn Rng>,
    pub ids: Box<dyn IdGen>,
    pub fs: Box<dyn FsAttributes>,
    pub faults: Faults,
    /// Shared, so background work (tile pyramid builds) can emit too.
    pub events: Arc<dyn EventSink>,
    pub provider: AnyProvider,
    /// The OS credential store for provider keys (FR-PRV-06). Tests use a memory store.
    pub credentials: std::sync::Arc<dyn CredentialStore>,
    /// `<data-dir>`: local, per machine, never synced (FR-PRJ-37).
    pub data_dir: PathBuf,
    /// `PF_LIBRARY_DIR`: replaces the configured library roots for this run.
    pub library_override: Option<PathBuf>,
    /// Base URL of the `pf-tiles` scheme as the webview sees it (EDT Appendix A).
    pub tiles_base_url: String,
    /// Runs PRJ background work (thumbnails, the decoded original) on the calling thread, so
    /// tests are deterministic. False in the app.
    pub inline_background: bool,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("data_dir", &self.data_dir)
            .field("library_override", &self.library_override)
            .field("provider", &self.provider.mode())
            .finish_non_exhaustive()
    }
}

/// Default `pf-tiles` base URL for the platform's webview.
pub fn default_tiles_base_url() -> String {
    if cfg!(windows) {
        "http://pf-tiles.localhost".into()
    } else {
        "pf-tiles://localhost".into()
    }
}

impl Context {
    /// A context with real seams writing under `data_dir`. Tests replace the seams they need.
    pub fn new(data_dir: PathBuf) -> Self {
        Context {
            clock: Box::new(SystemClock),
            rng: Box::new(SeededRng::from_entropy()),
            ids: Box::new(UlidGen),
            fs: Box::new(OsFsAttributes),
            faults: Faults::none(),
            events: Arc::new(NoEvents),
            provider: AnyProvider::fake(),
            credentials: std::sync::Arc::new(KeyringStore::for_provider("scenario")),
            data_dir,
            library_override: None,
            tiles_base_url: default_tiles_base_url(),
            inline_background: false,
        }
    }

    /// Fully deterministic seams for tests: manual clock at `now`, seeded rng, sequential IDs,
    /// fake filesystem attributes, the fake provider.
    pub fn deterministic(data_dir: PathBuf, now: Timestamp) -> Self {
        Context {
            clock: Box::new(ManualClock::new(now)),
            rng: Box::new(SeededRng::new(42)),
            ids: Box::new(SequentialIds::default()),
            fs: Box::new(FakeFsAttributes::default()),
            credentials: std::sync::Arc::new(MemoryStore::default()),
            inline_background: true,
            ..Context::new(data_dir)
        }
    }

    /// The app's context from the environment (AGENTS §9):
    /// `PF_DATA_DIR`, `PF_LIBRARY_DIR`, `PF_PROVIDER`, `PF_FAKE_CASE`; in test builds also
    /// `PF_FAULT` and `PF_FAKE_NOW` (never read by release builds).
    pub fn from_env() -> Result<Self, String> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let data_dir = match var("PF_DATA_DIR") {
            Some(d) => PathBuf::from(d),
            None => default_data_dir()?,
        };
        let mut ctx = Context::new(data_dir);
        ctx.library_override = var("PF_LIBRARY_DIR").map(PathBuf::from);
        ctx.provider = AnyProvider::for_mode(
            ProviderMode::from_env_value(var("PF_PROVIDER").as_deref())?,
            ctx.credentials.clone(),
            Some(ctx.data_dir.join("cache").join("provider")),
        );
        if let (Some(case), Some(fake)) = (var("PF_FAKE_CASE"), ctx.provider.as_fake()) {
            fake.set_case(
                pf_provider::fake::FakeCase::parse(&case)
                    .ok_or_else(|| format!("PF_FAKE_CASE: unknown case {case:?}"))?,
            );
        }
        #[cfg(debug_assertions)]
        {
            if let Some(spec) = var("PF_FAULT") {
                ctx.faults = Faults::parse(&spec, FaultAction::Abort)?;
            }
            if let Some(now) = var("PF_FAKE_NOW") {
                ctx.clock = Box::new(OffsetClock::starting_at(parse_fake_now(&now)?));
            }
        }
        Ok(ctx)
    }

    /// A fresh ULID from the ID seam.
    pub fn new_id(&self) -> Ulid {
        self.ids.next_id(&*self.clock, &*self.rng)
    }

    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    pub fn emit(&self, event: &str, payload: Value) {
        self.events.emit(event, payload);
    }
}

/// `PF_FAKE_NOW`: an RFC 3339 time, or an offset from now such as `+31d` / `-2h`.
pub fn parse_fake_now(spec: &str) -> Result<Timestamp, String> {
    let spec = spec.trim();
    if let Some(sign @ ('+' | '-')) = spec.chars().next() {
        let body = &spec[1..];
        let (num, unit) = body.split_at(body.len().saturating_sub(1));
        let n: i64 = num
            .parse()
            .map_err(|_| format!("PF_FAKE_NOW: bad offset {spec:?}"))?;
        let ms = match unit {
            "d" => n * 86_400_000,
            "h" => n * 3_600_000,
            "m" => n * 60_000,
            "s" => n * 1000,
            _ => {
                return Err(format!(
                    "PF_FAKE_NOW: unit must be d, h, m or s in {spec:?}"
                ));
            }
        };
        let now = SystemClock.now().millis();
        return Ok(Timestamp(if sign == '+' { now + ms } else { now - ms }));
    }
    spec.parse().map_err(|e| format!("PF_FAKE_NOW: {e}"))
}

/// Per-user local app data (never a synced folder): `%LOCALAPPDATA%\pocket-forge` on Windows,
/// `~/Library/Application Support/pocket-forge` on macOS, `$XDG_DATA_HOME/pocket-forge` or
/// `~/.local/share/pocket-forge` elsewhere.
pub fn default_data_dir() -> Result<PathBuf, String> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        env("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share")))
    };
    base.map(|b| b.join("pocket-forge"))
        .ok_or_else(|| "can't find the local app data folder; set PF_DATA_DIR".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_ids_and_clock() {
        let ctx = Context::deterministic(PathBuf::from("x"), Timestamp(1000));
        let a = ctx.new_id();
        let b = ctx.new_id();
        assert!(a < b);
        assert_eq!(ctx.now(), Timestamp(1000));
        let ctx2 = Context::deterministic(PathBuf::from("x"), Timestamp(1000));
        assert_eq!(ctx2.new_id(), a);
    }

    #[test]
    fn ulid_gen_uses_the_clock_seam() {
        let clock = ManualClock::new(Timestamp(1_700_000_000_000));
        let rng = SeededRng::new(1);
        let id = UlidGen.next_id(&clock, &rng);
        assert_eq!(id.timestamp_ms(), 1_700_000_000_000);
        assert_ne!(id, UlidGen.next_id(&clock, &rng));
    }

    #[test]
    fn seeded_rng_is_reproducible() {
        let (a, b) = (SeededRng::new(9), SeededRng::new(9));
        assert_eq!(
            (0..5).map(|_| a.next_u64()).collect::<Vec<_>>(),
            (0..5).map(|_| b.next_u64()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn fault_spec_parsing_and_error_action() {
        let f = Faults::parse("after_temp:manifest.json", FaultAction::Error).unwrap();
        assert!(
            f.hit(FaultPoint::AfterTemp, Path::new("a/manifest.json"))
                .is_err()
        );
        assert!(
            f.hit(FaultPoint::AfterTemp, Path::new("a/other.json"))
                .is_ok()
        );
        assert!(
            f.hit(FaultPoint::AfterRename, Path::new("a/manifest.json"))
                .is_ok()
        );
        assert!(Faults::parse("mid_write", FaultAction::Error).is_err());
        let lv = Faults::parse("after_level:2", FaultAction::Error).unwrap();
        assert!(lv.hit(FaultPoint::AfterLevel(2), Path::new("x")).is_err());
        assert!(lv.hit(FaultPoint::AfterLevel(1), Path::new("x")).is_ok());
        assert!(Faults::parse("after_level:x", FaultAction::Error).is_err());
    }

    #[test]
    fn fake_now_offsets_and_absolute() {
        let t = parse_fake_now("2026-01-01T00:00:00Z").unwrap();
        assert_eq!(t.to_string(), "2026-01-01T00:00:00.000Z");
        let later = parse_fake_now("+31d").unwrap();
        let delta = later.millis() - SystemClock.now().millis();
        assert!((31 * 86_400_000 - 5000..=31 * 86_400_000).contains(&delta));
        assert!(parse_fake_now("+3w").is_err());
    }

    #[test]
    fn fr_prj_17_fake_fs_attributes_hydrate_and_offline() {
        let fs = FakeFsAttributes::default();
        let p = PathBuf::from("map.png");
        fs.cloud_only.lock().unwrap().insert(p.clone());
        assert!(fs.is_cloud_only(&p));
        fs.offline.store(true, Ordering::Relaxed);
        assert!(fs.hydrate(&p).is_err());
        fs.offline.store(false, Ordering::Relaxed);
        fs.hydrate(&p).unwrap();
        assert!(!fs.is_cloud_only(&p));
    }
}

//! The chat thread file (FR-PRM-22, §8): `.pforge/chat/<thread-id>.json`, one per asset
//! project, written temp → rename. It never holds image bytes, credentials, session ids or
//! `auth status` fields (§8). A corrupt file is kept as `<id>.corrupt-<UTC>.json` and a new
//! thread starts (§11); sync conflict copies are settled by PRJ through [`settle_copies`] and
//! [`stray_files`] (the later `updated_at` stays the thread, the other goes to the trash).
//!
//! **New chat** (#61): a fresh thread replaces the shown one. The old file stays in
//! `.pforge/chat/`, marked `retired_at`, and the new thread names it in `previous_thread_id`;
//! either mark keeps it from being shown again or swept as a conflict copy.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ulid::Ulid;

use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::fsx;
use crate::manifest::{Extra, PFORGE_DIR};
use crate::prompt::context::{ChatHost, ChatImageRole};
use crate::time::Timestamp;

/// The thread folder inside the project.
pub const CHAT_DIR: &str = "chat";
pub const THREAD_SCHEMA_VERSION: u32 = 1;
/// §11: a locked thread file is retried for this long, then the turn runs "not saved".
pub const LOCK_RETRY: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnState {
    Pending,
    Done,
    Canceled,
    Failed,
    Interrupted,
}

/// A turn's error as stored: the code, Appendix D's message and fix. Never the stderr detail
/// (FR-PRM-37).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

/// One image a turn sent: identity and size only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentImageRecord {
    pub id: String,
    pub role: ChatImageRole,
    pub w: u32,
    pub h: u32,
    pub hash: String,
}

/// What a user turn sent (§8 `context`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnContext {
    pub summary: String,
    #[serde(default)]
    pub images: Vec<SentImageRecord>,
    /// Labels of the images the turn limit dropped (FR-PRM-25).
    #[serde(default)]
    pub dropped: Vec<String>,
    pub final_prompt: String,
    pub model_id: String,
    #[serde(default)]
    pub controls: BTreeMap<String, Value>,
}

/// A proposal card (FR-PRM-28/29).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: Ulid,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inserted_at: Option<Timestamp>,
}

/// FR-PRM-38.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TurnUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    pub notional: bool,
}

/// One turn: the user's message (with its context) or a reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub id: Ulid,
    pub at: Timestamp,
    pub role: Role,
    pub host: ChatHost,
    pub text: String,
    pub state: TurnState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<TurnError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<TurnContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proposals: Vec<Proposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TurnUsage>,
    /// The user turn started a fresh backend session (FR-PRM-35).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fresh_session: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Why the core started a thread on its own (#61): after accepting a region edit or picking a
/// generated image. The panel shows a one-line divider ("New chat after accepting r12").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartReason {
    Accepted,
    Picked,
}

/// `started_after` (#61): the commit after which the core started this thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartedAfter {
    pub reason: StartReason,
    /// The revision the accept or pick committed, and its number (`r<seq>`).
    pub revision_id: Ulid,
    pub seq: u64,
}

/// The thread file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub schema_version: u32,
    pub id: Ulid,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// `fake` or `claude-code`.
    pub backend: String,
    #[serde(default)]
    pub turns: Vec<Turn>,
    /// #61: the thread this one replaced (New chat, or the automatic one after an accept or a
    /// pick). That file stays, hidden.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_thread_id: Option<Ulid>,
    /// #61: set when the core started this thread after an accept or a pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_after: Option<StartedAfter>,
    /// #61: a newer thread replaced this one: it is kept, never shown, never swept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired_at: Option<Timestamp>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Thread {
    pub fn new(id: Ulid, now: Timestamp, backend: &str) -> Self {
        Thread {
            schema_version: THREAD_SCHEMA_VERSION,
            id,
            created_at: now,
            updated_at: now,
            backend: backend.into(),
            turns: Vec::new(),
            previous_thread_id: None,
            started_after: None,
            retired_at: None,
            extra: Extra::new(),
        }
    }

    pub fn turn_mut(&mut self, id: Ulid) -> Option<&mut Turn> {
        self.turns.iter_mut().find(|t| t.id == id)
    }

    /// §11: a turn still `pending` when the file is read (the app stopped mid-turn) is
    /// `interrupted`; the window offers Resend. Turns in `live` belong to the running turn and
    /// stay `pending` (M2). Returns true if any changed.
    pub fn mark_interrupted(&mut self, live: &[Ulid]) -> bool {
        let mut changed = false;
        for t in &mut self.turns {
            if t.state == TurnState::Pending && !live.contains(&t.id) {
                t.state = TurnState::Interrupted;
                changed = true;
            }
        }
        changed
    }
}

/// `<project>/.pforge/chat/`.
pub fn chat_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(PFORGE_DIR).join(CHAT_DIR)
}

pub fn thread_path(project_dir: &Path, id: Ulid) -> PathBuf {
    chat_dir(project_dir).join(format!("{id}.json"))
}

/// The JSON files in the chat folder that can be a thread or a copy of one (no temp files, no
/// set-aside corrupt files).
fn candidate_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            n.to_lowercase().ends_with(".json")
                && !n.contains(fsx::TEMP_MARKER)
                && !n.contains(".corrupt-")
        })
        .collect();
    names.sort();
    names
}

fn canonical_id(name: &str) -> Option<Ulid> {
    name.strip_suffix(".json").and_then(|s| s.parse().ok())
}

fn read_thread(path: &Path) -> Option<Thread> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// What [`load`] found.
#[derive(Debug, Default)]
pub struct Loaded {
    pub thread: Option<Thread>,
    /// A corrupt thread file was set aside (§11): the window shows a notice.
    pub corrupt_set_aside: bool,
}

/// #61: the thread ids a newer thread replaced: marked `retired_at`, or named as some
/// thread's `previous_thread_id`.
fn retired_ids<'a>(threads: impl IntoIterator<Item = &'a Thread>) -> HashSet<Ulid> {
    let mut out = HashSet::new();
    for t in threads {
        if t.retired_at.is_some() {
            out.insert(t.id);
        }
        if let Some(p) = t.previous_thread_id {
            out.insert(p);
        }
    }
    out
}

/// The project's thread, if any: the newest canonical `<id>.json` that no newer thread
/// replaced (#61). A canonical file that won't parse is renamed `<id>.corrupt-<UTC>.json`
/// (kept, not swept) when `writable`. Pending turns come back as `interrupted` (not written
/// until the next save), except the ones in `live`: the turn the runtime is running now (M2).
pub fn load(ctx: &Context, project_dir: &Path, writable: bool, live: &[Ulid]) -> Loaded {
    let dir = chat_dir(project_dir);
    let mut out = Loaded::default();
    let mut threads = Vec::new();
    for name in candidate_names(&dir) {
        let Some(id) = canonical_id(&name) else {
            continue;
        };
        let path = dir.join(&name);
        match read_thread(&path) {
            Some(t) if t.id == id => threads.push(t),
            _ if writable && path.exists() => {
                let aside = dir.join(format!("{id}.corrupt-{}.json", ctx.now().file_stamp()));
                if std::fs::rename(&path, &aside).is_ok() {
                    out.corrupt_set_aside = true;
                }
            }
            _ => {}
        }
    }
    let retired = retired_ids(&threads);
    out.thread = threads
        .into_iter()
        .filter(|t| !retired.contains(&t.id))
        .max_by_key(|t| t.updated_at);
    if let Some(t) = out.thread.as_mut() {
        t.mark_interrupted(live);
    }
    out
}

/// One thread by id, shown or not (a reply finishing into the thread it was sent in, #61).
pub fn load_id(project_dir: &Path, id: Ulid, live: &[Ulid]) -> Option<Thread> {
    let mut t = read_thread(&thread_path(project_dir, id)).filter(|t| t.id == id)?;
    t.mark_interrupted(live);
    Some(t)
}

/// Writes the thread (temp → rename). A `locked` file is retried for up to [`LOCK_RETRY`].
pub fn save(ctx: &Context, project_dir: &Path, thread: &Thread) -> CoreResult<()> {
    let dir = chat_dir(project_dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| CoreError::io(format!("creating {}", dir.display()), &e))?;
    let path = thread_path(project_dir, thread.id);
    let start = std::time::Instant::now();
    loop {
        match fsx::write_json_atomic(ctx, &path, thread) {
            Err(e) if e.code == ErrorCode::Locked && start.elapsed() < LOCK_RETRY => {
                std::thread::sleep(Duration::from_millis(250));
            }
            other => return other,
        }
    }
}

/// Every candidate file with its parsed thread (unparseable ones as `None`).
fn parsed(dir: &Path) -> Vec<(String, Option<Thread>)> {
    candidate_names(dir)
        .into_iter()
        .map(|n| {
            let t = read_thread(&dir.join(&n));
            (n, t)
        })
        .collect()
}

/// The latest copy of thread `id` among `files`: the latest `updated_at`, a canonical name
/// winning a tie.
fn latest_of(files: &[(String, Option<Thread>)], id: Ulid) -> Option<(String, Thread)> {
    files
        .iter()
        .filter_map(|(n, t)| t.as_ref().filter(|t| t.id == id).map(|t| (n, t)))
        .max_by_key(|(n, t)| (t.updated_at, canonical_id(n) == Some(t.id)))
        .map(|(n, t)| (n.clone(), t.clone()))
}

/// PRJ (FR-PRJ-34 amendment): for each thread, if a conflict copy holds a later version than
/// `<id>.json`, it becomes `<id>.json` and the copy then holds the loser, so the trash keeps
/// it (as `job.json` copies). Returns true if anything was settled.
pub(crate) fn settle_copies(ctx: &Context, project_dir: &Path) -> bool {
    let dir = chat_dir(project_dir);
    let files = parsed(&dir);
    if files.len() < 2 {
        return false;
    }
    let ids: BTreeSet<Ulid> = files
        .iter()
        .filter_map(|(_, t)| t.as_ref().map(|t| t.id))
        .collect();
    let mut settled = false;
    for id in ids {
        let Some((name, thread)) = latest_of(&files, id) else {
            continue;
        };
        let canonical = format!("{id}.json");
        if name == canonical {
            continue;
        }
        let loser = std::fs::read(dir.join(&canonical)).ok();
        let swapped =
            fsx::write_json_atomic(ctx, &dir.join(&canonical), &thread).and_then(
                |()| match &loser {
                    Some(bytes) => fsx::write_atomic(ctx, &dir.join(&name), bytes),
                    None => std::fs::remove_file(dir.join(&name))
                        .map_err(|e| CoreError::io("removing a settled chat copy", &e)),
                },
            );
        match swapped {
            Ok(()) => settled = true,
            Err(e) => tracing::warn!(error = %e, "chat thread copy not settled; retried later"),
        }
    }
    settled
}

/// PRJ: the chat files that aren't a thread of this project (conflict copies, or a second
/// thread from another machine), as project paths, for the trash (kind `conflict_copy`). The
/// shown thread and the threads it replaced (#61) stay.
pub(crate) fn stray_files(project_dir: &Path) -> Vec<String> {
    let dir = chat_dir(project_dir);
    let files = parsed(&dir);
    if files.len() < 2 {
        return Vec::new();
    }
    let own = |n: &str, t: &Thread| canonical_id(n) == Some(t.id);
    let retired = retired_ids(
        files
            .iter()
            .filter_map(|(n, t)| t.as_ref().filter(|t| own(n, t))),
    );
    let shown = files
        .iter()
        .filter_map(|(n, t)| t.as_ref().filter(|t| own(n, t)))
        .filter(|t| !retired.contains(&t.id))
        .max_by_key(|t| t.updated_at)
        .map(|t| t.id);
    files
        .iter()
        .filter(|(n, t)| {
            let kept = match t {
                // A thread under its own name: the shown one, or one a newer thread replaced.
                Some(t) if own(n, t) => Some(t.id) == shown || retired.contains(&t.id),
                Some(_) => false,
                // An unparseable canonical file is the chat's to set aside, not a copy.
                None => canonical_id(n).is_some(),
            };
            !kept
        })
        .map(|(n, _)| format!("{PFORGE_DIR}/{CHAT_DIR}/{n}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(id: Ulid, state: TurnState) -> Turn {
        Turn {
            id,
            at: Timestamp(0),
            role: Role::User,
            host: ChatHost::Rgn,
            text: "x".into(),
            state,
            error: None,
            context: None,
            proposals: Vec::new(),
            usage: None,
            fresh_session: false,
            extra: Extra::new(),
        }
    }

    fn write(dir: &Path, name: &str, t: &Thread) {
        std::fs::create_dir_all(chat_dir(dir)).unwrap();
        std::fs::write(chat_dir(dir).join(name), serde_json::to_vec(t).unwrap()).unwrap();
    }

    /// #61 / FR-PRJ-34: threads a newer thread replaced stay (hidden, never swept); a second
    /// thread nothing links to is a conflict copy, and so is a copy of a retired thread.
    #[test]
    fn fr_prm_46_retired_threads_are_hidden_and_never_swept() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let ctx = Context::deterministic(dir.join("data"), Timestamp(1_000));
        let (a, b, c, d) = (
            Ulid::from(1u128),
            Ulid::from(2u128),
            Ulid::from(3u128),
            Ulid::from(4u128),
        );
        // a (retired by its mark) → b (retired: c names it) → c, the shown thread.
        let mut ta = Thread::new(a, Timestamp(10), "fake");
        ta.retired_at = Some(Timestamp(20));
        let tb = Thread::new(b, Timestamp(20), "fake");
        let mut tc = Thread::new(c, Timestamp(30), "fake");
        tc.previous_thread_id = Some(b);
        // A retired thread that is newer than the shown one (a late write) stays hidden.
        let mut tb_late = tb.clone();
        tb_late.updated_at = Timestamp(99);
        write(dir, &format!("{a}.json"), &ta);
        write(dir, &format!("{b}.json"), &tb_late);
        write(dir, &format!("{c}.json"), &tc);
        assert_eq!(load(&ctx, dir, true, &[]).thread.map(|t| t.id), Some(c));
        assert!(stray_files(dir).is_empty());
        assert!(!settle_copies(&ctx, dir));

        // Another machine's unrelated thread, and a copy of a retired one: both strays.
        let mut td = Thread::new(d, Timestamp(5), "fake");
        td.updated_at = Timestamp(6);
        write(dir, &format!("{d}.json"), &td);
        write(dir, &format!("{a}-OTHERPC.json"), &ta);
        let mut strays = stray_files(dir);
        strays.sort();
        assert_eq!(
            strays,
            [
                format!("{PFORGE_DIR}/{CHAT_DIR}/{a}-OTHERPC.json"),
                format!("{PFORGE_DIR}/{CHAT_DIR}/{d}.json"),
            ]
        );
        assert_eq!(load(&ctx, dir, true, &[]).thread.map(|t| t.id), Some(c));
    }

    /// §11 / M2: only pending turns the runtime doesn't own become `interrupted`.
    #[test]
    fn fr_prm_22_live_turns_stay_pending_on_reload() {
        let (old, live, done) = (Ulid::from(1u128), Ulid::from(2u128), Ulid::from(3u128));
        let mut t = Thread::new(Ulid::from(9u128), Timestamp(0), "fake");
        t.turns = vec![
            turn(old, TurnState::Pending),
            turn(live, TurnState::Pending),
            turn(done, TurnState::Done),
        ];
        assert!(t.mark_interrupted(&[live]));
        let states: Vec<TurnState> = t.turns.iter().map(|t| t.state).collect();
        assert_eq!(
            states,
            [TurnState::Interrupted, TurnState::Pending, TurnState::Done]
        );
        assert!(!t.mark_interrupted(&[live]));
    }
}

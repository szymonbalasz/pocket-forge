---
feature: asset-project
prefix: PRJ
owner: spec-asset-project
status: in-review   # unclaimed → claimed → draft → in-review → complete (owner only)
depends_on: []
---

# Asset project and library folder

## 1. Summary
The **library** is plain folders the user picks, usually cloud-synced (#5, #13). This spec covers how images get in (a copy, with no friction, #16), how an **asset project** is laid out on disk, the versioned manifest, and how a save survives crashes, sync and locked files. It also covers retention and trash (#18), and the split between the synced folder and the **local cache** (#19). Every other spec reads and writes project state through the core store defined here.

## 2. Scope
Tier S ([feature-tiers](../feature-tiers.md)): *Open an existing image as an asset project* and *Save and reopen the asset project (layers, masks, prompts, settings, revisions)*. This includes the storage side of #16, #18, #19, #20, KQ-13, KQ-16 and KQ-17.

## 3. Goals
- Get images into the library with zero dialogs: drop, paste, multi-file, folder.
- Make reopening pixel-identical (success criterion #35), and make every commit crash-safe.
- Stay friendly to sync: write-once files, few rewrites, only relative paths, and detect conflict copies.
- Bound disk growth (30-day trash) and show it.

## 4. Non-goals
- Library browsing, thumbnails, search, tags and collections (tier A, #19 index).
- Multi-user use, or several machines editing at the same time (KQ-13: one machine at a time).
- Persisted undo (KQ-16). An "empty trash now" button. Merging conflicting layer stacks pixel by pixel.
- Moving or renaming assets inside the app (tier A, library).

## 5. User stories
- As a DM preparing a session, I drop a folder of maps on the window, and they're in my library without any questions.
- I paste a map I copied from a browser, and it becomes a library file I can edit.
- I close the laptop mid-edit, open the desktop later, and everything I accepted is there. If both machines touched the project, I'm told and nothing is lost.
- I can see how much disk a project uses, and that the old candidates I rejected go away on their own.

## 6. Functional requirements
| ID | Requirement (testable) |
|---|---|
| FR-PRJ-01 | Library roots are set per machine in `<data-dir>/settings.json`. A root must exist and be writable, and must not be nested in another root or inside `<data-dir>`. Each violation returns a field-level error. |
| FR-PRJ-02 | A root that's unavailable at startup (unmounted, deleted) shows **Library unavailable**. The app writes nothing under it. |
| FR-PRJ-03 | One import command takes the paths from drag-and-drop (files and folders), a multi-select file dialog, and a pasted file list. A separate command takes pasted image data. |
| FR-PRJ-04 | A source outside every root is **copied** into the import destination (OQ-PRJ-02). The source is never written, moved or renamed: its size, mtime and hash are the same after the import. |
| FR-PRJ-05 | A source already inside a root is used in place and never copied. |
| FR-PRJ-06 | Accepted inputs are still PNG, JPEG and WebP (KQ-28), at most 16384 px per side (#11). Any other file is skipped with a reason (`unsupported`, `animated`, `too_large`, `corrupt`), and the batch continues. |
| FR-PRJ-07 | A folder import recurses and keeps the relative structure under `<dest>/<folder name>/`. |
| FR-PRJ-08 | A name collision gets ` (2)`, ` (3)`, … before the extension. If a file with the same content hash is already in the target folder, it's reused and reported as `already_in_library`. |
| FR-PRJ-09 | Every copy goes to a temp name in the target folder and is then renamed. An interrupted import never leaves a partial file under a final name. |
| FR-PRJ-10 | Pasted image data is saved as lossless `pasted-<UTC yyyymmdd-hhmmss>.png`, using the clock seam. |
| FR-PRJ-11 | An import emits progress for each file and can be cancelled. Files that finished before the cancel stay. |
| FR-PRJ-12 | The asset project of `<file>` is the folder `<file>.pforge/` next to it (OQ-PRJ-01). Project files hold only paths relative to the project folder, and never credentials. |
| FR-PRJ-13 | Opening an image writes nothing. The project folder is created by the first commit (OQ-PRJ-03). |
| FR-PRJ-14 | Opening an image that has a `.pforge` sibling opens that project. Opening a `.pforge` folder works too. |
| FR-PRJ-15 | The **original** is never written, moved or renamed. On open, its size and mtime are compared with the manifest. If they differ, its hash is checked, and a different hash opens the project **read-only** with "Original changed". |
| FR-PRJ-16 | If the original is missing, the project opens with "Original missing" and a relink action. Relink accepts a file only if its hash matches. |
| FR-PRJ-17 | A cloud-only original is hydrated by reading it on open, with the state "Downloading from cloud". Offline, open fails with `not_available_offline` and nothing is written. No other file is ever read just to hydrate it (#19, KQ-14). |
| FR-PRJ-18 | Every mutating core command commits before it returns success. The order is: new write-once files (temp → flush → rename), then the manifest (temp → flush → rename). The manifest rename is the commit point. |
| FR-PRJ-19 | The unsubmitted edit (final prompt, model id, control values, region, mask) is saved through `project_put_draft`. The UI calls it ≥ 2 s after the last change, and on close. |
| FR-PRJ-20 | After a kill at any step of a commit, reopening shows exactly the state before or after that commit. On the next open, temp files are deleted, and files the manifest doesn't reference go to the trash. |
| FR-PRJ-21 | A locked file or a sharing violation is retried with backoff for up to 3 s. After that, `project:save_failed{locked}` fires, the state stays in memory and is retried every 10 s, and closing with unsaved state asks the user first. |
| FR-PRJ-22 | Save → close → reopen is pixel-identical for the original, layers and masks, and field-identical for the prompts, settings, jobs, candidates and revisions. |
| FR-PRJ-23 | A **revision** is a write-once snapshot of the full layer stack plus the unsubmitted edit. One is created on project creation (`r0`, the original only), on accept (RGN), on explicit Save (Ctrl+S, only if something changed since the head), on restore, and on conflict resolution. Restoring adds a new revision, and none is ever deleted. |
| FR-PRJ-24 | If `manifest.json` is missing or unreadable, open rebuilds it from the newest revision and the job files, and reports "Recovered". |
| FR-PRJ-25 | The manifest has an integer `schema_version`. Loading runs the migrations from each older version in order. The migrated manifest is written at the next commit, and the old file is kept as `manifest.v<N>.json`. |
| FR-PRJ-26 | A `schema_version` newer than the app supports opens **read-only** ("Made by a newer Pocket Forge"). Nothing is written. |
| FR-PRJ-27 | Unknown fields are kept on write. The JSON is UTF-8 with LF line endings, pretty-printed, with a stable field order. |
| FR-PRJ-28 | A job record (KQ-17, ADR 0001) is written before submit (`requested`), again at `submitted` (with the provider job handle), and at the terminal state. It holds the fields in §8. |
| FR-PRJ-29 | Reopening a project whose job record isn't terminal reports that job as resumable (it's consumed by RGN). |
| FR-PRJ-30 | Candidates are stored untouched when they arrive. Accepted or starred candidates are never trashed or purged (#18). |
| FR-PRJ-31 | When a job is resolved (a candidate accepted, or the job dismissed), its unstarred, unaccepted candidates move to `trash/` with `trashed_at` set from the clock seam. Starring or restoring a candidate brings it back. |
| FR-PRJ-32 | On open, trash items whose `trashed_at` is more than 30 days old are deleted. Nothing else is ever deleted by the app. |
| FR-PRJ-33 | Per-project disk use is shown by category (original, layers, masks, candidates, trash, other). It comes from file metadata only and never hydrates a file. |
| FR-PRJ-34 | Open and every commit check for **conflict copies**: a known project file name with a sync suffix (`-<any>` or ` (<n>)`) before the extension. |
| FR-PRJ-35 | Before every manifest write, the `commit.seq` and `commit.install_id` on disk are compared with the last ones loaded or written. On a mismatch, the write is refused and `project:conflict` fires. |
| FR-PRJ-36 | Resolving a conflict merges the write-once records (jobs, candidates, masks, layers, revisions) from both sides by id. The user picks which current state to keep, the other one becomes a revision, and the conflict copy goes to the trash. |
| FR-PRJ-37 | Rebuildable data lives in `<data-dir>` (`PF_DATA_DIR`, by default the per-user local app data folder, never synced). Deleting it loses no project data. |
| FR-PRJ-38 | The tile cache is keyed by the original's content hash, and least-recently-used entries are evicted above a cap (OQ-PRJ-09). |

## 7. UI/UX notes
- **First run / no library:** a "Choose your library folder" picker (`prj-setup-pick-root`). **Library unavailable:** a banner with "Choose again".
- **Home (empty):** a full-window drop zone, "Drop images or a folder, or paste" (`prj-home-dropzone`), plus "Open image…" and a recent-projects list (`prj-home-recent`, `data-record-id`).
- **Import:** a toast with "12 / 40" and Cancel (`prj-import-cancel`). The summary shows imported, already in library, and skipped with reasons. There's no confirmation dialog.
- **Status bar** (`prj-status-save`): Saved · time / Saving… / Save failed, file locked, retrying / Read-only (reason) / Conflict.
- **Project panel** (`prj-panel-*`): disk use by category, trash count, size and next purge date, a revision list with Restore, and trashed candidates with Restore.
- **Loading:** "Downloading from cloud…" for a cloud-only original, then the pyramid backdrop (EDT). **Offline:** `not_available_offline` with Retry.
- **Job running:** closing is allowed. The job record is already on disk, and the job resumes on reopen (FR-PRJ-29). **Provider unavailable:** N/A here.
- **Conflict banner:** "This project was also changed on another machine." Two choices: Keep this machine's version or Keep the other version. Both keep the loser as a revision.

## 8. Data and file effects
```
<library-root>/                      synced (#5, #13)
  Imported/                          default import destination (OQ-PRJ-02)
  dungeons/crypt.png                 original: never written by the app
  dungeons/crypt.png.pforge/         asset project
    manifest.json                    mutable: current state + indexes
    revisions/<rev-id>.json          write-once
    layers/<layer-id>.png            write-once, baked patch RGBA8 (ADR 0004)
    masks/<mask-id>.png              write-once, 8-bit grey, white = editable (ADR 0001)
    jobs/<job-id>.json               rewritten at ≤ 3 state changes (FR-PRJ-28)
    candidates/<cand-id>.png         write-once, untouched candidate
    trash/<item-id>/…                trashed files, original sub-path kept
<data-dir>/                          local, rebuildable, never synced (#19)
  settings.json                      library roots, import destination, install id
  recent.json · cache/tiles/<content-hash>/…   (tile format owned by EDT)
```
| Entity | Fields (IDs are ULIDs from the ID seam; times are UTC RFC 3339 from the clock seam) |
|---|---|
| manifest | `schema_version`, `project_id`, `created_at`, `commit{seq, install_id, at}`, `original`, `layers[]`, `masks[]`, `draft`, `jobs[]{id, state}`, `candidates[]`, `revisions[]{id, seq, at, reason}`, `head_revision`, `trash[]` |
| original | `file` (relative, e.g. `../crypt.png`), `content_hash`, `size`, `mtime`, `width`, `height`, `format` |
| layer | `id`, `kind` (`original`\|`patch`), `file`, `cx`, `cy`, `scale`, `rotation`, `opacity`, `visible` (ADR 0003). Patch adds `job_id`, `candidate_id`, `mask_id`, `region{x,y,w,h,context_px}`, `feather_px`, `colour_match{on, gain[3], offset[3]}` |
| mask | `id`, `file`, `rect{x,y,w,h}` in image px |
| draft | `final_prompt`, `model_id`, `controls{}`, `region`, `mask_id` |
| job | `id`, `created_at`, `state`, `state_times{}`, `operation`, `model_id`, `final_prompt`, `controls{}`, `seed{requested, returned}`, `capability_snapshot`, `region`, `mask_id`, `candidate_count`, `estimate`, `cost`, `error`, `provider{name, job_handle, asset_refs{}}` (a disposable cache, #20), `candidate_ids[]` |
| candidate | `id`, `job_id`, `file`, `width`, `height`, `seed`, `starred`, `status` (`new`\|`accepted`\|`dismissed`\|`trashed`) |
| revision | `id`, `seq`, `created_at`, `reason` (`create`\|`accept`\|`save`\|`restore`\|`conflict`), `parent_id`, `layers[]` (full), `draft` |
| trash item | `id`, `kind`, `files[]`, `from`, `trashed_at` |

**Recoverable:** a lost manifest is rebuilt from the revisions and jobs (FR-PRJ-24). Trashed items can be restored for 30 days. The original and accepted candidates are never deleted. A lost `<data-dir>` costs a re-pick of roots plus cache rebuilds only.

## 9. Commands and events
| Command | Input → result | Validation / errors |
|---|---|---|
| `library_get_config` / `library_set_roots` | — / `roots[]`, `import_dest` → config | FR-PRJ-01 field errors |
| `import_paths` | `paths[]` → per item `imported(path)` \| `already_in_library(path)` \| `skipped(reason)` | `library_unavailable`, `cancelled` (partial results) |
| `import_clipboard_image` | image bytes → library path | `unsupported`, `too_large` |
| `project_open` | image or `.pforge` path → snapshot + `warnings[]` (read-only reason, conflict, recovered, resumable jobs) | `not_found`, `not_available_offline`, `corrupt`, `outside_library` (the UI then calls `import_paths` first) |
| `project_close` | — → ok | `unsaved` (when a save failed) |
| `project_put_draft` | draft → `seq` | `read_only`, `locked`, `conflict` |
| `project_save_revision` / `project_restore_revision` | — / `rev_id` → revision | `unchanged`, `unknown_id`, `read_only` |
| `project_set_star` / `project_dismiss_job` / `project_trash_restore` | id → ok | `unknown_id`, `read_only` |
| `project_disk_usage` | — → bytes per category + next purge date | — |
| `project_relink_original` | path → ok | `hash_mismatch` |
| `project_resolve_conflict` | `keep: this \| other` → revision | `no_conflict` |
| `recent_list` | — → `[{path, name, opened_at}]` | — |

- **Store API** (Rust, used inside other specs' commands, each call is one commit): `record_job`, `add_candidates`, `put_mask`, `accept_candidate` (patch layer + candidate status + revision, in one commit), `read_project` (for EXP and EDT).
- **Events:** `import:progress{done, total}`, `import:done`, `project:committed{seq}`, `project:save_failed{reason, retrying}`, `project:conflict`, `project:recovered`, `trash:purged{count, bytes}`.

## 10. Provider interaction
N/A: there are no provider calls. PRJ persists the job record that ADR 0001 requires (KQ-17), and keeps provider asset ids only as a cache (#20).

## 11. Edge cases
| Case | Behaviour |
|---|---|
| Crash mid-save | Commit order and the commit point per FR-PRJ-18 and FR-PRJ-20. A crash mid-import leaves only temp files, which are cleaned up (FR-PRJ-09). |
| Sync conflict / locked file | FR-PRJ-21 and FR-PRJ-34 to 36. Both the conflict-copy check and the seq check are needed, because sync can land either way. |
| Offline | Only cloud-only files are affected (FR-PRJ-17). Nothing in PRJ needs the network. |
| Provider error or timeout | N/A. The job record keeps the error (FR-PRJ-28). |
| Very large image | Rejected above 16384 px per side (FR-PRJ-06). Hashing streams, and is skipped on open while size and mtime match (FR-PRJ-15). |
| Corrupt or unsupported input | Skipped at import from the header check (FR-PRJ-06). A full-decode failure on open returns `corrupt` and writes nothing. |
| Undo across the feature boundary | Undo is in-session (KQ-16, EDT). Undoing an accept: OQ-PRJ-06. |

## 12. Performance budget
| Operation | Typical map (#30) | 16k |
|---|---|---|
| `project_open`, PRJ share (manifest + layer metadata, no hash) | < 150 ms | < 300 ms |
| A commit (manifest ≤ 200 KB) on a local SSD | < 50 ms | same |
| Patch layer PNG write (≤ 4096², fast compression) | < 300 ms | same |
| Import copy + hash | ≈ disk speed, < 1 s | < 5 s |
| PRJ memory beyond ADR 0004's decoded original | < 20 MB | same |

## 13. Test plan
- **Unit (Rust, `pf-core`):** manifest round-trip and stable field order, and each migration step from a synthetic v0 fixture (FR-PRJ-25 to 27). Conflict-copy name matcher. Collision suffixes. Purge date math with a fake clock. Import validation. All of them are named `fr_prj_NN_*`.
- **Integration (Rust):** a fault-injecting filesystem seam that fails at each step of a commit, then reopen equals before or after (FR-PRJ-20). Lock simulation, k failures then success (FR-PRJ-21). The seq mismatch refusal (FR-PRJ-35). Import of a generated folder tree, with the source re-hashed (FR-PRJ-04). Save → reopen pixel and field identity with a patch made from the **fake provider's** candidate (FR-PRJ-22). Rebuild from a deleted manifest (FR-PRJ-24). A real-process kill test: a child process commits in a loop, gets killed, and the project is reopened.
- **Thin e2e (Playwright, mocked core):** drop → import summary. Status bar states. The conflict banner. The read-only banner. Revision restore.
- **Fixtures (generated by committed scripts, KQ-22):** a procedural grid map at 2100×2800. Small PNG, JPEG and WebP files. An animated WebP. A truncated PNG. A PSD-magic file. A 16385 px header-only PNG. Manifest v0/v1. Conflict-copy names. No personal art.
- **App walkthrough** (`PF_DATA_DIR` and `PF_LIBRARY_DIR` in `.pf-dev/`, fake provider):

| # | Step | Expected |
|---|---|---|
| 1 | First launch, pick `.pf-dev/library` | Empty home with the drop zone |
| 2 | Drop a PNG, a JPEG and a `.psd` from outside the library | 2 imported into `Imported/`, 1 skipped (`unsupported`). The source files are unchanged |
| 3 | Paste a copied image; drop a folder with subfolders | `pasted-….png` created. The structure is kept |
| 4 | Open the PNG | It displays. No `.pforge` folder exists yet |
| 5 | Paint a mask, submit, accept one candidate, star another | `.pforge` holds the manifest, job, mask, layer, `r0` and `r1`. One candidate is in `trash/`. Disk use updates |
| 6 | Kill the app during a commit (`PF_FAULT=after_temp`), relaunch | The last committed state, and no stray files |
| 7 | Copy `manifest.json` to `manifest-OTHERPC.json`, reopen | Conflict banner. Keep this → a merged revision, and the copy goes to the trash |
| 8 | Restart with the fake clock 31 days later | The trashed candidate is gone. The starred and accepted ones remain |
| 9 | Restore `r0`, close, reopen | Original only. History `r0`–`r3` intact, and the reopen is pixel-identical |

## 14. Build split
| # | Slice | Shared files (AGENTS §9) | Depends on |
|---|---|---|---|
| 1 | Store core: atomic writer (temp, rename, retry, fault seam), manifest v1 types, migration chain, version guard | Manifest schema (new) | skeleton |
| 2 | Library config and import (paths, folder, clipboard, validation, dedupe, hash) + home drop zone UI | command registry (append), UI feature registry (append), root `Cargo.toml` for OQ-PRJ-05 (orchestrator) | 1 |
| 3 | Open, create and close: original checks, relink, cloud-only handling, recent list, `<data-dir>` layout | command registry (append) | 1 |
| 4 | Store API for jobs, candidates, masks, patch layers and revisions; draft; save and restore revision | manifest schema (add fields), command registry (append) | 3. RGN, EDT and EXP build on it |
| 5 | Retention: star, dismiss → trash, purge, restore, disk usage + project panel | command registry (append) | 4 |
| 6 | Conflicts: copy detection, seq check, merge and resolve, banners and status bar | command registry (append) | 4 |

## 15. Open questions
| ID | Question | Options (pick in **bold**) | Blocking? | Affects |
|---|---|---|---|---|
| OQ-PRJ-01 | Where does an asset project live? | **A. A sidecar `<file>.pforge/` next to the original (KQ-10 A). The original stays a plain file other tools can see.** / B. A self-contained folder with the original copied inside. A loses the link if the original is renamed outside the app (relink, FR-PRJ-16); B survives renames but doubles disk for in-library images. | Yes | all specs |
| OQ-PRJ-02 | Import destination | **A. One setting, default `<library-root>/Imported/`, shown in the import toast** / B. Ask every time. A has no friction (#16); B gives precise placement. | No | — |
| OQ-PRJ-03 | When is the project folder created? | **A. At the first commit; viewing writes nothing** / B. On open. A causes no sync noise for browsing; B makes `r0` exist immediately. | No | EDT |
| OQ-PRJ-04 | Save model | **A. Every mutation commits; drafts are debounced ≥ 2 s; revisions only per FR-PRJ-23** / B. Explicit save only. A loses nothing on a crash for a few KB per commit; B is quieter on sync but loses work. | Yes | EDT, RGN |
| OQ-PRJ-05 | A new dependency for content hashes (§3.5) | **A. `sha2`** / B. `blake3` (faster, bigger) / C. none: size + mtime only. C loses dedupe and the safe relink. | Yes, for slice 2 | owner |
| OQ-PRJ-06 | Undoing an accept ([editor-canvas](editor-canvas.md), [region-edit](region-edit.md)) | **A. It hides the patch and records a `restore` revision; the candidate stays** / B. The undo stack stops at accept. A is natural; B is simpler. | No | EDT, RGN |
| OQ-PRJ-07 | Who owns the revision list and the project panel? (seams push-back) | **A. PRJ, minimal (list + Restore)** / B. [editor-canvas](editor-canvas.md). The seams table doesn't assign it. | No | EDT |
| OQ-PRJ-08 | Glossary: add **Trash** (a project's 30-day holding area, #18), **Conflict copy** (a duplicate file made by a sync client) and **Local cache** (`<data-dir>`, rebuildable, #19)? | **A. Add all three** / B. Implementation words only. Specs already need them. | No | glossary |
| OQ-PRJ-09 | Tile cache size cap | **A. LRU at 4 GiB by default, configurable** / B. Unbounded, with manual clear. A bounds the disk; B never rebuilds. | No | EDT |

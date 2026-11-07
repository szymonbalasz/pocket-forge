---
feature: asset-project
prefix: PRJ
owner: rev-asset-project
status: in-review   # unclaimed → claimed → draft → in-review → complete (owner only)
depends_on: []
---

# Asset project and library folder

## 1. Summary
The **library** is plain folders the user picks, usually cloud-synced (#5, #13). This spec covers how images get in (a copy, no friction, #16), how an **asset project** is laid out on disk, how a commit survives crashes, sync and locked files, retention and **trash** (#18), the **local cache** split (#19), app settings, and the decoded original in memory. Every other spec reads and writes project state through PRJ's store API. All field shapes live in [manifest-schema](../architecture/manifest-schema.md) (R26).

## 2. Scope
Tier S ([feature-tiers](../feature-tiers.md)): *Open an existing image as an asset project* and *Save and reopen the asset project*, plus the storage side of #16, #18, #19, #20, KQ-13, KQ-16, KQ-17. Includes a minimal revision list, a recent-projects list and relink (#40).

## 3. Goals
Zero-dialog import (drop, paste, multi-file, folder) · pixel-identical reopen (#35) and crash-safe commits · sync-friendly files (write-once, relative paths, conflict detection) · bounded, visible disk growth.

## 4. Non-goals
Library browsing, thumbnails, search, tags (tier A) · several machines editing at once (KQ-13) · record-level conflict merge (#39) · persisted undo (KQ-16) · "empty trash now" · moving or renaming assets in the app.

## 5. User stories
- I drop a folder of maps or paste one from a browser, and it's in my library without any questions. I close the laptop mid-edit and open the desktop later: everything I accepted is there. If both machines touched the project, I see both versions, keep one, and the other stays in the history.
- I can see how much disk a project uses, and old rejected candidates go away on their own.

## 6. Functional requirements
| ID | Requirement (testable) |
|---|---|
| FR-PRJ-01 | Library roots are per machine, in `settings.json` `library`. A root must exist, be writable, and not be nested in another root or `<data-dir>`. Each violation returns a field-level error. |
| FR-PRJ-02 | A root unavailable at startup shows **Library unavailable**. Nothing is written under it. |
| FR-PRJ-03 | `import_paths` takes paths from drag-and-drop (files and folders), a multi-select dialog and a pasted file list. `import_clipboard_image` takes pasted image data. |
| FR-PRJ-04 | A source outside every root is **copied** to the import destination (default `<library-root>/Imported/`, #40). The source's size, mtime and hash are unchanged after import. |
| FR-PRJ-05 | A source already inside a root is used in place, never copied. |
| FR-PRJ-06 | Accepted: PNG, JPEG, WebP (KQ-28), ≤ 16384 px per side (#11). Anything else is skipped with `unsupported`, `animated`, `too_large` or `corrupt`, and the batch continues. |
| FR-PRJ-07 | A folder import recurses and keeps the relative structure under `<dest>/<folder name>/`. |
| FR-PRJ-08 | A name collision gets ` (2)`, ` (3)`, … before the extension. Same content hash already in the target folder → reused, reported `already_in_library`. |
| FR-PRJ-09 | Every copy goes to a temp name in the target folder, then is renamed. An interrupted import never leaves a partial file under a final name. |
| FR-PRJ-10 | Pasted data is saved as lossless `pasted-<UTC yyyymmdd-hhmmss>.png` (clock seam). |
| FR-PRJ-11 | Import emits `import:progress` per file. `import_cancel` stops it; files already finished stay. |
| FR-PRJ-12 | The asset project of `<file>` is `<file>.pforge/` next to it (#40). Project files hold only project-relative paths and never credentials. |
| FR-PRJ-13 | Opening an image writes nothing. The first commit creates the folder and revision `r0`. |
| FR-PRJ-14 | Opening an image with a `.pforge` sibling opens that project; opening the `.pforge` folder works too. |
| FR-PRJ-15 | The **original** is never written, moved or renamed. On open, size and mtime are compared with the manifest; if they differ, the hash is checked, and a different hash opens **read-only** ("Original changed"). |
| FR-PRJ-16 | A missing original opens with "Original missing" and **Relink**, which accepts a file only if its hash matches. |
| FR-PRJ-17 | A cloud-only original (fs-attributes seam: `is_cloud_only`, `hydrate`, R13) is hydrated on open ("Downloading from cloud"). Offline → `not_available_offline`, nothing written. No other file is ever hydrated (#19, KQ-14). |
| FR-PRJ-18 | Every mutating command commits before it returns success: new write-once files, then the manifest, each temp → flush → rename. The manifest rename is the commit point ([schema §3](../architecture/manifest-schema.md#3-write-rules)). |
| FR-PRJ-19 | The draft is saved by `project_put_draft` ≥ 2 s after the last draft or export-settings change and on close (#40). The same commit writes EXP's unsaved export settings into manifest `export` (R30). `put_mask` (from `edt_mask_submit`, FR-EDT-28) writes the mask and points the draft at it in one commit. Masks no draft, job or revision references go to the trash at the next commit (R7). |
| FR-PRJ-20 | After a kill at any step of a commit, reopen shows exactly the state before or after it. On open, temp files are deleted, roll-forward steps run, and files referenced by no manifest field, revision or job file go to the trash. |
| FR-PRJ-21 | A locked file is retried with backoff for ≤ 3 s, then `project:save-failed{reason: locked}` fires, state stays in memory and is retried every 10 s, and closing with unsaved state asks first. |
| FR-PRJ-22 | Save → close → reopen is pixel-identical for the original, layers and masks, and field-identical for prompts, settings, jobs, candidates and revisions. |
| FR-PRJ-23 | A **revision** is a write-once snapshot of the layer stack plus the draft. Reasons: `create` (`r0`), `accept`, `save` (Ctrl+S, only if changed since the head), `restore`, `retune`, `conflict` (R24). Restore writes a new revision with the restored layers and draft. Optional `label`; `r<seq>` is a display label. None is ever deleted. |
| FR-PRJ-24 | If `manifest.json` is missing or unreadable, open rebuilds it from the newest revision and the revision and job files, and reports `project:recovered`. |
| FR-PRJ-25 | `schema_version` migrations run in order on load; the migrated file is written at its next commit, keeping `manifest.v<N>.json` ([schema §6](../architecture/manifest-schema.md#6-versioning-and-migration)). |
| FR-PRJ-26 | A newer `schema_version` than supported opens **read-only** ("Made by a newer Pocket Forge"). Nothing is written. |
| FR-PRJ-27 | Unknown fields are kept on write. JSON is UTF-8, LF, pretty-printed, stable field order. |
| FR-PRJ-28 | A job record is `jobs/<id>/job.json` + `region.png` + `mask.png` (as sent: after fit, before polarity conversion). State is persisted on **every** transition, with PRV's opaque `provider{}` (which holds the handle) verbatim. The job file is authoritative; manifest `jobs[]` is an index rebuilt on open (R2). |
| FR-PRJ-29 | `project_open` reports every non-terminal job as resumable (RGN consumes it). A `requested` job (no handle) reopened after a crash becomes `lost`. |
| FR-PRJ-30 | Candidates are stored untouched as `candidates/<id>.<sniffed ext>` (R10). Accepted or starred candidates, and files any revision references, are never trashed or purged (#18). |
| FR-PRJ-31 | Accept = one commit: patch layer, candidate statuses, revision, and the job's unstarred, unaccepted candidates to the trash (R4). A job that ends with nothing accepted or starred sends its candidates **and inputs** to the trash; accepted jobs keep inputs forever (R9, KQ-17). Starring or restoring brings an item back. |
| FR-PRJ-32 | On open, trash items older than 30 days (clock seam) are deleted. Nothing else is ever deleted by the app. |
| FR-PRJ-33 | Disk use per project by category (original, layers, masks, job inputs, candidates, trash, other) comes from file metadata only and never hydrates a file. |
| FR-PRJ-34 | Open and every commit look for **conflict copies** of `manifest.json` and `job.json`: the name with a sync suffix (`-<any>` or ` (<n>)`) before the extension. |
| FR-PRJ-35 | Before every manifest write, on-disk `commit{seq, install_id}` is compared with the last loaded or written values. A mismatch refuses the write and fires `project:conflict`. |
| FR-PRJ-36 | On a conflict, `project_conflict_get` returns both versions (time, this/other machine, layer stack, newest revision label, missing files). The user keeps one; one commit makes it current, writes the other's layers + draft as a `conflict` revision, and trashes the conflict copy. Nothing is merged; files from both sides stay and indexes are rebuilt (#39). A version with missing files can't be kept yet. |
| FR-PRJ-37 | `<data-dir>` (`PF_DATA_DIR`, default per-user local app data, never synced) holds caches, which rebuild, and per-machine config. Deleting it loses no committed project data (R8). |
| FR-PRJ-38 | The tile cache lives under `<data-dir>/cache/tiles/`. Key, cap (2 GiB) and eviction are EDT's ([editor-canvas](editor-canvas.md), R5). |
| FR-PRJ-39 | `settings.json` has sections `library`, `spend`, `provider` and is written temp → rename. A corrupt file → defaults plus a warning, the bad file kept aside (R8). |
| FR-PRJ-40 | While a write can't commit (conflict, `locked` after 3 s, read-only), candidate bytes and job updates go to the spool `<data-dir>/pending/<project-id>/` and are replayed in order once writable. A crash in between loses no paid output (R12). |
| FR-PRJ-41 | In a **read-only** project the user can view and paint. Every writing command and store call fails fast with `read_only`; the UI disables submit (R12). |
| FR-PRJ-42 | PRJ owns a `DecodedOriginal` (RGB8, ADR 0004): decoded lazily on the first RGN or EXP use, prefetched by `project_prefetch_original`, released on close. Until ready, RGN and EXP get `original_unavailable` (R11). |
| FR-PRJ-43 | Undo of an accept or re-tune = `project_restore_revision(parent)` (reason `restore`); redo = restore the later revision. Candidate statuses and trash are unchanged (R3). |

## 7. UI/UX notes
- **First run:** "Choose your library folder" (`prj-setup-pick-root`); **library unavailable:** banner with "Choose again". **Home:** full-window drop zone (`prj-home-dropzone`), "Open image…", recent projects (`prj-home-recent`, `data-record-id`).
- **Import:** toast "12 / 40" with Cancel (`prj-import-cancel`); the summary lists imported, already in library, skipped + reason. No confirmation dialog.
- **Status bar** (`prj-status-save`): Saved · time / Saving… / Save failed, file locked, retrying / Read-only (reason) / Conflict.
- **Project panel** (`prj-panel-*`): disk use by category, trash count, size and next purge date, revisions (`r<seq>`, label, Restore), trashed items with Restore.
- **Loading:** "Downloading from cloud…", then the backdrop (EDT). **Offline:** `not_available_offline` with Retry. **Job running:** closing is allowed; the job resumes on reopen (FR-PRJ-29).
- **Conflict:** banner "This project was also changed on another machine" → a two-pane view (`prj-conflict-this`, `prj-conflict-other`) with time and layer stack per side (OQ-PRJ-11), and **Keep this** / **Keep other** (`prj-conflict-keep-this`, `-keep-other`). A side with missing files shows "Still syncing" and its Keep is disabled.

## 8. Data and file effects
Full tree and field tables: [manifest-schema](../architecture/manifest-schema.md). In short:
- **Synced:** `<file>.pforge/` holds `manifest.json` (mutable, commit point), write-once `revisions/`, `layers/`, `masks/`, `candidates/`, mutable `jobs/<id>/job.json` + write-once inputs, and `trash/`.
- **Local (`<data-dir>`):** `settings.json`, `recent.json`, `cache/tiles/` (EDT), `cache/provider/` (PRV), `pending/` (spool).
- **Growth:** each re-tune writes a new layer PNG kept by its revision; acceptable (#18 keeps accepted history), shown in disk use.
- **Recoverable:** a lost manifest is rebuilt (FR-PRJ-24). Trash restores for 30 days. The original and accepted candidates are never deleted. A lost `<data-dir>` costs re-picking roots, the spend cap, cache rebuilds, and any unreplayed spool.

## 9. Commands and events
| Command | Input → result | Errors |
|---|---|---|
| `library_get_config` / `library_set_roots` | — / `roots[]`, `import_dest` → config | FR-PRJ-01 field errors |
| `import_paths` / `import_cancel` | `paths[]` → per item `imported` \| `already_in_library` \| `skipped(reason)` | `library_unavailable`, `canceled` (partial results) |
| `import_clipboard_image` | image bytes → library path | `unsupported`, `too_large` |
| `project_open` | image or `.pforge` path → snapshot + `warnings[]` (read-only reason, conflict, recovered, resumable, lost) | `not_found`, `not_available_offline`, `corrupt`, `outside_library` (UI calls `import_paths` first) |
| `project_close` | — → ok | `unsaved` |
| `project_put_draft` | draft (+ EXP's unsaved export settings, R30) → `seq` | `read_only`, `locked`, `conflict` |
| `project_list_revisions` / `project_save_revision` / `project_restore_revision` | — / label? / `rev_id` → `[{id, seq, created_at, reason, label}]` / revision | `unchanged`, `unknown_id`, `read_only` |
| `project_trash_list` / `project_trash_restore` | — / item id → items / ok | `unknown_id`, `read_only` |
| `project_disk_usage` / `project_relink_original` | — / path → bytes per category + next purge date / ok | — / `hash_mismatch` |
| `project_prefetch_original` | — → ok (decode starts) | `corrupt`, `not_available_offline` |
| `project_conflict_get` / `project_resolve_conflict` | — / `keep: this \| other` → both versions / revision | `no_conflict`, `files_missing` |
| `recent_list` | — → `[{path, name, opened_at}]` | — |

- **Store API** (Rust, one commit each, `read_only`/`locked`/`conflict` as above, R4): `read_project` (current layer stack, R6), `record_job`, `update_job`, `add_candidates`, `put_mask`, `accept_candidate`, `set_star`, `discard_candidate`, `resolve_job`, `replace_patch_layer` (re-tune), `put_export` (export settings + `last`), `settings_get` / `settings_put(section)`, `decoded_original`, `resolve_image(image_id)` (original, layer or candidate → file, for EDT). User-facing job and candidate commands are RGN's (`rgn_*`).
- **Events:** `import:progress{done, total}`, `import:done`, `project:committed{seq}`, `project:save-failed{reason, retrying}`, `project:conflict`, `project:recovered`, `project:original-ready`, `trash:purged{count, bytes}` (R21).

## 10. Provider interaction
N/A: no provider calls. PRJ persists the job record ADR 0001 requires (KQ-17) and keeps the opaque `provider{}` as a cache (#20).

## 11. Edge cases
| Case | Behaviour |
|---|---|
| Crash mid-save | FR-PRJ-18/20; job follow-ups roll forward on open. Mid-import: only temp files, cleaned up (FR-PRJ-09). |
| Sync conflict / locked file | FR-PRJ-21, 34–36, 40. Both the copy check and the seq check are needed; sync can land either way. |
| Offline / provider error | Only cloud-only files are affected (FR-PRJ-17). PRJ makes no provider calls; the job record keeps any error. |
| Very large image | > 16384 px rejected (FR-PRJ-06). Hashing streams and is skipped while size and mtime match. 16k decode is background (FR-PRJ-42). |
| Corrupt input or settings | Skipped at import by header check. A decode failure on open → `corrupt`, nothing written. Corrupt `settings.json` → FR-PRJ-39. |
| Undo across features | In-session only (KQ-16); accept/re-tune per FR-PRJ-43. |

## 12. Performance budget
| Operation | Typical map (#30) | 16k |
|---|---|---|
| `project_open`, PRJ share (manifest + layer metadata, no hash) | < 150 ms | < 300 ms |
| A commit (manifest ≤ 200 KB, local SSD) | < 50 ms | same |
| Patch layer PNG write (≤ 4096², fast compression) | < 300 ms | same |
| Import copy + hash | ≈ disk speed, < 1 s | < 5 s |
| `DecodedOriginal` decode (background) / memory; PRJ other memory < 20 MiB | < 0.5 s / ≤ 70 MiB | < 4 s / 768 MiB |
| Whole app, region job running (EDT UI 704 + core transient 256 MiB, decoded original, RGN 400 MiB, PRJ 20 MiB) | < 1.5 GiB | ≤ 2.1 GiB |
| Whole app, export running (EDT UI 704 + core transient 256 MiB, decoded original, EXP stream 400 MiB / rescale 1.6 GiB, PRJ 20 MiB) | < 1.5 GiB | ≤ 2.1 GiB / ≤ 3.3 GiB |

## 13. Test plan
- **Unit (`pf-core`, `fr_prj_NN_*`):** manifest round-trip + field order; each migration from a v0 fixture; conflict-copy matcher; collision suffixes; purge date math (fake clock); import validation; settings corrupt → defaults.
- **Integration (Rust):** fault-injecting fs seam at each commit step → reopen = before or after, roll-forward of job follow-ups (FR-PRJ-20). Lock: k failures then success (FR-PRJ-21). Seq-mismatch refusal; spool replay after resolve (FR-PRJ-35/40). Conflict resolve keeps both sides' job and revision files (FR-PRJ-36). Fake fs-attributes: cloud-only + offline, and disk use never calls `hydrate` (FR-PRJ-17/33). Import of a generated tree with the source re-hashed. Save → reopen identity with a patch from the **fake provider** (FR-PRJ-22). Rebuild from a deleted manifest. R9 retention. Undo via restore (FR-PRJ-43). Read-only refusals. A real-process kill test.
- **Thin e2e (Playwright, mocked core):** drop → summary; status bar states; conflict view; read-only banner; revision restore.
- **Fixtures (generated, KQ-22, no personal art):** procedural grid map 2100×2800; small PNG/JPEG/WebP; animated WebP; truncated PNG; PSD-magic file; 16385 px header-only PNG; manifest v0/v1; conflict-copy names.
- **Walkthrough:** [Appendix A](#appendix-a-walkthrough), 10 steps incl. a manual OneDrive check (R13).

## 14. Build split
| # | Slice | Shared files (AGENTS §9) | Depends on |
|---|---|---|---|
| 1 | Store core: atomic writer (temp, rename, retry, fault seam), manifest v1 types, migrations, version guard, `settings.json` | manifest schema (new) | skeleton |
| 2 | Library config + import (paths, folder, clipboard, validation, dedupe, hash) + home drop zone | command + UI registries (append) | 1 |
| 3 | Open/create/close: original checks, relink, fs-attributes seam, recent list, `DecodedOriginal`, read-only | command registry (append) | 1 |
| 4 | Store API: jobs, candidates, masks, patch layers, revisions, draft, export; roll-forward; undo via restore | manifest schema (add fields), command registry | 3. RGN, EDT, EXP build on it |
| 5 | Retention: trash, purge, restore, disk use + project panel | command registry (append) | 4 |
| 6 | Conflicts: copy + seq detection, spool, conflict view, resolve, status bar | command registry (append) | 4 |

## 15. Open questions
| ID | Question | Options (pick in **bold**) | Blocking? | Affects |
|---|---|---|---|---|
| OQ-PRJ-10 | A conflict copy of a `job.json` | **A. The copy with the later state (diagram order) wins; the other goes to the trash** / B. Follows the user's keep choice. A never drops a job handle or paid candidates; B is stricter to #39 but can lose candidate refs to the trash. | No | RGN |
| OQ-PRJ-11 | How the conflict view draws each version ([editor-canvas](editor-canvas.md)) | **A. EDT's canvas draws a given layer stack read-only (not only the manifest's)** / B. Text only (time, layer count, revision label). A lets the user see what they keep; B needs nothing from EDT. | No (slice 6; B is the fallback) | EDT |

**Resolved:** OQ-PRJ-01 ✅ A, `<file>.pforge/` (#40) · OQ-PRJ-02 ✅ A, `Imported/` (#40) · OQ-PRJ-03 ✅ A, first commit (#40) · OQ-PRJ-04 ✅ A, commit every change, draft after 2 s idle (#40) · OQ-PRJ-05 ✅ A, `sha2` (#38) · OQ-PRJ-06 ✅ restore the parent revision (R3, FR-PRJ-43) · OQ-PRJ-07 ✅ A, PRJ minimal revision list, recent list, relink (#40) · OQ-PRJ-08 ✅ added to the glossary (R27) · OQ-PRJ-09 ✅ EDT owns the tile cache, 2 GiB (R5) · OQ-PRJ-12 ✅ A, Head revision, Import destination and Spool added (R29) · OQ-EDT-14 (asks PRJ): compatible with EDT's pick A, a restore brings back the revision's draft and EDT re-submits the live mask.

## Appendix A: walkthrough
`PF_DATA_DIR` + `PF_LIBRARY_DIR` in `.pf-dev/`, fake provider.

| # | Step | Expected |
|---|---|---|
| 1 | First launch, pick `.pf-dev/library` | Empty home with the drop zone |
| 2 | Drop a PNG, a JPEG and a `.psd` from outside the library | 2 in `Imported/`, 1 `unsupported`; sources unchanged |
| 3 | Paste an image; drop a folder with subfolders | `pasted-….png`; structure kept |
| 4 | Open the PNG | Displays; no `.pforge` yet |
| 5 | Paint, submit (3 candidates), accept one, star one | Manifest, `jobs/<id>/` (job + inputs), mask, layer, `r0`, `r1`; 1 candidate in `trash/` |
| 6 | Kill during a commit (`PF_FAULT=after_temp`), relaunch | Last committed state, no stray files |
| 7 | Copy `manifest.json` to `manifest-OTHERPC.json`, reopen | Conflict view shows both; Keep this → `r2` (`conflict`), copy in trash |
| 8 | Relaunch with `PF_FAKE_NOW` + 31 days | Trashed candidate gone; starred and accepted remain |
| 9 | Restore `r0`, close, reopen | Original only; `r0`–`r3` intact; pixel-identical |
| 10 | Manual: a cloud-only original in a real OneDrive folder, online then offline | Hydrates once; offline → `not_available_offline` |

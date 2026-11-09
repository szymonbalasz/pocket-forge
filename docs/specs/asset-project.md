---
feature: asset-project
prefix: PRJ
owner: rev-asset-project
status: complete   # unclaimed → claimed → draft → in-review → complete (owner only)
depends_on: []
---

# Asset project and library folder

## 1. Summary
The **library** is plain folders the user picks, usually cloud-synced (#5, #13). Each **library root** holds three **sections**, `Maps/`, `Tokens/` and `Items/`, and each **asset project** is a visible **project folder** in one of them (#44). This spec covers the folder layout, "new from image" (a copy; the source is never touched), the home grid, section rail and left project panel (#45, [ui-guidelines](../architecture/ui-guidelines.md)), crash-safe commits, sync and locked files, retention and **trash** (#18), the **local cache** split (#19), app settings and the decoded original. Every other spec reads and writes project state through PRJ's store API. Field shapes live in [manifest-schema](../architecture/manifest-schema.md) (R26).

## 2. Scope
Tier S ([feature-tiers](../feature-tiers.md)): *Open an existing image as an asset project*, *Save and reopen the asset project*, and *Sections, project-as-folder layout, home grid* (#44, #45), plus the storage side of #16, #18, #19, #20, KQ-13, KQ-16, KQ-17. Includes a minimal revision list and relink (#40). For reference packs (#51) it adds the `References/` folder in each root and the rail entry (FR-PRJ-51–54); packs themselves are [reference-packs](reference-packs.md) (REF).

## 3. Goals
Browsable in Explorer: one folder per project, a home for references and results · zero-dialog creation (drop, paste, multi-file, folder) · pixel-identical reopen (#35), crash-safe commits · sync-friendly files (write-once, relative paths, conflict detection) · bounded, visible disk growth.

## 4. Non-goals
Search, tags, collections (tier A) · using `structure/` or `parts/` in generation, placing parts (tier A, #45); sending `references/` is REF's (#51) · moving a project between sections in the app · blank projects · several machines editing at once (KQ-13) · record-level conflict merge (#39) · persisted undo (KQ-16) · "empty trash now" · a file watcher.

## 5. User stories
- I drop three maps on the Maps home. Each becomes a project card with its thumbnail; my source files stay where they were. In Explorer I see `Maps/Crypt/results/003 kitchen/` with the candidates I kept, and I drop mood images into `Maps/Crypt/references/`; the project panel lists them.
- I close the laptop mid-edit and open the desktop later: everything I accepted is there. If both machines touched the project, I see both versions, keep one, and the other stays in the history.

## 6. Functional requirements
FRs marked (#44) were rewritten for the project-as-folder model; retired ones keep their number.

| ID | Requirement (testable) |
|---|---|
| FR-PRJ-01 | Library roots are per machine, in `settings.json` `library.roots[]`; the first receives new projects (#44). A root must exist, be writable, and not be nested in another root or `<data-dir>`; each violation is a field-level error. Setting a root creates any missing `Maps/`, `Tokens/`, `Items/` in it. |
| FR-PRJ-02 | A root unavailable at startup shows **Library unavailable**. Nothing is written under it. |
| FR-PRJ-03 | (#44) **New from image** takes paths from a drop on the home grid, **+ New → From image…** (the multi-select dialog) and a pasted file list (`project_create`), or pasted image data (`project_create_from_clipboard`). Projects go into the selected section. **+ New** is a menu: From image… \| Generate new… ([new-asset-generation](new-asset-generation.md) FR-GEN-01, #60). |
| FR-PRJ-04 | (#44) Every source is **copied** to `<section>/<name>/working/<source file name>`, also when it is already inside the library (e.g. a result of another project). The source's size, mtime and hash are unchanged afterwards. |
| FR-PRJ-05 | Retired (#44): sources inside a root are no longer used in place. |
| FR-PRJ-06 | Accepted: PNG, JPEG, WebP (KQ-28), ≤ 16384 px per side (#11), passing the end-of-file check (R33). Anything else is skipped with `unsupported`, `animated`, `too_large` or `corrupt`, and the batch continues. |
| FR-PRJ-07 | (#44) A dropped folder makes one project per supported image found recursively; folder structure is not kept. |
| FR-PRJ-08 | (#44) A taken name gets ` (2)`, ` (3)`, … (FR-PRJ-44). A source whose hash equals an existing project's original in the same section creates nothing and reports `already_in_library{project}`. |
| FR-PRJ-09 | (#44) A project is built in `<section>/.<name>.pf-tmp-<ulid>/` (working copy, subfolders, `project.json`, `r0`), then renamed to `<section>/<name>/`. An interrupted creation never leaves a partial project under a final name; temp folders older than 1 h are removed at startup (R35). |
| FR-PRJ-10 | (#44) Pasted data becomes project `pasted-<UTC yyyymmdd-hhmmss>` (clock seam) with a lossless PNG `working/pasted-<…>.png`. |
| FR-PRJ-11 | Creation emits `import:progress` per file. `project_create_cancel` stops it; projects already finished stay. |
| FR-PRJ-12 | (#44) A project folder holds `project.json` (the **manifest**), `working/`, `references/`, `structure/`, `parts/`, `results/`, `exports/` and a hidden `.pforge/` (§8). Project files hold only project-relative paths and never credentials. |
| FR-PRJ-13 | Retired (#44): creation writes the project and `r0` at once (FR-PRJ-09); opening never creates one. |
| FR-PRJ-14 | (#44) `project_open` takes a project folder (one with `project.json`) inside a root; anything else → `not_a_project` or `outside_library`. |
| FR-PRJ-15 | The **original** `working/<file>` is never written, moved or renamed by the app. On open, size and mtime are compared with the manifest; if they differ, the hash is checked, and a different hash (e.g. saved over by another editor) opens **read-only** ("Original changed"). |
| FR-PRJ-16 | (#44) A missing original opens with "Original missing" and **Relink**, which accepts a file only if its hash matches and copies it back into `working/`. |
| FR-PRJ-17 | (#44) Cloud-only files (fs-attributes seam: `is_cloud_only`, `hydrate`, R13) are hydrated only when needed: `project.json` and the thumbnail for the home grid, the original on open, and a file the user views (a thumbnail in the left panel or tracker). Offline → `not_available_offline`, nothing written. Nothing else is ever hydrated (#19, KQ-14). |
| FR-PRJ-18 | Every mutating command commits before it returns success: new write-once files, then `project.json`, each temp → flush → rename. The manifest rename is the commit point ([schema §3](../architecture/manifest-schema.md#3-write-rules)). |
| FR-PRJ-19 | The draft is saved by `project_put_draft` ≥ 2 s after the last draft or export-settings change and on close (#40); the same commit writes EXP's unsaved export settings (R30). RGN's and GEN's pending draft edits are saved the same way, on close and on project switch (R97). Quitting the app (the window X) is a close (R101). Opening never commits by itself: the UI's first load of the draft and export settings isn't an edit and sends nothing back (FR-EXP-25, R101); only the open sweep (FR-PRJ-20) may commit. `put_mask` (FR-EDT-28) writes the mask and points the draft at it in one commit. Masks no draft, job or revision references go to the trash at the next commit (R7). |
| FR-PRJ-20 | After a kill at any step of a commit, reopen shows exactly the state before or after it. On open, temp files are deleted (in `exports/` only FR-EXP-17's stale temps, `.*.pf-tmp-*` ≥ 1 h old, so a running export's temp survives; R70), roll-forward steps run, and files in `.pforge/` that no manifest field, revision or job file references go to the trash. User-owned files are never swept (FR-PRJ-47). **Incomplete refs** (R102, R105): if a revision or `job.json` can't be read this open, nothing is called an orphan, no superseded mask is trashed (R7) and nothing is purged (FR-PRJ-32). A job folder without a `job.json` skips the orphan sweep for that open if any of its files is less than 7 days old (its `job.json` may still be syncing); an older one is a crash leftover and is swept as usual. |
| FR-PRJ-21 | A locked file is retried with backoff for ≤ 3 s, then `project:save-failed{reason: locked}` fires, state stays in memory and is retried every 10 s, and closing with unsaved state asks first. Quitting the app runs the same close guards (R101): the window holds its close request (Tauri `CloseRequested` → `prevent_close`), the UI flushes pending saves and asks "Close anyway?" on `unsaved` and the export question (EXP §7, OQ-EXP-09), then exits. A refused or unanswered guard keeps the app open. Before the UI has loaded, the window X closes as usual; the chat window's X is not held. |
| FR-PRJ-22 | Save → close → reopen is pixel-identical for the original, layers and masks, and field-identical for prompts, settings, jobs, candidates and revisions. |
| FR-PRJ-23 | A **revision** is a write-once snapshot of the layer stack plus the draft. Reasons: `create` (`r0`), `accept`, `save` (Ctrl+S, only if changed since the head), `restore`, `retune`, `conflict` (R24). Restore writes a new revision with the restored layers and draft; the current `draft.generate` is kept, not restored (the New image draft is not part of undo, R98). Optional `label`; `r<seq>` is a display label. None is ever deleted. |
| FR-PRJ-24 | If `project.json` is missing or unreadable, open rebuilds it from the newest revision and the revision and job files, and reports `project:recovered`. An unreadable file is never overwritten without a copy: before the rebuild its bytes go to `.pforge/broken/project.<stamp>.json` (an identical copy is written only once); if that copy fails, the open fails and nothing is written. The rebuild keeps the old `project_id` when it can be read, from the JSON or by a text scan of a truncated file (R102). |
| FR-PRJ-25 | `schema_version` migrations run in order on load; the migrated file is written at its next commit, keeping `.pforge/project.v<N>.json` ([schema §6](../architecture/manifest-schema.md#6-versioning-and-migration)). |
| FR-PRJ-26 | A newer `schema_version` than supported opens **read-only** ("Made by a newer Pocket Forge", warning `newer_version`). Nothing is written. Valid JSON with an enum value this build doesn't know opens read-only the same way, from a lenient read: list entries that don't parse are left out, and an unknown `kind` follows the folder (FR-PRJ-46). It is never rebuilt or rewritten (R102, R105). |
| FR-PRJ-27 | Unknown fields are kept on write. JSON is UTF-8, LF, pretty-printed, stable field order. |
| FR-PRJ-28 | A job record is `.pforge/jobs/<id>/job.json` + `region.png` + `mask.png` (as sent: after fit, before polarity conversion). State is persisted on **every** transition, with PRV's opaque `provider{}` verbatim. The job file is authoritative; manifest `jobs[]` is an index rebuilt on open (R2). |
| FR-PRJ-29 | `project_open` reports every non-terminal job as resumable (RGN consumes it). A `requested` job (no handle) reopened after a crash becomes `lost`, only when this process isn't submitting it and it is older than the submit timeout plus 60 s (FR-RGN-22, R102). |
| FR-PRJ-30 | (#44) Candidates are stored untouched as `results/NNN <slug>/NN.<sniffed ext>`, one folder per **iteration** (FR-RGN-39, R10). The folder name is committed in `job.json` before any file is written, so a re-fetch after a crash replaces only its own unreferenced files. Accepted candidates and files any revision references are never trashed or purged (#18). Other candidates are trashed only by an explicit Discard (#56). |
| FR-PRJ-31 | (#56; amends R9) Accept = one commit: patch layer, candidate statuses (the job's other `new` candidates → `dismissed`, kept in `results/`) and revision (R4). Accept, dismiss and the next submit trash no candidate. `discard_candidate` and `discard_rest` (one commit, one trash item, R65) send candidates to the trash; an accepted one is refused. A job's inputs go to the trash only once it is resolved and every candidate is discarded (invalid ones count; a job with no candidates trashes them on resolve, R66); accepted jobs keep inputs forever (KQ-17). A trashed result leaves `results/`; an iteration folder left empty is removed (OQ-RGN-16). Starring or restoring brings an item back to its old path, as `dismissed`; inputs already trashed stay in the trash (R66). |
| FR-PRJ-32 | On open, trash items older than 30 days (clock seam) are deleted. Nothing else is ever deleted by the app. A file the purge can't delete (a kept file, or a failed remove) stays in its item and is retried at the next open; a candidate gets `purged_at` only when its file is really gone. When nothing could be deleted, nothing is committed (R102). |
| FR-PRJ-33 | (#44) Disk use per project by category (original, layers, masks, job inputs, results, trash, user folders, exports, other) comes from file metadata only and never hydrates a file. |
| FR-PRJ-34 | Open and every commit look for **conflict copies** of `project.json` and `job.json`: the name with a sync suffix (`-<any>` or ` (<n>)`) before the extension. A conflict copy of the thumbnail goes to the trash. Between `job.json` copies the later state wins, then the later write (OQ-PRJ-10), except that a copy with a job handle or candidates beats a `lost` or `failed` one (R102). |
| FR-PRJ-35 | Before every manifest write, on-disk `commit{seq, install_id}` is compared with the last loaded or written values. A mismatch refuses the write and fires `project:conflict`. |
| FR-PRJ-36 | On a conflict, `project_conflict_get` returns both versions (time, this/other machine, layer stack, newest revision label, missing files). The user keeps one; one commit makes it current, writes the other's layers + draft as a `conflict` revision, and trashes the conflict copy. Nothing is merged; files from both sides stay and indexes are rebuilt (#39). A version with missing files can't be kept yet. |
| FR-PRJ-37 | `<data-dir>` (`PF_DATA_DIR`, default per-user local app data, never synced) holds caches, which rebuild, and per-machine config. Deleting it loses no committed project data (R8). |
| FR-PRJ-38 | The tile cache lives under `<data-dir>/cache/tiles/`. Key, cap (2 GiB) and eviction are EDT's ([editor-canvas](editor-canvas.md), R5). |
| FR-PRJ-39 | `settings.json` has sections `library`, `spend`, `provider`, `region_edit` (#55), `prompt_helpers` (PRM, #60; an invalid section falls back to the starter helpers with a warning, without resetting the file) and is written temp → rename. A corrupt file → defaults plus a warning, the bad file kept aside (R8). |
| FR-PRJ-40 | While a write can't commit (conflict, `locked` after 3 s, read-only, or the folder moved: `not_found` with `folder_moved`, R105), candidate bytes and job updates go to the spool `<data-dir>/pending/<project-id>/` and are replayed in order once writable. A crash in between loses no paid output (R12). An entry's `.json` is removed before its bytes. An entry that can never replay (it won't parse, its bytes are gone, its files are taken, or the store refuses candidates while their job still waits for them) moves with its bytes to `pending/<project-id>/quarantine/`, so the entries after it go on, and fires `project:spool-quarantined` (not for one that won't parse). A quarantined `seq` is never reused (R102, R105). |
| FR-PRJ-41 | In a **read-only** project the user can view and paint. Every writing command and store call fails fast with `read_only`; the UI disables submit (R12). |
| FR-PRJ-42 | PRJ owns a `DecodedOriginal` (RGB8, ADR 0004) of `layers[0]`'s file: the original, or a picked base image (FR-GEN-33, #60), with alpha composited over the flatten colour `#808080` (FR-GEN-12). Decoded lazily on the first RGN or EXP use, prefetched by `project_prefetch_original`, released on close and replaced when a commit changes `layers[0]`. Until ready, RGN and EXP get `original_unavailable` (R11); a project with no image gets `no_image` (FR-GEN-03). |
| FR-PRJ-43 | Undo of an accept or re-tune = `project_restore_revision(parent)` (reason `restore`); redo = restore the later revision. Candidate statuses and trash are unchanged (R3). |
| FR-PRJ-44 | (#44) **Name** = the folder name: the source stem by default, with `<>:"/\|?*` and control characters replaced by `-`, trailing dots and spaces trimmed, reserved Windows names (`CON`, `NUL`, `COM1`, …) suffixed with `_`, ≤ 64 chars; empty → `untitled`. Names compare case-insensitively within a section. |
| FR-PRJ-45 | (#44) `project_rename` renames a **closed** project's folder under FR-PRJ-44 rules; nothing inside changes (paths are relative). Taken → `name_taken`; open → `project_open`; locked → `locked`. |
| FR-PRJ-46 | (#44) `project.json` `kind` is `map`, `token` or `item`, from the section. A project found in another section's folder (moved in Explorer) is listed there and takes that `kind` at its next commit. |
| FR-PRJ-47 | (#44) App-owned: `project.json`, `working/` (written at creation and relink, and at a pick: one new write-once file `working/NNN <slug> NN.<ext>`, never overwritten or deleted, FR-GEN-27, #60), `results/`, `exports/` (EXP) and `.pforge/`. User-owned: `references/`, `structure/`, `parts/`; the app only adds copies there (`project_folder_add`) and never moves, renames or deletes in them. Files the app didn't name in `results/` or `exports/` are ignored, never swept. |
| FR-PRJ-48 | (#44) User folders are listed on open, when the window regains focus and on Refresh: supported images with a thumbnail, other files by name. Files dropped there in Explorer appear without a restart. |
| FR-PRJ-49 | (#44, #45) `library_list_projects(section)` reads only `project.json` and the thumbnail of each project folder in every root. Cards: thumbnail, name, iteration count, last edited (`commit.at`), newest first, with badges read-only, conflict, unreadable, not available offline, no image. A project with no layers (Generate new before its first pick) shows a placeholder and the "No image yet" badge instead of a thumbnail, with its iteration count as usual (FR-GEN-03, #60). A folder without `project.json` is ignored. |
| FR-PRJ-50 | (#44) The **thumbnail** `.pforge/thumb.jpg` (≤ 512 px long side, JPEG q80) of the **working image** is written in the background after creation and after each commit that changes `layers[]`, temp → rename, outside the commit (OQ-PRJ-13). Missing or older than the last layer change → rebuilt on open. It never blocks a commit. With no layers (no image, FR-GEN-03) the thumbnail is removed instead. |
| FR-PRJ-51 | (#51) Setting a root also creates a missing `References/` in it. It is **not a section**: `library_list_projects` never lists it, `project_create` never targets it, and a `project.json` found under it is ignored. Its contents are REF's (FR-REF-01). |
| FR-PRJ-52 | (#51) The rail has a **References** entry between Items and Settings (`prj-rail-references`). Selecting it shows REF's packs home in the home slot; REF registers that view through the UI feature registry. |
| FR-PRJ-53 | (#51) REF may read files in a project's `references/` to send them with a job and may hydrate a cloud-only one at that moment (amends FR-PRJ-17's list). FR-PRJ-47 still holds: the app never moves, renames or deletes there. |
| FR-PRJ-54 | (#51) The draft carries REF's `references` and the job record its `references[]` ([schema §4.5, §4.6](../architecture/manifest-schema.md#45-draft)); `put_draft`, `record_job` and the rebuild of FR-PRJ-24 keep them verbatim. |

## 7. UI/UX notes
Slots per [ui-guidelines](../architecture/ui-guidelines.md); PRJ owns the section rail, the home grid and the left panel.
- **Section rail** (`prj-rail-{maps,tokens,items,references,settings}`): picks the section; the home grid shows it. References shows REF's packs home instead (FR-PRJ-52). **First run:** "Choose your library folder" (`prj-setup-pick-root`); **library unavailable:** banner with "Choose again".
- **Home** (no project open): project cards (`prj-home-card`, `data-record-id` = project id; open on click; Rename in the context menu), **+ New** (`prj-home-new`), full-area drop zone (`prj-home-dropzone`), paste. Empty section: "Drop an image to start a map / token / item".
- **Creation:** toast "12 / 40" with Cancel (`prj-import-cancel`); the summary lists created, already in library (with Open), skipped + reason. No confirmation dialog.
- **Left panel** (`prj-panel-*`): Working, References, Structure, Parts (file lists, **Add…** copies in, Refresh), Results (iterations, RGN's tracker selects them; an iteration opens to its kept candidates, each with **Discard** `prj-results-discard`, disabled on the accepted one, no confirm, notice "in the trash for 30 days (Storage → Restore)", #56), then History (revisions `r<seq>`, label, Restore) and Storage (disk use, trash count, size, next purge date, trashed items with Restore).
- **Status bar segment** (`prj-status-save`): Saved · time / Saving… / Save failed, file locked, retrying / Read-only (reason) / Conflict.
- **Loading:** "Downloading from cloud…", then the backdrop (EDT). **Offline:** `not_available_offline` with Retry. **Job running:** closing is allowed; the job resumes on reopen (FR-PRJ-29).
- **Conflict:** banner "This project was also changed on another machine" → a two-pane view (`prj-conflict-this`, `prj-conflict-other`) with time and layer stack per side. Each pane draws its stack with EDT's `LayerStack` (`prj-conflict-<side>-stack`, `data-state` = `loading|drawn|fallback`), the text list kept as the caption; a pane with any missing or unreadable layer file shows the text list only (OQ-PRJ-11 A, R71), and **Keep this** / **Keep other** (`prj-conflict-keep-this`, `-keep-other`). A side with missing files shows "Still syncing" and its Keep is disabled.

## 8. Data and file effects
Full tree and field tables: [manifest-schema](../architecture/manifest-schema.md). In short:
- **Synced, per project** `<library-root>/<Maps|Tokens|Items>/<name>/`: `project.json` (mutable, commit point) · `working/<file>` (the original) · `references/`, `structure/`, `parts/` (user-owned) · `results/NNN <slug>/NN.<ext>` (write-once) · `exports/` (EXP) · `.pforge/`: write-once `revisions/`, `layers/`, `masks/`, mutable `jobs/<id>/job.json` + write-once inputs, `trash/`, `thumb.jpg` (rebuildable), `broken/` (copies of unreadable `project.json` files, FR-PRJ-24).
- **Synced, per root** `<library-root>/References/`: reference packs, owned by REF (FR-PRJ-51).
- **Local (`<data-dir>`):** `settings.json`, `cache/tiles/` (EDT), `cache/provider/` (PRV), `pending/` (spool, with `quarantine/`, FR-PRJ-40). `recent.json` is retired (OQ-PRJ-14).
- **Growth and recovery:** each re-tune writes a new layer PNG kept by its revision; acceptable (#18), shown in disk use. A lost manifest is rebuilt (FR-PRJ-24). Trash restores for 30 days. The original and accepted candidates are never deleted. The thumbnail rebuilds. A lost `<data-dir>` costs re-picking roots, the spend cap, cache rebuilds, and any unreplayed spool.
- **Migration:** none. No real projects exist yet (only dev data in `.pf-dev/`, which is deleted), so manifest v1 is redefined in place and there is no migration code. `library.import_dest` is dropped from `settings.json`; an old value is kept as an unknown field and ignored.

## 9. Commands and events
| Command | Input → result | Errors |
|---|---|---|
| `library_get_config` / `library_set_roots` | — / `roots[]` → config | FR-PRJ-01 field errors |
| `library_list_projects` | section → cards (FR-PRJ-49) | `library_unavailable` |
| `project_create` / `project_create_cancel` | section, `paths[]` → per item `created{path}` \| `already_in_library{path}` \| `skipped(reason)` | `library_unavailable`, `canceled` (partial results) |
| `project_create_from_clipboard` | section, image bytes → project path | `unsupported`, `too_large` |
| `project_rename` | path, name → new path | `name_taken`, `invalid_name`, `project_open`, `locked` |
| `project_open` / `project_close` | project folder → snapshot (`kind`, name, layers, draft, revisions) + `warnings[]` (read-only reason, conflict, recovered, resumable, lost) / — → ok | `not_found`, `not_a_project`, `outside_library`, `not_available_offline`, `corrupt`, `unsaved` (another open project still has waiting changes, R101) / `unsaved` |
| `project_folder_list` / `project_folder_add` | folder (`working\|references\|structure\|parts\|exports`) / folder, `paths[]` → files `[{name, bytes, mtime, image}]` | `unknown_folder`, `read_only` (add), `not_found` (`detail.reason = folder_moved`, BL-30) |
| `project_put_draft` | draft (+ EXP's unsaved export settings, R30) → `seq` | `read_only`, `locked`, `conflict` |
| `project_list_revisions` / `project_save_revision` / `project_restore_revision` | — / label? / `rev_id` → `[{id, seq, created_at, reason, label}]` / revision | `unchanged`, `unknown_id`, `read_only` |
| `project_trash_list` / `project_trash_restore` | — / item id → items / ok | `unknown_id`, `read_only`, `not_restorable` (kinds other than `candidates`, `job_inputs`, `conflict_copy`; R51), `not_found` (`folder_moved`, list, BL-30) |
| `project_disk_usage` / `project_relink_original` | — / path → bytes per category + next purge date / ok | `not_found` (`folder_moved`, never 0 B, BL-30) / `hash_mismatch` |
| `project_prefetch_original` | — → ok (decode starts) | `corrupt`, `not_available_offline` |
| `project_conflict_get` / `project_resolve_conflict` | — / `keep: this \| other` → both versions / revision | `no_conflict`, `files_missing` |

- Retired (#44): `import_paths`, `import_clipboard_image`, `import_cancel` (→ `project_create*`), `recent_list` (OQ-PRJ-14).
- **Store API** (Rust, one commit each, `read_only`/`locked`/`conflict` as above, R4): `read_project` (current layer stack, R6), `record_job`, `update_job`, `add_candidates` (assigns the iteration folder, FR-RGN-39), `put_mask`, `accept_candidate` (also restores a trashed candidate file for a re-pick, FR-RGN-36), `set_star`, `discard_candidate`, `discard_rest` (all but accepted and starred, one trash item, #56), `resolve_job`, `replace_patch_layer` (re-tune), `put_export` (export settings + `last`), `settings_get` / `settings_put(section)`, `decoded_original`, `resolve_image(image_id)` (original, layer or candidate → file, for EDT). User-facing job and candidate commands are RGN's (`rgn_*`).
- **Events:** `import:progress{done, total}`, `import:done`, `project:committed{seq}`, `project:save-failed{reason, retrying}`, `project:conflict`, `project:recovered`, `project:original-ready`, `project:original-failed{project_id, code}` (the background decode failed; kept until close, R104), `project:thumb-updated{project_id}`, `trash:purged{count, bytes}` (R21), `project:spool-dropped{project_id, job_id, seq, op, code}` (a spooled write refused for good; BL-25), `project:spool-quarantined{project_id, job_id, seq, op}` (a spooled write set aside with its bytes, FR-PRJ-40, R105).

## 10. Provider interaction
N/A: no provider calls. PRJ persists the job record ADR 0001 requires (KQ-17) and keeps the opaque `provider{}` as a cache (#20).

## 11. Edge cases
| Case | Behaviour |
|---|---|
| Crash mid-save | FR-PRJ-18/20; job follow-ups roll forward on open. Mid-creation: only a temp folder, removed (FR-PRJ-09). |
| Sync conflict / locked file | FR-PRJ-21, 34–36, 40. Two machines creating the same name offline get one merged folder with a `project.json` conflict copy: the conflict view shows both, and the kept side's sweep trashes the other's `.pforge/` files (30-day restore). |
| Folder renamed or moved in Explorer | Closed: works (relative paths), listed where it now is (FR-PRJ-46). Open: the next commit fails with `not_found`, state stays in memory, and the banner asks to reopen from home. Job updates and candidates are refused the same way, never written to a recreated old path; the runner spools them and they replay after the reopen (FR-PRJ-40, R105). Folder lists, Storage and the trash list fail with `not_found` (`folder_moved`) and the UI clears stale figures, never showing 0 / 0 B (BL-30). |
| Offline / provider error | Only cloud-only files are affected (FR-PRJ-17). PRJ makes no provider calls; the job record keeps any error. |
| Very large image | > 16384 px rejected (FR-PRJ-06). Hashing streams and is skipped while size and mtime match. 16k decode and thumbnail are background (FR-PRJ-42, 50). |
| Corrupt input or settings | Skipped at creation by the header and end-of-file check (R33). A decode failure on open → `corrupt`, nothing written. Unreadable `project.json` → FR-PRJ-24 (copied to `.pforge/broken/` first); an unknown enum value → read-only (FR-PRJ-26). Corrupt `settings.json` → FR-PRJ-39. |
| Undo across features | In-session only (KQ-16); accept/re-tune per FR-PRJ-43. |

## 12. Performance budget
| Operation | Typical map (#30) | 16k |
|---|---|---|
| `library_list_projects`, 200 local projects (manifest + thumb stat) | < 500 ms | same |
| `project_open`, PRJ share (manifest + layer metadata, no hash) | < 150 ms | < 300 ms |
| A commit (manifest ≤ 200 KB, local SSD) / patch layer PNG write (≤ 4096²) | < 50 ms / < 300 ms | same |
| New from image: copy + hash / thumbnail (background) | < 1 s / < 1 s | < 5 s / < 6 s |
| `DecodedOriginal` decode (background) / memory; PRJ other memory < 20 MiB | < 0.5 s / ≤ 70 MiB | < 4 s / 768 MiB |
| Whole app (EDT UI 704 + core transient 256 MiB, decoded original, PRJ 20 MiB, plus RGN job 400 MiB / EXP stream 400 MiB / EXP rescale 1.6 GiB) | < 1.5 GiB | ≤ 2.1 GiB / ≤ 2.1 GiB / ≤ 3.3 GiB |

## 13. Test plan
- **Unit (`pf-core`, `fr_prj_NN_*`):** manifest round-trip + field order; conflict-copy matcher; name sanitising and collision suffixes (44, 08); iteration slug and numbering (with RGN); purge date math (fake clock); creation validation; settings corrupt → defaults.
- **Integration (Rust):** fault-injecting fs seam at each commit step and during creation → reopen = before or after; no partial project folder (09, 20). Lock: k failures then success (21). Seq-mismatch refusal; spool replay after resolve (35, 40). Conflict resolve keeps both sides' files (36). Fake fs-attributes: cloud-only + offline, home listing hydrates only manifest and thumb, disk use never hydrates (17, 33). Creation from a generated tree with the sources re-hashed (04, 07). Rename (45). A file dropped in `references/` is listed and never swept (47, 48). Save → reopen identity with a **fake provider** patch (22). Rebuild from a deleted manifest. R9 retention and trashed results leaving `results/` (31). Undo via restore (43). Read-only refusals. A real-process kill test.
- **Thin e2e (Playwright, mocked core):** rail → home grid; drop → summary → cards; status bar states; left panel lists; conflict view; read-only banner; revision restore.
- **Fixtures (generated, KQ-22, no personal art):** procedural grid map 2100×2800; small PNG/JPEG/WebP; animated WebP; truncated PNG; PSD-magic file; 16385 px header-only PNG; manifest v1; conflict-copy names; awkward names (`CON.png`, `a:b?.png`, 80-char stem).
- **Walkthrough:** [Appendix A](#appendix-a-walkthrough), incl. a manual OneDrive check (R13).

## 14. Build split
| # | Slice | Shared files (AGENTS §9) | Depends on |
|---|---|---|---|
| 1 | Store core: atomic writer (temp, rename, retry, fault seam), manifest v1 types (`project.json`, `kind`), version guard, `settings.json` | manifest schema | skeleton |
| 2 | Library + creation: roots, sections, `project_create*`, naming, dedupe, temp-folder build, thumbnail; home grid, rail, drop zone | command + UI registries (append) | 1 |
| 3 | Open/close: original checks, relink, fs-attributes seam, `DecodedOriginal`, read-only, rename; left panel folders | command registry (append) | 1 |
| 4 | Store API: jobs, candidates (iteration folders), masks, patch layers, revisions, draft, export; roll-forward; undo via restore | manifest schema (add fields), command registry | 3. RGN, EDT, EXP build on it |
| 5 | Retention: trash, purge, restore, disk use; History and Storage in the left panel | command registry (append) | 4 |
| 6 | Conflicts: copy + seq detection, spool, conflict view, resolve, status bar | command registry (append) | 4 |
| 7 | (#51) `References/` on root set, not a section (51); rail entry (52); draft/job fields (54). Built inside REF slices 1, 3 and 6 | manifest schema (add fields), UI registry | 2, 4 |

## 15. Open questions
None open.

**Resolved:** OQ-PRJ-01 ✅ superseded by #44 (project folder) · OQ-PRJ-02 ✅ superseded by #44 (copy into the project's `working/`) · OQ-PRJ-03 ✅ superseded by #44 (creation writes `r0`) · OQ-PRJ-04 ✅ A, commit every change, draft after 2 s idle (#40) · OQ-PRJ-05 ✅ A, `sha2` (#38) · OQ-PRJ-06 ✅ restore the parent revision (R3, FR-PRJ-43) · OQ-PRJ-07 ✅ A, minimal revision list and relink (#40); recent list → OQ-PRJ-14 · OQ-PRJ-08 ✅ glossary (R27) · OQ-PRJ-09 ✅ EDT owns the tile cache, 2 GiB (R5) · OQ-PRJ-12 ✅ A (R29) · OQ-PRJ-10 ✅ A, the later `job.json` wins (#41) · OQ-PRJ-11 ✅ A, EDT draws a given layer stack read-only (#41) · OQ-EDT-14 ✅ A (#41). · OQ-PRJ-13 ✅ A, `.pforge/thumb.jpg` synced (#49) · OQ-PRJ-14 ✅ A, recent list retired; the home grid replaces it (#49) · OQ-PRJ-15 ✅ A, no flattened copy in `working/` (#49) · OQ-PRJ-16 ✅ A, EDT adds a `library` key space on `/i/` (#49)

## Appendix A: walkthrough
`PF_DATA_DIR` + `PF_LIBRARY_DIR` in `.pf-dev/`, fake provider.

| # | Step | Expected |
|---|---|---|
| 1 | First launch, pick `.pf-dev/library` | `Maps/`, `Tokens/`, `Items/` created; empty Maps home with the drop zone |
| 2 | Drop a PNG, a JPEG and a `.psd` from outside the library on Maps | 2 cards with thumbnails, 1 `unsupported`; sources unchanged; each folder has the §8 layout |
| 3 | Paste an image; drop a folder with subfolders on Tokens; drop the PNG again on Maps | `pasted-…` project; one Tokens project per image; `already_in_library` |
| 4 | Copy an image into `Maps/<name>/references/` in Explorer, focus the app, open the project | References lists it |
| 5 | Paint, submit (3 candidates), accept one, star one, Discard the third in Results | `results/001 <slug>/` holds the accepted and starred ones; the discarded one in `.pforge/trash/` (#56); `r0`, `r1`; card thumbnail shows the patch, iteration count 1 |
| 6 | Kill during a commit (`PF_FAULT=after_temp`), relaunch | Last committed state, no stray files |
| 7 | Copy `project.json` to `project-OTHERPC.json`, reopen | Conflict view shows both; Keep this → `r2` (`conflict`), copy in trash |
| 8 | Relaunch with `PF_FAKE_NOW` + 31 days | Trashed candidate gone; starred and accepted remain |
| 9 | Restore `r0`, close, rename the project from its card, reopen | Original only; `r0`–`r3` intact; pixel-identical |
| 10 | Manual: library in a real OneDrive folder, a cloud-only project, online then offline | Home hydrates only manifest and thumb; open hydrates the original once; offline → `not_available_offline` |

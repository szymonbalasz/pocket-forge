---
feature: export
prefix: EXP
owner: rev-export
status: complete   # unclaimed → claimed → draft → in-review → complete (owner only)
depends_on: [asset-project]
---

# Flattened export

## 1. Summary
Export flattens an asset project's **current** layer stack (the original plus its visible patch layers, R6) into one plain image file for the owner's own VTT (#24, #26). This is MVP scenario step 7. Rust is the only compositor (KQ-25). It flattens at source resolution in 512-row strips (ADR 0004), can resample to a chosen target px per square, and writes PNG, JPEG or WebP atomically into the project's own `exports/` folder (#44). The same `compose_rect` function feeds region extraction (FR-RGN-04) and the golden-image tests.

## 2. Scope
Tier S ([feature-tiers](../feature-tiers.md)): *Flattened export at full resolution* and *Export at a chosen px per square, plain image files (#26)*. It also provides the flatten seam ([seams](../agents/seams-phase1b.md)) and the two px-per-square values behind EDT's display size (FR-EDT-04).

## 3. Goals
- The exported patch passes the FR-RGN-32 seam metric on the exported file (MVP step 7, #35, R25).
- Deterministic: the same layer stack, settings and build give the same decoded pixels (OQ-IMG-07).
- A typical map (#30) exports to PNG in ≤ 2 s. A 16k export never blocks the UI and can be canceled.
- An export is never half-written under its final name. It never touches the original, and in the asset project it writes only the `export` record.

## 4. Non-goals
- Metadata sidecars, grid overlays or grid metadata (#26; grid is tier B). VTT presets (KQ-21). Animated WebP and GIF (#25, #27). Alpha (token) export (tier A; ADR 0004 keeps exports RGB8).
- Exporting an older revision without restoring it, batch export, crop on export (tier A). "Show in folder" (a new plugin dependency, §3.5).

## 5. User stories
- As a DM preparing a session, I export my edited map and the kitchen patch blends in with no seam.
- My map is 30×40 cells at 140 px per square and my VTT wants 70, so I enter the grid and target and get a 2100×2800 file.
- I re-export after a tweak, and the file in the project's `exports/` folder is replaced under the same name once I confirm.
- A 16k export takes a while: I see progress and can cancel it, and nothing broken is left behind.

## 6. Functional requirements
| ID | Requirement (testable) |
|---|---|
| FR-EXP-01 | `compose_rect(stack, rect)` returns the RGB8 flatten of the visible layers over `rect` (image px) at source resolution. It is the only compositor: export, RGN extraction (FR-RGN-04) and the goldens all call it. |
| FR-EXP-02 | The stack is PRJ's **current** manifest `layers[]` from `read_project` (R6), drawn in order with tiny-skia, premultiplied sRGB source-over (ADR 0004), over PRJ's `DecodedOriginal` (FR-PRJ-42). Hidden layers are skipped. The output starts as opaque black beneath the original. |
| FR-EXP-03 | A layer with scale 1, rotation 0 and an integer top-left (within 1e-6 px) is drawn with exact pixel sampling. Other layers use bicubic sampling about the centre (ADR 0003 affine). |
| FR-EXP-04 | Exactness: with no patch layers the output equals `layers[0]`'s pixels (the original or a picked base image, alpha over the flatten colour `#808080`; [new-asset-generation](new-asset-generation.md) §8, #60). A project with no image yet: Export is disabled and `export_plan` / `export_start` / `export_set_settings` fail with `no_image` (FR-GEN-03). For integer-placed layers (FR-EXP-03) the output is independent of strip height, and `compose_rect(r)` equals the same rect of the full flatten bit for bit. Pixels where every patch has alpha 0 equal `layers[0]`. |
| FR-EXP-05 | Strip height is 512 rows and is pinned, together with `COMPOSE_VERSION` (a constant bumped on any renderer change). Each export records both. |
| FR-EXP-06 | Grid input: either **source px per square** (real, 8–1024) or **grid size in cells** (`cols` 1–1000, optional `rows`), giving source = width ÷ cols. If rows are given and width ÷ cols differs from height ÷ rows by > 1 %, the plan warns "Grid isn't square" without blocking. |
| FR-EXP-07 | **Target px per square** is an integer, 8–1024, enabled only once a source value exists. Scale = target ÷ source. With no target, scale is 1 ("Full resolution"). |
| FR-EXP-08 | Output size = round(side × scale) per side. Each side must be 1–16384 and scale ≤ 4 (OQ-EXP-05). Scale > 1 adds the warning "Upscaled, softer". A violation returns `invalid_settings{field, limit}`. |
| FR-EXP-09 | At scale 1, PNG and JPEG are flattened in strips and streamed to the encoder (ADR 0004). WebP, or any scale ≠ 1, flattens into one RGB8 buffer first. Rescaling uses `fast_image_resize` Lanczos3 in sRGB (OQ-IMG-13). |
| FR-EXP-10 | Formats: **PNG** `Compression::Fast` (OQ-IMG-03) · **JPEG** q90 · **WebP** lossy q90 via libwebp (OQ-EXP-06). No metadata: PNG has only IHDR/IDAT/IEND, JPEG only JFIF APP0, WebP no EXIF/ICCP/XMP chunks. No sidecar (#26). |
| FR-EXP-11 | WebP is offered only when both output sides are ≤ 16383. Otherwise the plan lists it as unavailable with that reason (OQ-IMG-04). |
| FR-EXP-12 | If lossy WebP fails with a partition overflow, the export retries once as lossless WebP. The result reports `fallback: lossless` and the UI shows a notice (OQ-IMG-04). |
| FR-EXP-13 | (#44) Destination: the project folder's `exports/`, created if missing. Exports of different projects never collide. |
| FR-EXP-14 | (#44) File name: `<project name>_<target>px.<ext>` with a target, else `<project name>.<ext>`; extensions `.png`, `.jpg`, `.webp`. The project name is the folder name (FR-PRJ-44), so it is already file-safe. A renamed project exports under its new name; older exports keep theirs. |
| FR-EXP-15 | If the destination exists, `export_start` fails with `exists{bytes, mtime}` unless `replace: true` (OQ-EXP-03). Nothing else is ever overwritten or deleted, except this spec's own stale temp files (FR-EXP-17). Other files in `exports/` (the user's own, sync conflict copies) are left alone (FR-PRJ-47). |
| FR-EXP-16 | The write goes to `.<name>.pf-tmp-<ulid>` in the destination folder, is flushed, then renamed over the final name. A cancel, failure or crash never leaves a partial file under the final name, and an existing file stays intact until the rename. |
| FR-EXP-17 | Before writing, temp files matching `.*.pf-tmp-*` older than 1 h in the destination folder are deleted. PRJ's open sweep of `exports/` uses this same rule, so it never deletes a running export's temp (FR-PRJ-20, R70). |
| FR-EXP-18 | A locked destination (sharing violation) is retried with backoff for up to 3 s, then fails with `locked`; the temp file is removed. |
| FR-EXP-19 | An export runs in the background and emits `export:progress` at least every 500 ms with a phase (`compose`, `resize`, `encode`, `write`) and a fraction. WebP progress comes from libwebp's progress hook. |
| FR-EXP-20 | `export_cancel` stops the export within 1 s (3 s for a 16k resize), removes the temp file and emits `export:canceled`. Strips check for cancel between strips; libwebp aborts through its progress hook. |
| FR-EXP-21 | One export per asset project at a time; a second start returns `busy`. The export uses the layer stack snapshot taken at start, and editing continues meanwhile (OQ-EXP-09). |
| FR-EXP-22 | A missing or undecodable layer file fails the export with `layer_unreadable{layer_id}`. No partial or layer-skipping output is ever written. |
| FR-EXP-23 | Big buffers are reserved fallibly (`try_reserve`). An allocation failure returns `out_of_memory` and the app keeps running. |
| FR-EXP-24 | `export_plan` is pure (it only stats the destination and reads manifest sizes). It returns output size, scale, formats with reasons, destination path, `exists`, estimated peak memory and warnings. A peak estimate above 2 GiB adds a warning. |
| FR-EXP-25 | Settings (grid input, target, format) are session state of the open project. PRJ's debounced draft save (≥ 2 s after the last change, and on close, FR-PRJ-19) commits them to the manifest `export` record (R30), and `export_start` commits them through PRJ `put_export` (R6). A keystroke never commits by itself. Opening a project never commits either: loading the saved settings only plans and sends nothing back (`export_set_settings` is called only for the user's edits, R101). Settings, and so display size, are field-identical on reopen with or without an export; after an export, `last` is too (OQ-EXP-04). |
| FR-EXP-26 | `export_get_settings` returns `source_px_per_square` and `target_px_per_square` (each or null); every settings change emits `export:settings-changed`. EDT's display size = target ÷ source and needs **both** values (R6, FR-EDT-04). |
| FR-EXP-27 | On success, after the rename, a second `put_export` commits `last = {at, revision_id: head_revision, layers_hash, path, format, width, height, bytes, fallback, strip_h, compose_version}`. `layers_hash` is computed over the stack snapshot (manifest-schema §4.10); `path` is relative to the project folder (R6, FR-PRJ-12). |
| FR-EXP-28 | Until PRJ's `DecodedOriginal` is ready, `export_start` returns `original_unavailable` (R11). Opening the panel calls `project_prefetch_original`; Start is enabled once `project:original-ready` has fired. A failed prefetch is asked again on the next expand and when Start gets `original_unavailable`. A failed background decode fires `project:original-failed{project_id, code}` (FR-PRJ-42): the panel shows the readable error (`corrupt`, `io`, `not_found`) instead of "Loading original…" (R104). |
| FR-EXP-29 | In a **read-only** project (FR-PRJ-41) the panel and `export_plan` work, but `export_start` fails fast with `read_only` and writes nothing (R12). |
| FR-EXP-30 | (#63) **Export as, canvas.** EDT's canvas menu offers **Export as ▸ PNG / JPEG / WebP**. It calls `export_start` with the panel's current grid and target and `format_override`. The session settings and the manifest's `grid`, `target_px_per_square` and `format` stay unchanged, so the start commits no settings. Destination and name follow FR-EXP-13, 14 with the picked format's extension; `last` is committed after the rename as in FR-EXP-27. A format that isn't available (FR-EXP-11) is disabled with its reason. With no image (`no_image`) or on a read-only project (`read_only`) the submenu is disabled with the reason. Opening the menu calls `project_prefetch_original`; a start before `project:original-ready` waits in the toast ("Loading original…") and then starts, and a failed decode shows FR-EXP-28's error. Progress, Cancel and the result are shown in a toast. `exists` shows a toast "Replace <name> (12.3 MB, 2 days ago)?" with Replace and Cancel, and nothing is written until Replace. |
| FR-EXP-31 | (#63) **Save as, canvas.** **Save as…** opens the native save dialog: filters PNG, JPEG and WebP; default name per FR-EXP-14 with the panel's format; default folder = the last Save as folder (app data `settings.json`, never a project file), else the project's `exports/`. The extension picks the format (`.png`, `.jpg`/`.jpeg`, `.webp`, any case). Any other extension fails with `invalid_settings{field: dest}` and writes nothing. `export_start` with `dest` runs the same pipeline at the panel's grid and target. The OS dialog already asked about overwriting, so `replace` is implied. The write is temp → rename in the chosen folder (FR-EXP-16, 18) and only its own temp is ever removed there: FR-EXP-17's sweep never runs outside `exports/`. Writes nothing to the project: no settings commit, no `last`, so it works on a read-only project. A missing or unwritable folder → `dest_unavailable`. Cancel in the dialog does nothing. |
| FR-EXP-32 | (#63) **Single file.** `export_file{source, format, dest}` writes one image file: a candidate, a file in a project folder (FR-PRJ-12) or a pack image. `source` must be inside a library root (`outside_library`). `dest` = `exports` (that project's `exports/`; not offered for pack images) or an absolute path from the save dialog (as FR-EXP-31). The same format as the source's sniffed type → a byte copy. Otherwise the file is decoded (FR-PRJ-06 checks) and encoded with FR-EXP-10's encoders at its own size, with no resample. PNG and WebP keep alpha; JPEG flattens over `#808080` (#60). Names in `exports/`: `results/NNN <slug>/NN.<ext>` → `<project>_<NNN>-<NN>.<ext>`, any other file → `<project>_<file stem>.<ext>`; Save as defaults to the same name (a pack image to its stem). `exists{bytes, mtime}` unless `replace` (toast as FR-EXP-30). Errors: `read_only` for `exports` on a read-only project (Save as is allowed), `not_available_offline` for a cloud-only source offline, `unsupported`/`corrupt` for an unreadable source. Nothing is committed to the manifest. |
| FR-EXP-33 | (#63) Menu exports and Save as share the export runner: one at a time per project (`busy`); a pack image's Save as runs one at a time app-wide. They emit the same `export:*` events with their `export_id`, can be canceled (FR-EXP-20) and count as a running export for the close, quit, rename and delete questions (§7, FR-PRJ-56, 57). |

## 7. UI/UX notes
- **Export section** of the right panel, under Generate, collapsible ([ui-guidelines](../architecture/ui-guidelines.md); `Ctrl+E` expands and focuses it): grid mode toggle (`exp-panel-grid-mode`: px per square / cells), `exp-panel-grid-source` or `exp-panel-grid-cols` + `exp-panel-grid-rows`, `exp-panel-target-ppsq`, format radios (`exp-panel-format-{png,jpeg,webp}`; an unavailable one is disabled with its reason as a tooltip), and a plan line (`exp-panel-plan`): "4200×5600 → 2100×2800 · 0.5× · `exports/Crypt_70px.png`". Warnings sit under it.
- **Exists:** the button (`exp-panel-start`) reads **Replace**, with "Replaces Crypt_70px.png (12.3 MB, 2 days ago)". No modal.
- **Loading:** Start disabled, "Loading original…" (FR-EXP-28). **Read-only:** Start disabled with the reason. **Running:** a progress bar with the phase and Cancel (`exp-panel-cancel`); the canvas stays usable.
- **Done:** a toast with path, size and copy-path (`exp-toast-copy`). The WebP lossless fallback and an unsaved `last` record (§11) add a notice.
- **Context menus (#63):** EDT's canvas menu (`edt-canvas-menu`) and the menus on candidates, project files and pack images (PRJ, RGN, REF) use one shared component from `app/src/shared/`: **Save as…** (`<menu>-save-as`), **Export as ▸** PNG / JPEG / WebP (`<menu>-export-{png,jpeg,webp}`) and **Open in Explorer** (`<menu>-reveal`, FR-PRJ-59). A menu export shows a toast (`exp-toast`) with progress, Cancel (`exp-toast-cancel`), Replace (`exp-toast-replace`) on `exists`, and on success the path with copy-path. The Export panel shows nothing for a menu export, except Start reading busy while one runs.
- **Error:** the code's message plus Retry (`exp-panel-retry`). `locked`: "The file is in use (VTT or sync?)". **Empty** (no project): panel disabled. **Offline / provider unavailable:** no effect (no network calls). **Closing during an export:** the inline bar asks "Cancel the export and close?" (OQ-EXP-09). Quitting the app asks the same (R101). If the export ends (done, failed or canceled) while the question is open, the question goes away and the close goes on.

## 8. Data and file effects
Field shapes live in [manifest-schema §4.10](../architecture/manifest-schema.md) (PRJ owns the schema, R26). EXP needs:

| Data | Where | Written by | Recoverable? |
|---|---|---|---|
| Export file | `<project folder>/exports/<name>` (synced, #13, #44) | `export_start`, `export_file`, temp → rename | Derived. Reproducible when the stack at `last.revision_id` hashes to `last.layers_hash`, with the same settings and `compose_version`. A replaced export isn't kept. |
| Temp file | same folder, `.<name>.pf-tmp-<ulid>` | `export_start` | Deleted on success, cancel or failure; stale ones by FR-EXP-17 |
| Manifest `export` (`grid`, `target_px_per_square`, `format`, `last`) | asset project, via PRJ | settings by the debounced draft save (R30) and at `export_start` (`put_export`); `last` after the rename | Yes, as manifest state |
| Settings typed since the last draft save | memory (session) | `export_set_settings` | Only the last < 2 s of typing is lost in a crash (R30) |
| Save as file (#63) | the folder the user picked | `export_start` / `export_file` with `dest`, temp → rename | The user's file; the app never touches it again |
| Last Save as folder | app data `settings.json` | after a Save as succeeds | Yes; missing → the project's `exports/` |
| Flatten buffers, plan | memory | — | Transient |

- Store calls (PRJ store API, R4): `read_project` (current `layers[]`, `head_revision`, original size), `decoded_original` and `put_export`; the settings also ride PRJ's draft save (R30). EXP never uses the pending-write spool. `last.layers_hash` equal to the current hash ⇒ the export shows exactly the current stack.

## 9. Commands and events
| Command | Input → result | Errors |
|---|---|---|
| `export_get_settings` | — → settings + `source_px_per_square`, `target_px_per_square` | `no_project` |
| `export_set_settings` | settings → settings (session state; committed by the next draft save, R30) | `no_project`, `invalid_settings{field, limit}` |
| `export_plan` | settings → `{width, height, scale, formats[{format, available, reason}], path, exists{bytes, mtime}?, est_peak_mib, warnings[]}` | `invalid_settings`, `dest_unavailable` (root offline or not writable) |
| `export_start` | settings, `replace`, `format_override?`, `dest?` (absolute path, FR-EXP-30, 31) → `export_id` | the above + `exists`, `busy`, `original_unavailable`, `too_large{format, limit}`, PRJ's `read_only` / `locked` / `conflict` from the settings commit (R12; nothing is written) |
| `export_file` | `{source, format, dest: exports \| path, replace}` → `export_id` (FR-EXP-32) | `outside_library`, `not_found`, `unsupported`, `corrupt`, `not_available_offline`, `read_only`, `exists`, `busy`, `invalid_settings{field: dest}`, `dest_unavailable` |
| `export_cancel` | `export_id` → ok | `unknown_id` (already finished) |

- **Events** (R21): `export:progress{export_id, phase, fraction}` · `export:done{export_id, path, width, height, bytes, format, fallback, record_saved}` · `export:failed{export_id, code, detail}` (`layer_unreadable`, `locked`, `disk_full`, `out_of_memory`, `encode_failed`) · `export:canceled{export_id}` · `export:settings-changed{source, target}`.
- **Library API** (`pf-imaging`): `compose_rect(stack, rect) -> Rgb8`, `flatten_strips(stack, strip_h, sink)`, `encode_{png,jpeg}_stream`, `encode_webp(buf, progress, cancel)`, `resize_lanczos3`. `pf-core` maps the manifest to a `LayerStack`.

## 10. Provider interaction
N/A. Export makes no provider calls and has no cost.

## 11. Edge cases
| Case | Behaviour |
|---|---|
| Crash mid-save | Only a temp file can remain (FR-EXP-16), swept by FR-EXP-17. `last` is committed after the rename, so it never points to an unwritten file. A crash between rename and commit leaves a valid file and the previous `last`. |
| Sync conflict / file locked | Locked destination: FR-EXP-18. The settings commit at start fails with PRJ's `conflict`/`locked` and nothing is written. If the `last` commit fails, the file stands, `export:done` has `record_saved: false`, and PRJ's retry applies (FR-PRJ-21). A sync conflict copy of an export is a plain file EXP ignores. |
| Offline | No effect. Library root unavailable: `dest_unavailable`. Original cloud-only: `original_unavailable` until PRJ hydrates it. |
| Provider error or timeout | N/A. |
| Very large image | 16k PNG/JPEG stream (≤ 400 MiB extra). WebP > 16383 px is unavailable (FR-EXP-11). Rescale at 16k needs up to 1.6 GiB extra; the plan warns above 2 GiB (FR-EXP-24). Disk full: `disk_full`, temp removed. |
| Corrupt or unsupported input | `layer_unreadable` (FR-EXP-22). The original is guarded by PRJ (FR-PRJ-15); a changed original makes the project read-only (FR-EXP-29). |
| Undo across the feature boundary | Exporting isn't on the undo stack. Undo of accept or re-tune is `project_restore_revision(parent)` (R3), so the current stack — and the next export — no longer holds that patch. |

## 12. Performance budget
Release build on the owner's desktop. Typical = 4200×5600 (#30). Memory is extra beyond PRJ's `DecodedOriginal` (≤ 70 MiB typical, 768 MiB at 16k, [asset-project §12](asset-project.md#12-performance-budget)).

| Operation | Typical | 16k |
|---|---|---|
| `export_plan` | ≤ 50 ms | ≤ 50 ms |
| PNG / JPEG, scale 1 (measured 1.2 s typical; 13.2 / 16.6 s at 16k) | ≤ 2 s | ≤ 20 s |
| Rescale to 0.5× + PNG | ≤ 3 s | ≤ 25 s |
| WebP lossy (extrapolated from 130 s at 10240², OQ-EXP-07) | ≤ 35 s | n/a (> 16383) |
| Extra memory: strips / full buffer + rescale / WebP | ≤ 60 / 140 / 250 MiB | ≤ 400 MiB / 1.6 GiB / n/a |
| Whole-app line (R11) | owned by PRJ ([asset-project §12](asset-project.md#12-performance-budget)) | EXP adds ≤ 400 MiB (stream) or ≤ 1.6 GiB (rescale) |
| Progress interval · cancel acknowledgement | ≤ 500 ms · ≤ 1 s | ≤ 500 ms · ≤ 3 s |

## 13. Test plan
- **Unit (Rust, `pf-imaging`, `fr_exp_NN_*`):** size and scale maths and limits (06–08, 11); destination and naming from the project name (13, 14); integer-placement detection (03); PNG/JPEG/WebP chunk lists hold no metadata (10); settings validation table.
- **Golden (Rust, OQ-IMG-07):** decoded pixels exact on Windows, ±1 elsewhere; strip height 512 and `COMPOSE_VERSION` pinned. Cases: (a) no layers = original; (b) 3 baked patches from **fake provider** candidates at fixed seeds; (c) one affine layer (scale 1.3, rotation 15°, opacity 0.8); (d) Lanczos3 140 → 70 px per square. Lossless PNGs at 1024×1280 (< 1 MB each), regenerated only with `PF_UPDATE_GOLDENS=1` and reviewed in the diff.
- **Property (Rust):** `compose_rect(r)` = crop of the flatten for 200 random rects (04). Strip heights 512 / 256 / full are equal for integer layers. Alpha-0 pixels equal the original. JPEG and WebP decode within PSNR ≥ 40 dB of the PNG export. The bar is for the smooth generated test image: a noisy map at 0.5× scores ~34 dB with WebP q90 (48.6 dB at full size), and q90 stays (R59).
- **Integration (Rust, `pf-core`):** PRJ's fault seam fails before and after the rename → no partial final file, old file intact (16). Locked k times then free (18). Encoder seam returns partition overflow → lossless (12). Cancel mid-strip and mid-WebP (20). `busy`, `exists`/`replace`, `layer_unreadable`, `out_of_memory` (allocator seam). `export_set_settings` leaves `commit.seq` unchanged until the draft save (fake clock, ≥ 2 s) commits once; `export_start` commits; reopen without an export is field-identical (25, R30). `last.layers_hash` equals the schema hash of the snapshot and `last.path` is project-relative (27). Before `DecodedOriginal` is ready → `original_unavailable` (28). Read-only → `read_only`, nothing written (29). Accept, then restore the parent (R3), then export = golden (a). Seam fixture with fake `offset(12)` accepted → FR-RGN-32 metric passes on the exported PNG (R25). A 16384² generated export streams within 400 MiB extra peak (§12).
- **Thin e2e (Playwright, mocked core, KQ-33):** panel modes → plan line; WebP disabled with its reason; Replace flow; loading and read-only states; progress and cancel; done toast; errors; `export:settings-changed` enables EDT's `Ctrl+2` only with both values.
- **#63 (Rust integration + thin e2e):** `format_override` leaves `commit.seq` and settings unchanged and writes `last` (30). `dest` outside the library: temp → rename, no `last`, no sweep of other `.*.pf-tmp-*` files there, works on read-only (31). `export_file` byte copy vs convert, alpha kept in PNG and over `#808080` in JPEG, candidate and file names, `exists`/`replace`, `outside_library` (32). A second start → `busy`; a running Save as triggers the close question (33). e2e: each menu's items and disabled reasons, toast progress, Replace, cancel; the save dialog through the mocked `api` seam.
- **Tauri smoke (Windows):** export the generated typical map to PNG in `.pf-dev/`, then decode it and check its size.
- **Fixtures (generated, KQ-22):** EDT's `fixtures/gen` battlemap (2048×2560; 4200×5600 = 30×40 cells at 140 px; 16384² on demand), RGN's seam fixture, synthetic RGBA patches (feathered disc), fake-provider candidates (PRV). No personal art.
- **App walkthrough:** [Appendix A](#appendix-a-app-walkthrough).

## 14. Build split
| # | Slice | Shared files (AGENTS §9) | Depends on |
|---|---|---|---|
| 1 | Compositor: `LayerStack`, `compose_rect`, `flatten_strips`, integer path, `COMPOSE_VERSION`, goldens (a)(c) | none (new `pf-imaging` module); fixtures | skeleton. **Unblocks RGN slice 1** |
| 2 | Encoders and rescale: PNG/JPEG streaming, libwebp lossy + lossless fallback + progress/cancel hook, Lanczos3, limits; golden (d) | root `Cargo.toml` (orchestrator adds the #32 crates) | 1 |
| 3 | Core commands: session settings, plan, start/cancel, events, destination, naming, atomic write, `put_export`, read-only and `original_unavailable`; golden (b) | `commands.rs` (append), manifest schema (`export`, add only) | 2; PRJ slices 3–4 (`DecodedOriginal`, store API, fault seam); PRV fake provider for (b) |
| 4 | Export panel UI + `export:settings-changed` → EDT display size | UI feature registry (append) | 3 (or the mock); EDT slice 2 |
| 5 | Tauri smoke + walkthrough | none | 4 |
| 6 | (#63) `format_override`, `dest`, `export_file`, runner sharing; shared context menu, toast, save dialog seam (with PRJ slice 9) | `commands.rs` (append), capability `dialog:allow-save` (orchestrator) | 5 |

## 15. Open questions
None open.

**Resolved:** OQ-EXP-01 ✅ superseded by #44: the project's `exports/` · OQ-EXP-02 ✅ A, `<name>_<target>px` (#40), with the project name instead of the original's stem (#44) · OQ-EXP-03 ✅ A, inline Replace (#40) · OQ-EXP-04 ✅ A, per asset project, manifest `export` (#40, R6) · OQ-EXP-05 ✅ A, ≤ 4× (#40) · OQ-EXP-06 ✅ A, q90 (#40) · OQ-EXP-07 ✅ A, measure in slice 2 (#40) · OQ-EXP-08 ✅ A, EXP supplies reference and tolerance, EDT reads back (#40) · OQ-EXP-09 ✅ A, snapshot at start (#40) · OQ-EXP-10 ✅ Px per square in the glossary (R27) · OQ-EXP-11 ✅ B, settings persist through PRJ's debounced draft save (R30).

**Answers to other specs:** OQ-EDT-01 ✅ both values required (R6). OQ-RGN-02 ✅ RGN calls `compose_rect` on the outer rect of the current stack (FR-RGN-04), delivered in slice 1.

## Appendix A: App walkthrough
Test agent; `PF_DATA_DIR` and `PF_LIBRARY_DIR` in `.pf-dev/`; fake provider.

| # | Step | Expected |
|---|---|---|
| 1 | Create a Maps project `Crypt` from the generated 4200×5600 map, open it, then `Ctrl+E` | Plan "4200×5600 · Full resolution"; target disabled; EDT's display size disabled |
| 2 | Export PNG | `Maps/Crypt/exports/Crypt.png` exists and decodes pixel-equal to the original |
| 3 | Mask, fake job, accept a patch; export PNG again | "Replaces …" shown; after Replace the file differs from the original only inside the patch rect |
| 4 | Undo the accept (`Ctrl+Z`); export PNG with Replace | The file decodes pixel-equal to the original again (R3) |
| 5 | Open RGN's seam fixture; mask, fake job `offset(12)`, accept; export PNG | FR-RGN-32 metric on the exported file: max step of the 3-px smoothed column-mean profile ≤ 0.5 levels/px on both mask edges, residual ≤ 0.1 (R25, R68) |
| 6 | Back on the map: cells 30×40, target 70 | Plan 2100×2800, name `Crypt_70px.png`; `Ctrl+2` in EDT now shows 50 % |
| 7 | Choose WebP, export | Progress moves; the file decodes at 2100×2800 |
| 8 | Open the 16384² map; choose WebP | WebP disabled: "WebP max 16383 px" |
| 9 | Export it as JPEG and cancel at ~30 % | No final file, no temp file; any previous export intact |
| 10 | Hold the target file with an exclusive lock (PowerShell `[IO.File]::Open`) and export | After about 3 s, "The file is in use"; no temp left |
| 11 | Close and reopen the first map | Cells 30×40, target 70, WebP and the last export restored |
| 12 | Close; overwrite the original with a different generated image; reopen | Read-only banner; plan works; Start disabled, and `export_start` returns `read_only` |
| 13 | (#63) Create `Vault` from the generated 2048×2560 map; cells 16×20, target 64, PNG; right-click the canvas → Export as ▸ JPEG | `exports/Vault_64px.jpg` at 1024×1280; the panel still shows PNG; the toast names the file |
| 14 | Right-click the canvas → Save as…, pick `.pf-dev/out/vault.webp`; then the same on the read-only first map | Both files decode (1024×1280 for Vault); neither project's `last` nor `commit.seq` changes |
| 15 | In Vault, mask, fake job (3 candidates); right-click candidate `01` of `001` → Export as ▸ WebP, twice | `exports/Vault_001-01.webp` written; the second time the toast offers Replace |

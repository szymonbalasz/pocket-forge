---
feature: export
prefix: EXP
owner: spec-export
status: in-review   # unclaimed → claimed → draft → in-review → complete (owner only)
depends_on: [asset-project]
---

# Flattened export

## 1. Summary
Export turns an asset project's **original** plus its visible **layers** into one plain image file for the owner's own VTT (#24, #26), which is MVP scenario step 7. Rust is the only compositor (KQ-25). It flattens at source resolution in 512-row strips (ADR 0004), optionally resamples to a user-chosen px per square, and writes PNG, JPEG or WebP atomically into the library. The same compose function feeds region extraction (RGN) and the golden-image tests.

## 2. Scope
Tier S ([feature-tiers](../feature-tiers.md)): *Flattened export at full resolution* and *Export at a chosen px per square, plain image files (#26)*. It also provides the flatten seam ([seams](../agents/seams-phase1b.md)) and the px-per-square values behind EDT's display size (OQ-EDT-01).

## 3. Goals
- The exported patch shows no visible seam at the original resolution (MVP step 7, #35).
- Output is deterministic: the same project, settings and build give the same decoded pixels (OQ-IMG-07).
- A typical map (#30) exports to PNG in ≤ 2 s. A 16k export never blocks the UI and can be cancelled.
- An export is never half-written under its final name, and it never touches the original or the asset project's files.

## 4. Non-goals
- Metadata sidecars, grid overlays or grid metadata (#26; grid is tier B). VTT presets (KQ-21).
- Animated WebP and GIF (#25, #27, tier A/B). Alpha (token) export (tier A; ADR 0004 keeps exports RGB8).
- Exporting an older revision without restoring it, batch export of many projects, or a crop on export (tier A).
- Opening the OS file manager ("show in folder"): it needs a new plugin dependency (§3.5).

## 5. User stories
- As a DM preparing a session, I export my edited map and the kitchen patch blends in with no seam.
- My map is 30×40 cells at 140 px per square, and my VTT wants 70, so I type the grid and the target and get a 2100×2800 file.
- I re-export after a tweak and the file in my VTT folder is replaced under the same name, after I confirm.
- A 16k export takes a while: I see progress and can cancel it, and nothing broken is left behind.

## 6. Functional requirements
| ID | Requirement (testable) |
|---|---|
| FR-EXP-01 | `compose_rect(stack, rect)` returns the RGB8 composite of the visible layers over `rect` (image px) at source resolution. It is the only compositor: export, RGN extraction (OQ-RGN-02) and the goldens all call it. |
| FR-EXP-02 | Layers are drawn in manifest order with tiny-skia, premultiplied sRGB source-over (ADR 0004). Hidden layers are skipped. The output starts as opaque black, beneath the original. |
| FR-EXP-03 | A layer with scale 1, rotation 0 and an integer top-left (within 1e-6 px) is drawn with exact pixel sampling. Other layers use bicubic sampling about the centre (ADR 0003 affine). |
| FR-EXP-04 | Exactness: with no patch layers the output equals the original's pixels. For integer-placed layers (FR-EXP-03) the output is independent of strip height, and `compose_rect(r)` equals the same rect of the full flatten, bit for bit. Pixels where every patch has alpha 0 equal the original. |
| FR-EXP-05 | Strip height is 512 rows and is pinned, together with `COMPOSE_VERSION` (a constant bumped on any renderer change). Each export records both. |
| FR-EXP-06 | Grid input, per asset project: either **source px per square** (real, 8–1024) or **grid size in cells** (`cols` 1–1000, optional `rows`), giving source = width ÷ cols. If rows are given and width ÷ cols differs from height ÷ rows by more than 1 %, the plan warns "Grid isn't square" without blocking. |
| FR-EXP-07 | **Target px per square** is an integer, 8–1024, enabled only once a source value exists. Scale = target ÷ source. With no source value, scale is 1 ("Full resolution"). |
| FR-EXP-08 | Output size = round(side × scale) per side. Each side must be ≥ 1 and ≤ 16384, and scale ≤ 4 (OQ-EXP-05). A violation returns `invalid_settings{field, limit}`. |
| FR-EXP-09 | At scale 1, PNG and JPEG are composed in strips and streamed to the encoder (ADR 0004). WebP, or any scale ≠ 1, flattens into one RGB8 buffer first. Rescaling uses `fast_image_resize` Lanczos3 in sRGB (OQ-IMG-13). |
| FR-EXP-10 | Formats: **PNG** `Compression::Fast` (OQ-IMG-03) · **JPEG** q90 · **WebP** lossy q90 via libwebp (OQ-EXP-06). Files hold no metadata: PNG has only IHDR/IDAT/IEND, JPEG only JFIF APP0, WebP has no EXIF/ICCP/XMP chunks. There is no sidecar (#26). |
| FR-EXP-11 | WebP is offered only when both output sides are ≤ 16383. Otherwise the plan lists it as unavailable with that reason (OQ-IMG-04). |
| FR-EXP-12 | If lossy WebP fails with a partition overflow, the export retries once as lossless WebP. The result reports `fallback: lossless` and the UI shows a notice (OQ-IMG-04). |
| FR-EXP-13 | Destination: `<library-root>/Exports/<original's folder relative to its root>/` on the original's root (OQ-EXP-01). Missing folders are created. |
| FR-EXP-14 | File name: `<original stem>_<target>px.<ext>` when a target is set, else `<original stem>.<ext>`. The extensions are `.png`, `.jpg` and `.webp` (OQ-EXP-02). |
| FR-EXP-15 | If the destination file exists, `export_start` fails with `exists{bytes, mtime}` unless `replace: true` (OQ-EXP-03). Nothing else is ever overwritten or deleted, except this spec's own stale temp files (FR-EXP-17). |
| FR-EXP-16 | The write goes to `.<name>.pf-tmp-<ulid>` in the destination folder, is flushed, then renamed over the final name. A cancel, failure or crash never leaves a partial file under the final name, and an existing file stays intact until the rename. |
| FR-EXP-17 | Before writing, temp files matching `.*.pf-tmp-*` older than 1 h in the destination folder are deleted. |
| FR-EXP-18 | A locked destination (sharing violation) is retried with backoff for up to 3 s, then fails with `locked`. The temp file is removed. |
| FR-EXP-19 | An export runs in the background and emits `export:progress` at least every 500 ms, with a phase (`compose`, `resize`, `encode`, `write`) and a fraction. WebP progress comes from libwebp's progress hook. |
| FR-EXP-20 | `export_cancel` stops the export within 1 s (3 s for a 16k resize), removes the temp file and emits `export:cancelled`. Strips check for cancel between strips, and libwebp aborts through its progress hook. |
| FR-EXP-21 | One export per asset project at a time; a second start returns `busy`. The export uses the layer stack snapshot taken at start, and editing continues meanwhile (OQ-EXP-09). |
| FR-EXP-22 | A missing or undecodable layer file fails the export with `layer_unreadable{layer_id}`. No partial or layer-skipping output is ever written. |
| FR-EXP-23 | Big buffers are reserved fallibly (`try_reserve`). An allocation failure returns `out_of_memory` and the app keeps running. |
| FR-EXP-24 | `export_plan` is pure (it only stats the destination). It returns the output size, scale, available formats with reasons, the destination path, `exists`, the estimated peak memory, and warnings. A peak estimate above 2 GiB adds a warning. |
| FR-EXP-25 | Export settings (grid input, target, format) and the last export record are stored per asset project (OQ-EXP-04) and are field-identical after reopen. In a read-only project they apply for the session only, and export still works. |
| FR-EXP-26 | `export_get_settings` returns `source_px_per_square` and `target_px_per_square` (or null), and every change emits `export:settings-changed`. EDT's display size = target ÷ source (OQ-EDT-01). |
| FR-EXP-27 | Export never writes the original or any file in the asset project folder except the manifest's export record (one commit through the PRJ store, after the rename). |

## 7. UI/UX notes
- **Export panel** (`Ctrl+E`, `exp-panel-*`): grid mode toggle (`exp-grid-mode`: px per square / cells), `exp-grid-source` or `exp-grid-cols` + `exp-grid-rows`, `exp-target-ppsq`, format radios (`exp-format-{png,jpeg,webp}`; an unavailable one is disabled with its reason as a tooltip), and a plan line: "4200×5600 → 2100×2800 · 0.5× · `Exports/dungeons/crypt_70px.png`". Warnings sit under it.
- **Exists:** the button turns into **Replace** (`exp-start`), with "Replaces crypt_70px.png (12.3 MB, 2 days ago)". There's no modal.
- **Running:** a progress bar with phase and Cancel (`exp-cancel`). The canvas stays usable.
- **Done:** a toast with the path, size and a copy-path button (`exp-done-copy`). The WebP lossless fallback adds a notice.
- **Error:** the code's message plus Retry. `locked`: "The file is in use (VTT or sync?)". **Empty** (no project): the panel is disabled. **Offline / provider unavailable:** no effect, because export makes no network calls.
- **Closing during an export:** the inline bar asks "Cancel the export and close?" (OQ-EXP-09).

## 8. Data and file effects
| Data | Where | Written by | Recoverable? |
|---|---|---|---|
| Export file | `<library-root>/Exports/<rel dir>/<name>` (synced, #13) | `export_start`, temp → rename | Derived. Reproducible from `last.revision_id` + settings + `COMPOSE_VERSION`. A replaced export isn't kept. |
| Temp file | same folder, `.<name>.pf-tmp-<ulid>` | `export_start` | Deleted on success, cancel or failure; stale ones by FR-EXP-17 |
| `export` record | manifest, field added through PRJ (OQ-EXP-04) | `export_set_settings`, `export_start` on success | Yes, as manifest state |
| Flatten buffers, plan | memory | — | Transient |

- `export` = `grid{mode: px\|cells, source_px_per_square?, cols?, rows?}`, `target_px_per_square?`, `format: png\|jpeg\|webp`, `last{at, revision_id, path (relative to the library root), format, width, height, bytes, fallback, strip_h, compose_version}`.
- The layer stack is read through PRJ's `read_project` (the head revision's layers). The decoded original is the in-memory copy ADR 0004 already holds.

## 9. Commands and events
| Command | Input → result | Validation / errors |
|---|---|---|
| `export_get_settings` | — → settings + `source_px_per_square`, `target_px_per_square` | `no_project` |
| `export_set_settings` | settings → settings | `invalid_settings{field, limit}`, `read_only` (applied for the session), `locked`, `conflict` (PRJ) |
| `export_plan` | settings → `{width, height, scale, formats[{format, available, reason}], path, exists{bytes, mtime}?, est_peak_mib, warnings[]}` | `invalid_settings`, `dest_unavailable` (root offline or not writable) |
| `export_start` | settings, `replace` → `export_id` | the above + `exists`, `busy`, `original_unavailable`, `too_large{format, limit}` |
| `export_cancel` | `export_id` → ok | `unknown_id` (already finished) |

- **Events:** `export:progress{export_id, phase, fraction}`, `export:done{export_id, path, width, height, bytes, format, fallback}`, `export:failed{export_id, code, detail}` (`layer_unreadable`, `locked`, `disk_full`, `out_of_memory`, `encode_failed`), `export:cancelled{export_id}`, `export:settings-changed{source, target}`.
- **Library API** (`pf-imaging`): `compose_rect(stack, rect) -> Rgb8` and `flatten_strips(stack, strip_h, sink)`; `encode_{png,jpeg}_stream`, `encode_webp(buf, progress, cancel)`, `resize_lanczos3`. `pf-core` maps the manifest to a `LayerStack`.

## 10. Provider interaction
N/A. Export makes no provider calls and has no cost.

## 11. Edge cases
| Case | Behaviour |
|---|---|
| Crash mid-save | Only a temp file can remain (FR-EXP-16); it's swept by FR-EXP-17. The manifest record is written after the rename, so it never points to an unwritten file. |
| Sync conflict / file locked | Locked destination: FR-EXP-18. A conflict on the manifest record follows PRJ (FR-PRJ-35). A sync client's conflict copy of an export is a plain file that EXP ignores. |
| Offline | No effect. If the library root is unavailable: `dest_unavailable`. |
| Provider error or timeout | N/A. |
| Very large image | 16k PNG/JPEG streams (≤ 400 MiB extra). WebP > 16383 px is unavailable (FR-EXP-11). Rescale at 16k needs up to 1.6 GiB extra, and the plan warns above 2 GiB (FR-EXP-24). Disk full: `disk_full`, temp removed. |
| Corrupt or unsupported input | `layer_unreadable` (FR-EXP-22). The original is guarded by PRJ (FR-PRJ-15/16): if it's missing, `original_unavailable`. |
| Undo across the feature boundary | Exporting isn't undoable and doesn't touch the undo stack. A patch hidden by undoing an accept (OQ-PRJ-06) is hidden in the export. |

## 12. Performance budget
Release build on the owner's desktop. Typical = 4200×5600 (#30).

| Operation | Typical | 16k |
|---|---|---|
| `export_plan` | ≤ 50 ms | ≤ 50 ms |
| PNG / JPEG, scale 1 (measured 1.2 s typical; 13.2 / 16.6 s at 16k) | ≤ 2 s | ≤ 20 s |
| Rescale to 0.5× + PNG | ≤ 3 s | ≤ 25 s |
| WebP lossy (extrapolated from 130 s at 10240², OQ-EXP-07) | ≤ 35 s | n/a (> 16383) |
| Extra memory: strips / full buffer + rescale / WebP | ≤ 60 / 140 / 250 MiB | ≤ 400 MiB / 1.6 GiB / n/a |
| Progress interval · cancel acknowledgement | ≤ 500 ms · ≤ 1 s | ≤ 500 ms · ≤ 3 s |

## 13. Test plan
- **Unit (Rust, `pf-imaging`, `fr_exp_NN_*`):** size and scale maths and limits (FR-EXP-06–08, 11). Naming and the relative-path mirror (FR-EXP-13/14). Integer-placement detection (FR-EXP-03). The PNG/JPEG/WebP chunk lists hold no metadata (FR-EXP-10). Settings validation table.
- **Golden (Rust, OQ-IMG-07):** decoded pixels compared exactly on Windows and ±1 elsewhere; strip height 512 and `COMPOSE_VERSION` pinned. Cases: (a) no layers = original; (b) 3 baked patches from the **fake provider's** candidates at fixed seeds; (c) one affine layer (scale 1.3, rotation 15°, opacity 0.8); (d) Lanczos3 140 → 70 px per square. Goldens are lossless PNGs at 1024×1280 (< 1 MB each), regenerated only with `PF_UPDATE_GOLDENS=1` and reviewed in the diff.
- **Property (Rust):** `compose_rect(r)` = crop of the flatten for 200 random rects (FR-EXP-04). Strip heights 512 / 256 / full are equal for integer layers. Alpha-0 pixels equal the original. JPEG and WebP decode within PSNR ≥ 40 dB of the PNG export.
- **Integration (Rust, `pf-core`):** the fault-injecting filesystem seam (PRJ) fails before and after the rename → no partial final file, and the old file is intact (FR-EXP-16). Locked k times then free (FR-EXP-18). An encoder seam returns partition overflow → lossless fallback (FR-EXP-12). Cancel mid-strip and mid-WebP (FR-EXP-20). `busy`, `exists`/`replace`, `layer_unreadable`, `out_of_memory` (allocator seam). Settings round-trip through save → reopen (FR-EXP-25). A 16384² generated export streams under 450 MiB peak.
- **Thin e2e (Playwright, mocked core, KQ-33):** panel modes → plan line; WebP disabled with its reason; the Replace flow; progress and cancel; done toast; error states; `export:settings-changed` enables EDT's `Ctrl+2`.
- **Tauri smoke (Windows):** export the generated typical map to PNG in `.pf-dev/`, then decode it and check its size.
- **Fixtures (generated, KQ-22):** EDT's `fixtures/gen` battlemap (2048×2560, 4200×5600 = 30×40 cells at 140 px, 16384² on demand), synthetic RGBA patches (feathered disc), and fake-provider candidates (PRV). No personal art.
- **App walkthrough** (test agent; `PF_DATA_DIR` and `PF_LIBRARY_DIR` in `.pf-dev/`; fake provider):

| # | Step | Expected |
|---|---|---|
| 1 | Open the generated 4200×5600 map, then `Ctrl+E` | Plan "4200×5600 · Full resolution"; target disabled; EDT's Display size is disabled |
| 2 | Export PNG | `Exports/<rel>/<stem>.png` exists and decodes pixel-equal to the original |
| 3 | Mask, fake job, accept a patch; export PNG again | "Replaces …" shown; after Replace the patch is in the file, and at 100 % in an image viewer there's no seam |
| 4 | Set cells 30×40, target 70 | Plan 2100×2800, name `<stem>_70px.png`; `Ctrl+2` in EDT now shows 50 % |
| 5 | Choose WebP, export | Progress moves; the file decodes at 2100×2800 |
| 6 | Open the 16384² map; choose WebP | WebP disabled: "WebP max 16383 px" |
| 7 | Export it as JPEG and cancel at ~30 % | No final file, no temp file; any previous export is intact |
| 8 | Hold the target file with an exclusive lock (PowerShell `[IO.File]::Open`) and export | After about 3 s, "The file is in use"; no temp left |
| 9 | Close and reopen the first map | Cells 30×40, target 70, WebP and the last export restored |

## 14. Build split
| # | Slice | Shared files (AGENTS §9) | Depends on |
|---|---|---|---|
| 1 | Compositor: `LayerStack`, `compose_rect`, `flatten_strips`, integer path, `COMPOSE_VERSION`, goldens (a)(c) | none (new `pf-imaging` module); fixtures | skeleton. **Unblocks RGN slice 1** (OQ-RGN-02) |
| 2 | Encoders and rescale: PNG/JPEG streaming, libwebp lossy + lossless fallback + progress/cancel hook, Lanczos3, limits; golden (d) | root `Cargo.toml` (orchestrator adds the #32 crates) | 1 |
| 3 | Core commands: settings, plan, start/cancel, events, destination, naming, atomic write, export record; golden (b) | `commands.rs` (append), manifest schema (add `export`) | 2; PRJ slice 4 (store API, fault seam); PRV fake provider for (b) |
| 4 | Export panel UI + `export:settings-changed` → EDT display size | UI feature registry (append) | 3 (or the mock); EDT slice 2 |
| 5 | Tauri smoke + walkthrough | none | 4 |

## 15. Open questions
| ID | Question | Options (pick in **bold**) | Blocking? | Affects |
|---|---|---|---|---|
| OQ-EXP-01 | Where do exports go? (#16) | **A. `<library-root>/Exports/` mirroring the original's folder** / B. next to the original / C. any folder via a save dialog — A stays inside the library (#16) and avoids name clashes. B clutters map folders. C can reach the VTT folder directly but needs the owner's OK against #16. | Yes, for slice 3 | owner |
| OQ-EXP-02 | File name | **A. `<stem>_<target>px.<ext>`, or `<stem>.<ext>` at full resolution** / B. editable per export — A is stable for the VTT. B is flexible but adds a field. | No | — |
| OQ-EXP-03 | An existing file at the destination | **A. Replace atomically after an inline "Replace" confirmation** / B. never overwrite: add ` (2)` / C. overwrite silently — A keeps the VTT's path stable. B piles up files in a synced folder. | No | — |
| OQ-EXP-04 | Where do export settings live? | **A. Per asset project, a manifest field `export` ([asset-project](asset-project.md) stores it)** / B. app-wide — A matches different grids per map and feeds EDT's display size. B is one value for every map. | Yes, for slice 3 | asset-project, editor-canvas |
| OQ-EXP-05 | Upscaling | **A. Allow up to 4× and ≤ 16384 px per side, with an "upscaled, softer" warning** / B. downscale only — A serves a 70 px map in a 140 px VTT. B avoids soft output. | No | — |
| OQ-EXP-06 | JPEG/WebP quality | **A. Fixed q90 in tier S** / B. a quality slider — A is one less control. B helps file size. | No | — |
| OQ-EXP-07 | Lossy WebP speed (130 s measured at 10240², about 30 s typical extrapolated) | **A. Keep libwebp's default method; measure in slice 2 and lower `method` if typical > 35 s (integrator ruling)** / B. make WebP lossless by default — A keeps small files. B is fast but about 6× larger. | No | — |
| OQ-EXP-08 | Who owns the canvas vs export parity test (ADR 0003, KQ-25)? | **A. EXP supplies the reference flatten and tolerance; [editor-canvas](editor-canvas.md) reads back the canvas in its Tauri smoke** / B. EXP owns it end to end — A puts each half where its code lives. | No | editor-canvas |
| OQ-EXP-09 | Editing during an export | **A. Allowed; the export uses the snapshot taken at start, and closing asks to cancel** / B. lock the editor — A doesn't block a 16k export. B is simpler. | No | asset-project |
| OQ-EXP-10 | Glossary: add **Px per square** (pixels per grid square; source vs target, #26)? | **A. Add it** / B. UI label only — specs (EDT, EXP) already use it. | No | glossary |

**Answers to other specs:** **OQ-EDT-01:** A is agreed. EXP stores both values per project (FR-EXP-25/26, OQ-EXP-04) and emits `export:settings-changed`. Display size is disabled until a source value exists. **OQ-RGN-02:** A is agreed. RGN calls `compose_rect` on the outer rect (FR-EXP-01). It equals the export's pixels bit for bit and equals a crop of the original when there are no patch layers (FR-EXP-04). It's delivered in slice 1.

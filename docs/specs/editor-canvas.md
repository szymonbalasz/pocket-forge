---
feature: editor-canvas
prefix: EDT
owner: rev-editor-canvas
status: complete   # unclaimed → claimed → draft → in-review → complete (owner only)
depends_on: [asset-project, export]
---

# Editor canvas

## 1. Summary
The editor canvas shows an asset project's **original** and its **layers** at full fidelity, with zoom and pan up to 16k px (#11, #30). There the DM selects a **region**, paints its **mask**, and compares and picks **candidates**.
It implements [ADR 0003](../architecture/adr/0003-canvas-library.md): Konva, one app-owned **view** transform, a **tile pyramid** served over a custom URI scheme, and a region-sized mask canvas. It covers MVP scenario steps 1, 2 and 4 ([vision](../vision.md)). Revised per the [phase 1b rulings](../agents/rulings-phase1b.md).

## 2. Scope
Tier S ([feature-tiers](../feature-tiers.md)): *Zoom, pan, view at intended display size* · *Paint and adjust masks* (brush, erase, hardness; the tier-S "feather" is RGN's blend control, R27) · *Candidate comparison (before/after, pick)*, the UI components only · *Undo within a session* (KQ-16) · the region pointer tool (R18) · layer **display**. Ownership follows the [phase 1b seams](../agents/seams-phase1b.md).

## 3. Goals
- A typical map is usable in < 2 s and a 16k map in < 5 s (#35). Pan, zoom and painting stay smooth on an iGPU.
- The mask the DM sees is exactly the mask that is handed off: byte-identical, and pixel-identical after reopen (#35).
- Mask edits undo one stroke at a time within a bounded memory budget, and every behaviour is testable in Playwright with mocked core commands (KQ-33).

## 4. Non-goals
- Moving, scaling or rotating layers, or editing opacity and visibility (tier A). The Konva `Transformer` isn't used.
- Region geometry rules and fit, composing jobs, accepting candidates, baking the feather (RGN). Flatten and export (EXP). The canvas never composites for output (KQ-25).
- Pressure-sensitive brushes, lasso or shape masks, mask invert, grid overlays (tier A/B).

## 5. User stories
- As a DM preparing a session, I open a battlemap and zoom from the whole map down to single pixels without waiting or blurring, then flip to **display size** to judge the map the way my players will see it in my VTT.
- I drag a region over one room, paint a soft mask, erase an overshoot, and undo a bad stroke.
- I compare each candidate with the original, in place, with a split slider, and accept one with the keyboard.

## 6. Functional requirements
| ID | Requirement (testable) |
|---|---|
| FR-EDT-01 | On open, the view fits the whole original, centred, inside the viewport. |
| FR-EDT-02 | Wheel zoom is anchored at the cursor. One notch (`deltaY` 100) scales by 2^¼. The image point under the cursor moves ≤ 0.5 device px after 2000 notches. Scale is clamped to [0.5 × fit, 32 device px per image px]. |
| FR-EDT-03 | Pan works with Space+drag, middle-drag or the pan tool (`H`). At least 64 CSS px of the image always stays inside the viewport. |
| FR-EDT-04 | Presets: **Fit** (`Ctrl+0`), **Actual pixels** (`Ctrl+1`, 1 image px = 1 device px) and **Display size** (`Ctrl+2`, 1 image px = target ÷ source px per square **CSS** px, R23). Both values come from `export_get_settings` and refresh on `export:settings-changed` (FR-EXP-26). Display size is disabled, with a tooltip, until both exist (R6). The readout shows image px : device px as a percentage. |
| FR-EDT-05 | HiDPI (OQ-CNV-07): Konva `pixelRatio` = `devicePixelRatio`, and the tile level is chosen by `scale × DPR`. After a DPR change, the next animation frame has the new ratio and level, with no reload (asserted through `__pfTest`, R23). |
| FR-EDT-06 | The view is one transform `screen = image × scale + (tx, ty)` in app state. The Konva Stage props always equal it, and Konva keeps no view state of its own. |
| FR-EDT-07 | `edt_tiles_open` returns a pyramid descriptor at once and builds missing levels in the background, smallest first. The one-tile level is shown as soon as it exists, so the view is never blank after that. |
| FR-EDT-08 | The level shown is the smallest whose scale is ≥ the device scale. The one-tile level stays loaded beneath it. Only levels reported built are requested. |
| FR-EDT-09 | The `pf-tiles` scheme follows [Appendix A](#appendix-a-pf-tiles-uri-scheme). It serves only keys registered for the open asset project and never maps a URL to a file path. |
| FR-EDT-10 | The pyramid key changes whenever the image id, source size, source mtime or builder version changes. A level is served only after it is fully written (atomic per-level marker). A partial build is never served and is rebuilt on the next open. |
| FR-EDT-11 | The UI keeps decoded tiles in a 384 MiB LRU. EDT owns the disk tile cache (R5): `<data-dir>/cache/tiles/`, capped at **2 GiB**, evicting the least recently opened pyramids that aren't open. Each eviction pass first removes the caches of older builder versions (`tiles/v<N>/` other than the current one) whole (R105). |
| FR-EDT-12 | A failed tile fetch or decode retries twice (250 ms, 1 s). Then the lower level stays visible and a badge (`edt-tiles-error`) offers **Rebuild** (`edt_tiles_open` with `rebuild: true`). A missing, empty or corrupt tile file is a failed fetch: `pf-tiles` answers 404 for an empty one, and a corrupt one fails to decode; Rebuild replaces them. Tile files get no per-tile fsync, because the tile cache is a cache (R105). |
| FR-EDT-13 | Layers are drawn above the original from the **current manifest layer stack**, in order, each with `{cx, cy, scale, rotation°, opacity, visible}` about its centre (ADR 0003). Read-back corners are within 0.01 px of the affine. Hidden layers aren't drawn. After every `project:committed`, the drawn stack equals the manifest's. |
| FR-EDT-14 | The mask canvas covers the region (the inner rect, R1) at full resolution, ≤ 4096 px per side (OQ-CNV-04). Canvas alpha is the mask value. It is shown as a 50 % tint, toggled with `M`. Strokes are clipped to the region. |
| FR-EDT-15 | Brush (`B`): a round stamp of radius r = size/2. Size is 1–1024 image px (`[`, `]`). Hardness h is 0–100 % (`Shift+[`, `Shift+]`). Alpha is 1 inside h·r and falls linearly to 0 at r. Dabs are spaced max(1, 0.25 r). A 100 % hardness stroke gives 255 inside and 0 outside. Eraser (`E`): the same stamp with `destination-out`. |
| FR-EDT-16 | **Mask whole region** (was "Fill region", R72) and **Clear mask** set every mask pixel to 255 or 0. Each is one undo step ("mask whole region" / "clear mask"). |
| FR-EDT-17 | The brush cursor draws the true radius and the hardness radius at the current zoom. With no region, painting is disabled and the cursor tooltip says "Select a region first". |
| FR-EDT-18 | Hand-off: the UI reads the mask back as w×h 8-bit values for the inner rect (0–255, 255 = editable, ADR 0001) and calls `edt_mask_submit` → `MaskRef`. `edt_mask_read` returns byte-identical bytes, which load back identically. RGN pads to the outer rect and resamples; PRV converts polarity only (R1). |
| FR-EDT-19 | On reopen, the draft's mask is loaded into the mask canvas, pixel-identical to the last submit. If the draft region differs from the mask's rect, the overlap is kept and the rest is 0. |
| FR-EDT-20 | When the region changes, the overlapping mask pixels are kept and the pixels outside the new rect are discarded. Removing the region discards the mask canvas. Each is one undo step. |
| FR-EDT-21 | One stroke (pointer down → up) is one undo entry storing the before and after alpha of its bounding box. `Ctrl+Z` undoes; `Ctrl+Shift+Z` / `Ctrl+Y` redo. Undo then redo is byte-identical. A new action clears the redo stack. History is per session (KQ-16): not persisted, cleared on close. |
| FR-EDT-22 | Undo memory is capped at 256 MiB (alpha only). The oldest entries drop first; the newest is always kept. The undo tooltip shows the remaining depth. |
| FR-EDT-23 | Other features push entries `{label, bytes, undo(), redo()}` onto the same stack. Undo of accept or re-tune = `project_restore_revision(parent)`; redo = restore the later revision (R3). If an entry's call fails, the stack position doesn't move and the error code is shown. The stack belongs to the open project, not to one canvas ([new-asset-generation](new-asset-generation.md) FR-GEN-29, #60): a canvas remount for a new bottom image (a pick or its undo) drops only the mask and region entries, which belong to that canvas; Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y also work in the "No image yet" state. |
| FR-EDT-24 | Compare: given `{key, rect}` (R16), the after image is drawn over that rect in image coordinates at any zoom. **Split** has a draggable divider (default 50 %, `edt-compare-divider`). **Flip** shows "before" while `\` is held. `C` switches the mode. A whole image of another size than the working image, or with no image yet (GEN FR-GEN-26, OQ-GEN-13), is shown **alone**, fit to view (`edt-preview-alone`), instead of split or flip; holding `\` shows the working image. |
| FR-EDT-25 | Candidate picker (R19): lists the candidates it is given, each with a thumbnail, index, UI status `pending \| ready \| invalid` (R10) and star. Click, `1`–`9` or `←`/`→` selects a ready one and emits `onSelect`. `Enter` or `edt-picker-accept` emits `onAccept`. Star and discard emit `onStar` / `onDiscard`. (#56, R67) `X` discards the selected candidate; `Delete` or `X` on a focused item discards that one (a window-level `Delete` still removes the region, FR-EDT-27); the selection and focus move to the next ready candidate. A candidate given as `accepted` can't be discarded. When `onDiscardRest` is given (after an accept), **Discard the rest** (`edt-picker-discard-rest`) emits it (R65). No confirm. It calls no core command itself. |
| FR-EDT-26 | In test builds only, `window.__pfTest` exposes the view, `stage.find`, the mask readback and the undo depth. It is absent from the production bundle (checked by grepping the build). |
| FR-EDT-27 | Region tool (`R`, R18): drag on the image draws a new region; drag inside moves it; 8 handles resize it. On drag end it emits `onRegionChange({inner})` in integer image px, clamped to the image. It draws only the validated `{inner, outer}` it's given back (RGN `rgn_region_fit`): solid inner rect, dashed context margin. `Delete` removes the region. |
| FR-EDT-28 | Draft mask (R7): `edt_mask_submit` runs ≥ 2 s after the last stroke end, before job submit, and on close. It is skipped when the mask hasn't changed since the last successful submit. `edt-mask-status` shows saved / saving / unsaved (reason). |
| FR-EDT-29 | Read-only project (R12): view, region tool, painting and undo work. No mask submit is attempted, a direct call fails fast with `read_only`, and the status says "Read-only: mask not saved". |
| FR-EDT-30 | Single-key and bracket shortcuts are ignored while a text input, textarea or contenteditable has focus (R23). |
| FR-EDT-31 | Transient images (R16): a core fn registers an in-memory image for the open project and returns a key served at `/i/<key>.<ext>` ([Appendix A](#appendix-a-pf-tiles-uri-scheme)). At most 4 per project (≤ 256 MiB); a fifth unregisters the oldest. All are unregistered on close. |
| FR-EDT-32 | `edt_image_open` with `max_side` returns a downscaled copy (Lanczos3, aspect kept) under its own key. The picker uses `max_side` = 256 for thumbnails. |

## 7. UI/UX notes
- **Layout** (slots per [ui-guidelines](../architecture/ui-guidelines.md); EDT owns the toolbar, canvas, status bar and picker): the canvas fills the centre between the left project panel (PRJ) and the right panel (RGN, EXP). The vertical **toolbar** on the canvas's left edge has pan, region, brush, eraser, size, hardness, mask whole region, clear, undo, redo and mask visibility. The **status bar** under the bottom strip has the zoom readout, the presets and `edt-mask-status`, plus the segments other features register (PRJ save state, RGN job state). The **picker** sits in the bottom strip, inside RGN's iteration tracker, showing the selected iteration's candidates (FR-RGN-40).
- **States:** *empty*: not shown; with no asset project open the home grid (PRJ) fills the centre · *loading*: a spinner until the one-tile level exists, then progressive sharpening · *job running*: the picker shows pending slots, and the canvas stays usable · *read-only*: FR-EDT-29 · *error*: the tile badge (FR-EDT-12), or a banner if the build fails · *offline / provider unavailable*: no effect, the canvas never calls the provider.
- **Test ids (R21):** `edt-toolbar-{pan,region,brush,eraser,size,hardness,mask-region,clear,undo,redo,mask}` (`mask-region` was `fill`, R72), `edt-view-{zoom,fit,actual,display}`, `edt-canvas-stage`, `edt-region-box`, `edt-region-handle` (+ `data-handle`), `edt-mask-status`, `edt-tiles-error`, `edt-compare-{divider,mode}`, `edt-picker-{item,star,accept,discard,discard-rest}` (R65); an accepted item carries `data-accepted`. Picker items carry `data-record-id` = candidate id. RGN defines no candidate button ids of its own.

## 8. Data and file effects
Field shapes live in [manifest-schema.md](../architecture/manifest-schema.md) (PRJ). EDT never writes the manifest itself; it only uses PRJ's store API.

| Data | Where | Written by | Recoverable? |
|---|---|---|---|
| Tile pyramid ([Appendix B](#appendix-b-pyramid-format)) | Local cache `<data-dir>/cache/tiles/v<builder>/<key>/` (R5) | `edt_tiles_open` builder | Rebuildable, safe to delete any time |
| Mask: 8-bit grey PNG, one write-once file per submit | Asset project `.pforge/masks/` (PRJ layout) | `edt_mask_submit` via PRJ `put_mask` (atomic) | Yes. Jobs and revisions reference them (KQ-17) |
| Transient images, decoded tiles | Memory | FR-EDT-31, FR-EDT-11 | Rebuilt on demand |
| Undo history, view, brush settings | UI memory | n/a | Lost on close by design (KQ-16, OQ-EDT-11) |

What EDT **needs** from the schema:
- **mask** record: id, file, rect = the region's inner rect in image px.
- **draft** mask reference: `edt_mask_submit` writes the mask and points the draft at it in one commit (R7). Superseded masks that no job or revision references go to the trash at the next commit (PRJ).
- **layer**: id, file, `cx, cy, scale, rotation, opacity, visible`, in stack order. Layer files are write-once, so a new file means a new id (re-tune, R3). That makes the image id a valid cache key.
- **image ids** for the original, layer files and candidates, resolvable to a file by PRJ.

## 9. Commands and events
Errors are `snake_case`, using PRJ's codes (`locked`, `conflict`, `read_only`) where they apply (R12, R21).

| Command (pf-core) | Input → result | Validation and errors |
|---|---|---|
| `edt_tiles_open` | `{project_id, image_id, rebuild?}` → `{key, base_url, width, height, tile_size, levels[]}` | The image must belong to the open project. `not_found`, `unsupported` (decode failure), `too_large` (> 16384 per side), `io`. Idempotent. |
| `edt_image_open` | `{project_id, image_id, max_side?}` → `{key, url, width, height}` | Without `max_side`: ≤ 4096 per side, else `use_tiles`. Otherwise as above. While a conflict is open it also resolves either version's original or patch ids; a copy's path with `..` or an absolute path is refused (R71). |
| `edt_mask_submit` | `{project_id, rect: {x, y, w, h}}` + raw w·h bytes → `MaskRef {mask_id, rect}` | Integer rect inside the image, 1 ≤ w, h ≤ 4096. `invalid_rect`, `size_mismatch` (body ≠ w·h), `read_only`, `locked` / `conflict` / `io` (the mask stays in the UI and is retried at the next trigger or after the conflict is resolved). |
| `edt_mask_read` | `{project_id, mask_id}` → raw w·h bytes + rect | `not_found`, `corrupt` (not 8-bit grey, or size ≠ rect) |

- **Core fn (not a command):** `register_transient_image(project_id, image) → key` (FR-EDT-31). RGN uses it to return previews as `{key, rect}`.
- **UI exports** (`features/editor/index.ts`, read-only for other features): `useEditorMaskPainted()` → whether the live mask has paint (`null` before EDT knows), for RGN's mask hint (R72); `LayerStack` draws a given layer stack (the original copy via `edt_image_open`, max side 1024, plus the baked patch files placed by the manifest affine, hidden ones skipped, no feather or colour match in the UI), for PRJ's conflict view (R71, OQ-PRJ-11 A).
- **Events:** `edt:tiles-progress {key, level}` once per completed level · `edt:tiles-ready {key}` · `edt:tiles-failed {key, error}`.

## 10. Provider interaction
N/A. The canvas never calls the provider. Candidates and previews reach it as image keys from RGN.

## 11. Edge cases
| Case | Behaviour |
|---|---|
| Crash mid-save | PRJ writes the mask file atomically. A crash loses only strokes since the last submit (≤ 2 s idle). A crash mid-build leaves no level marker, so the level is rebuilt (FR-EDT-10). |
| Sync conflict / file locked | The tile cache is outside the library. A `locked` or `conflict` mask write keeps the mask in memory, the status shows unsaved, and it is retried (FR-EDT-28). A synced replacement original gets a new key on the next open. |
| Offline / provider error or timeout | No effect on the canvas (§10). Read-only: FR-EDT-29. |
| Very large image | Up to 16384 per side with tiles; larger → `too_large` with a message. No full-image canvas anywhere (ADR 0003). |
| Corrupt or unsupported input | Build fails → `edt:tiles-failed` → banner; the app keeps running. A corrupt cached tile → retry, then Rebuild. A corrupt mask → `corrupt`, and the mask canvas starts empty with a warning. |
| Undo across the feature boundary | Submitted masks are immutable; undo changes only the live mask canvas. Accept and re-tune undo through revisions (FR-EDT-23). After a restore, the live mask canvas is kept (OQ-EDT-14 ✅ A, #41). |
| GPU reset / context loss | The mask canvas is CPU-backed (`willReadFrequently`), so it can't lose content. Tiles redraw from the LRU. |

## 12. Performance budget
Reference: the spike rig (iGPU, 1600×900) at DPR 1 and 1.5, measured by a non-gating bench (CI has no GPU).

| Metric | Budget |
|---|---|
| Open → one-tile level visible and interactive (cold) | typical (#30) ≤ 1.5 s · 16k ≤ 5 s · warm ≤ 0.5 s |
| Pan/zoom over 16k tiles | p95 frame ≤ 16.7 ms, no frame > 100 ms after first view |
| Mask stroke at 4096² | p95 frame ≤ 16.7 ms; a dab shows on the next frame |
| Mask readback + `edt_mask_submit` at 4096² · mask undo/redo step | ≤ 0.5 s · ≤ 50 ms |
| Tile serve (release build) | ≤ 2 ms per tile |
| Memory at 16k | UI: tiles 384 MiB + mask 64 MiB + undo 256 MiB · core: transient ≤ 256 MiB (whole-app line: PRJ, R11) |

## 13. Test plan
- **Unit (Vitest), `test("FR-EDT-NN …")`:** view math (anchored drift, clamps), display-size scale in CSS px, level choice × DPR, tile wanted-set, stamp profile, undo cap, readback alpha→bytes, compare split geometry, region drag/move/resize → `{inner}`, debounce timing (fake timers), shortcut suppression.
- **Unit (Rust), `fr_edt_NN_…`:** URI grammar table (valid, malformed, traversal: `..`, `%2e`, absolute paths), key derivation, level sizes, `edt_mask_submit` validation incl. `read_only`, 8-bit PNG round trip, transient registry cap and unregister on close, `max_side` size.
- **Integration (Rust):** build a pyramid from a generated fixture and serve every tile. Level 0 decodes **exactly** equal to the source; JPEG levels within tolerance. An mtime change gives a new key. A build aborted after level k (`PF_FAULT`) is never served and is rebuilt on reopen.
- **Thin e2e (Playwright, mocked core, KQ-33):** open → fit · wheel anchor · presets, display size enabled by `export:settings-changed` · `deviceScaleFactor: 2` → ratio and level · region draw/move/resize/delete · stroke → undo/redo byte-identical · mask whole region / clear · debounced `edt_mask_submit` payload · reopen → mask restored · read-only → no submit, status · shortcuts ignored in a text field · tile 404 → badge · compare split/flip · picker keys and `onSelect`/`onAccept` events · picker `X` / focused `Delete` discard, accepted not discardable, Discard the rest (25, #56) · failed restore entry → stack unchanged. Assertions use `__pfTest`, not canvas pixels (ADR 0003).
- **Tauri smoke (Windows):** open the generated typical fixture through the real scheme; the one-tile level appears and `/t/` responses are 200.
- **Fixtures (generated, KQ-22, no real art):** `fixtures/gen` battlemap (grid, rooms, noise) at 2048×2560 (committed if < 1 MB), 4200×5600 and 16384² (on demand). A coordinate-probe image (R/G encode x, y). Mask patterns (disc, ramp, checker). Candidates from the fake provider.
- **App walkthrough** (test agent; `PF_DATA_DIR` and `PF_LIBRARY_DIR` set inside the worktree; generated 4200×5600 map):
  1. Create a Maps project from it and open it. → Fit within 1.5 s, and the image sharpens.
  2. Wheel-zoom on a door to 800 %. → The door stays under the cursor; pixels are crisp at 100 % and 150 % Windows scaling.
  3. `Ctrl+1`, then `Ctrl+2`. → 1:1 device px, then display size (or disabled with a tooltip if EXP values aren't set).
  4. `R`, drag, move and resize a region; paint a soft stroke, erase part, undo twice. → Inner rect and dashed margin follow; each undo reverses exactly; after 2 s idle the status shows saved.
  5. Submit with the fake provider; use the picker (keys 1–3, split drag, hold `\`, `Enter`). → Compare follows the selection; accept emits once. `Ctrl+Z` restores the parent revision.
  6. Close, delete the tile cache, reopen. → The pyramid rebuilds, nothing breaks, and the mask is pixel-identical.

## 14. Build split
| # | Slice | Shared files touched (AGENTS §9) | Depends on |
|---|---|---|---|
| 1 | Pyramid builder + cache + `edt_tiles_open`/events + `pf-tiles` scheme | `commands.rs` (append); scheme registration (orchestrator one-liner) | Skeleton; `project_open` |
| 2 | View, tile manager, presets, HiDPI, shortcuts, test hook (FR 01–08, 11–12, 26, 30) | UI feature registry (append) | 1 (or the mock) |
| 3 | Layer display + `edt_image_open` + transient images (FR 13, 31–32) | `commands.rs` (append) | 2; PRJ layer fields |
| 4 | Region tool + mask tools + undo (FR 14–17, 19–23, 27) | none | 2; `rgn_region_fit` (or the mock) |
| 5 | Mask hand-off, draft debounce, read-only, reopen (FR 18, 28–29) | `commands.rs` (append) | 4; PRJ `put_mask` |
| 6 | Compare + candidate picker (FR 24–25) | none | 3; RGN wiring after |

## 15. Open questions
None open.

**Resolved:** OQ-EDT-01 ✅ A, both values required (#40, R6) · OQ-EDT-02 ✅ A, 100 % = 1 device px (#40) · OQ-EDT-03 ✅ region first (R18 = OQ-RGN-01 A) · OQ-EDT-04 ✅ inner-rect mask, RGN pads (R1) · OQ-EDT-05 ✅ one shared stack, accept undone by revision restore (R3) · OQ-EDT-06 ✅ 256 MiB (#40) · OQ-EDT-07 ✅ write-once PNG via PRJ, draft debounce (#40, R7) · OQ-EDT-08 ✅ layer files write-once (#40, R3) · OQ-EDT-09 ✅ 2 GiB, EDT owns the cache (R5) · OQ-EDT-10 ✅ compare shows RGN's baked preview as `{key, rect}` (#40, R16) · OQ-EDT-11 ✅ view not persisted (#40, owner) · OQ-EDT-12 ✅ CPU-backed mask canvas (#40) · OQ-EDT-13 ✅ *Display size* added to the glossary (R27) · OQ-EDT-14 ✅ A, the live mask canvas wins and is re-submitted (#41).

## Appendix A: `pf-tiles` URI scheme
- Host: `pf-tiles` (`http://pf-tiles.localhost/…` on Windows). The UI takes `base_url` / `url` from command results and never builds hosts itself, so Playwright mocks it with a static folder.
- `GET /t/<key>/<level>/<col>_<row>.<jpg|webp>` → a tile. `GET /i/<key>.<png|webp>` → a whole image (`edt_image_open` or a transient key).
- Grammar: `key = [A-Za-z0-9_-]{1,80}`; level, col and row are decimal integers. Anything else → 400.
- Unknown key or out of range → 404. Level not built yet → 425. Keys are unregistered when the project closes (then 404).
- 200 responses carry `Content-Type` and `Cache-Control: max-age=31536000, immutable`, safe because keys are immutable.

## Appendix B: Pyramid format
- Level 0 = 1:1 (lossless WebP). Level n = ⌈size / 2ⁿ⌉ (JPEG q90, 2× box filter), up to the first level that fits one 512 px tile (ADR 0004). Edge tiles are partial, not padded.
- `pyramid.json`: `{key, width, height, tile_size, builder_version, source: {size, mtime_ns}, levels: [{level, scale, cols, rows, ext, complete}]}`.
- Key = `<image-id>-<16 hex>`: an in-crate FNV-1a 64 hash (no new dependency) of image id, source size, mtime_ns and builder version. A `max_side` copy adds it to the hash input.

---
feature: editor-canvas
prefix: EDT
owner: spec-editor-canvas
status: in-review   # unclaimed → claimed → draft → in-review → complete (owner only)
depends_on: [asset-project]
---

# Editor canvas

## 1. Summary
The editor canvas shows an asset project's **original** and its **layers** at full fidelity, with zoom and pan up to 16k px (#11, #30). The DM paints a **mask** over a **region** there, and compares and picks **candidates** on it.
It implements [ADR 0003](../architecture/adr/0003-canvas-library.md): Konva, one app-owned **view** transform, a **tile pyramid** served over a custom URI scheme, and a region-sized mask surface. It covers MVP scenario steps 1, 2 and 4 ([vision](../vision.md)).

## 2. Scope
Tier S ([feature-tiers](../feature-tiers.md)): *Zoom, pan, view at intended display size* · *Paint and adjust masks (brush, erase, feather)* · *Candidate comparison (before/after, pick)*, the UI components only · *Undo within a session* (KQ-16) · layer **display** for patch layers. Ownership follows the [phase 1b seams](../agents/seams-phase1b.md).

## 3. Goals
- A typical map is usable in < 2 s and a 16k map in < 5 s (#35). Pan, zoom and painting stay smooth on an iGPU.
- The mask the DM sees is exactly the mask that gets handed off: byte-identical, and pixel-identical after reopen (#35).
- Mask edits can be undone one stroke at a time within a bounded memory budget.
- Every behaviour can be tested in Playwright with mocked core commands (KQ-33).

## 4. Non-goals
- Moving, scaling or rotating layers, or editing opacity and visibility (tier A). The Konva `Transformer` isn't used in tier S.
- Selecting regions, composing jobs, accepting candidates and baking the feather (RGN). Flatten and export (EXP). The canvas never composites for output (KQ-25).
- Pressure-sensitive brushes, lasso or shape masks, mask invert, and grid overlays (tier A/B).

## 5. User stories
- As a DM preparing a session, I open a battlemap and zoom from the whole map down to single pixels without waiting or blurring.
- I flip to **display size** to judge the map the way my players will see it in my VTT.
- I paint a soft mask over one room, erase an overshoot, and undo a bad stroke.
- I compare each candidate with the original, in place, with a split slider, and pick one with the keyboard.

## 6. Functional requirements
| ID | Requirement (testable) |
|---|---|
| FR-EDT-01 | On open, the view fits the whole original, centred, inside the viewport. |
| FR-EDT-02 | Wheel zoom is anchored at the cursor. One notch (`deltaY` 100) scales by 2^¼. The image point under the cursor moves ≤ 0.5 device px after 2000 notches. Scale is clamped to [0.5 × fit, 32 device px per image px]. |
| FR-EDT-03 | Pan works with Space+drag, middle-drag or the pan tool (`H`). At least 64 CSS px of the image always stays inside the viewport. |
| FR-EDT-04 | Zoom presets: **Fit** (`Ctrl+0`), **Actual pixels** (`Ctrl+1`, 1 image px = 1 device px) and **Display size** (`Ctrl+2`, scale = export px per square ÷ source px per square, OQ-EDT-01). Display size is disabled, with a tooltip, until both values exist. The readout shows image px : device px as a percentage. |
| FR-EDT-05 | HiDPI (OQ-CNV-07): the Konva `pixelRatio` equals `devicePixelRatio`. The tile level is chosen by `scale × DPR`. A DPR change (window moved to another monitor) re-renders within one frame, without a reload. |
| FR-EDT-06 | The view is one transform `screen = image × scale + (tx, ty)` in app state. The Konva Stage props always equal it, and Konva keeps no view state of its own. |
| FR-EDT-07 | `tiles_open` returns a pyramid descriptor at once and builds missing levels in the background, smallest level first. The single-tile backdrop level is shown as soon as it exists, so the view is never blank after that. |
| FR-EDT-08 | The level shown is the smallest one whose scale is ≥ the device scale. The backdrop stays loaded beneath it. Only levels already reported built are requested. |
| FR-EDT-09 | The URI scheme follows §9 exactly. It serves only keys registered for the open asset project and never maps a URL to an arbitrary file path. Malformed → 400, unknown key or out of range → 404, level not built yet → 425. |
| FR-EDT-10 | Invalidation: the key changes whenever the image id, source size, source mtime or builder version changes. A level is served only after it was fully written (atomic per-level marker). A partial build is never served and is rebuilt on the next open. |
| FR-EDT-11 | The UI keeps decoded tiles in an LRU of 384 MiB. The disk cache is capped (OQ-EDT-09) by evicting the least recently opened pyramids that aren't open. |
| FR-EDT-12 | A failed tile fetch or decode retries twice (250 ms, 1 s). After that the lower level stays visible and an error badge (`edt-tiles-error`) offers **Rebuild**, which calls `tiles_open` with `rebuild: true`. |
| FR-EDT-13 | Layers are drawn above the original in manifest order, each with `{cx, cy, scale, rotation°, opacity, visible}` about its centre (ADR 0003). Read-back corners are within 0.01 px of the affine. Hidden layers aren't drawn. |
| FR-EDT-14 | The mask surface is one canvas covering the region rect (OQ-EDT-03/04) at full resolution, ≤ 4096 px per side (OQ-CNV-04). Canvas alpha is the mask value. It's shown as a 50 % tint, toggled with `M`. Strokes are clipped to the region rect. |
| FR-EDT-15 | Brush (`B`): a soft round stamp of radius r = size/2. Size is 1–1024 image px (`[`, `]`). Hardness h is 0–100 % (`Shift+[`, `Shift+]`). Alpha is 1 inside h·r and falls linearly to 0 at r. Dabs are spaced at max(1, 0.25 r). A 100 % hardness stroke gives 255 inside and 0 outside. |
| FR-EDT-16 | Eraser (`E`): the same stamp, composited with `destination-out`. |
| FR-EDT-17 | **Fill region** and **Clear mask** set every mask pixel to 255 or to 0. Each is one undo step. |
| FR-EDT-18 | The brush cursor draws the true radius and the hardness radius at the current zoom. With no region, painting is disabled and the cursor tooltip says "Select a region first". |
| FR-EDT-19 | Hand-off: the UI reads the region back as w×h 8-bit values (255 = editable, ADR 0001) and calls `mask_submit`, which returns a `MaskRef`. `mask_read` of that ref returns byte-identical bytes, and loading them back into the surface reads back identically. |
| FR-EDT-20 | On reopen, the mask stored in the asset project is loaded into the surface. It is pixel-identical to the mask at save time. |
| FR-EDT-21 | When the region changes, the overlapping mask pixels are kept and the pixels outside the new rect are discarded. That's one undo step. |
| FR-EDT-22 | Undo granularity: one stroke (pointer down → up) is one entry, storing the before and after alpha of the stroke's bounding box. `Ctrl+Z` undoes, and `Ctrl+Shift+Z` / `Ctrl+Y` redo. Undo then redo restores a byte-identical mask. A new action clears the redo stack. |
| FR-EDT-23 | Undo memory is capped (OQ-EDT-06). The oldest entries are dropped first, and the newest is always kept. The undo button tooltip shows the remaining depth. |
| FR-EDT-24 | Undo history is per session (KQ-16). It isn't persisted and is cleared when the asset project closes. |
| FR-EDT-25 | Other features push entries with `{label, bytes, undo(), redo()}` onto the same stack (OQ-EDT-05). |
| FR-EDT-26 | Compare: given `{rect, afterKey}`, the after image is drawn over the region in image coordinates at any zoom. **Split** mode has a draggable divider (default 50 %, `edt-compare-divider`). **Flip** shows "before" while `\` is held. `C` switches the mode. |
| FR-EDT-27 | Candidate picker: lists the candidates it's given, each with a thumbnail, index, status (pending / ready / failed) and star. Click, `1`–`9` or `←`/`→` selects one, and the selection drives the compare. It emits `onPick`, `onDiscard` and `onStar` and calls no core command itself. |
| FR-EDT-28 | In test builds only, `window.__pfTest` exposes the view, `stage.find`, the mask readback and the undo depth. It is absent from the production bundle (checked by grepping the build). |

## 7. UI/UX notes
- **Layout:** the canvas fills the editor area. A left toolbar has pan, brush, eraser, size, hardness, fill, clear, undo, redo and mask visibility. A bottom bar has the zoom readout and the Fit / Actual / Display presets. The picker is a strip above the bottom bar while candidates exist.
- **Region box:** a solid inner rect and a dashed context margin, drawn from RGN's geometry.
- **States:** *empty* (no asset project open): "Open an image" · *loading*: a spinner until the backdrop exists, then progressive sharpening · *job running*: the picker shows pending slots, and the canvas stays fully usable · *error*: the tile badge (FR-EDT-12), or a banner if the pyramid build fails · *offline / provider unavailable*: no effect, because the canvas never calls the provider.
- **Test ids:** `edt-toolbar-{pan,brush,eraser,size,hardness,fill,clear,undo,redo,mask}`, `edt-view-{zoom,fit,actual,display}`, `edt-canvas-stage`, `edt-compare-{divider,mode}`, `edt-picker-{item,star,pick,discard}`. Picker items carry `data-record-id` = candidate id.

## 8. Data and file effects
| Data | Where | Written by | Recoverable? |
|---|---|---|---|
| Tile pyramid: `pyramid.json` + `<level>/<col>_<row>.<ext>` | Local cache (#19; location owned by PRJ), `tiles/v<builder>/<key>/` | `tiles_open` builder | Rebuildable. Safe to delete at any time. |
| Mask files: 8-bit greyscale PNG, one immutable file per submit | Asset project. Path and manifest field owned by PRJ (OQ-EDT-07) | `mask_submit`, using PRJ's atomic write | Yes. Jobs reference them (KQ-17). |
| Undo history, view, brush settings | UI memory | n/a | Lost on close by design (KQ-16, OQ-EDT-11) |

- Pyramid levels: level 0 = 1:1 (lossless WebP), level n = ⌈size / 2ⁿ⌉ (JPEG q90, 2× box filter), up to the first level that fits one 512 px tile (ADR 0004). Edge tiles are partial, not padded.
- `pyramid.json`: `{key, width, height, tile_size, builder_version, source: {size, mtime_ns}, levels: [{level, scale, cols, rows, ext, complete}]}`.
- Key = `<image-id>-<16 hex>`. The hex is an in-crate FNV-1a 64 hash (no new dependency) of the image id, source size, mtime_ns and builder version.
- EDT never writes the manifest itself. Mask refs reach the manifest through PRJ.

## 9. Commands and events
| Command (pf-core) | Input → result | Validation and errors |
|---|---|---|
| `tiles_open` | `{project_id, image_id, rebuild?}` → `{key, base_url, width, height, tile_size, levels[]}` | The image must belong to the open project. `NotFound`, `Unsupported` (decode failure), `TooLarge` (> 16384 per side), `Io`. Repeated calls are idempotent. |
| `image_open` | `{project_id, image_id}` → `{key, url, width, height}` for patch layers and candidates | ≤ 4096 per side, otherwise `UseTiles`. Same errors as above. |
| `mask_submit` | `{project_id, rect: {x, y, w, h}}` + raw body of w·h bytes → `MaskRef {mask_id, rect}` | Integer rect inside the image, 1 ≤ w, h ≤ 4096. `InvalidRect`, `SizeMismatch` (body ≠ w·h), `Io` / `Locked` (the mask stays in the UI, and Retry is offered). |
| `mask_read` | `{project_id, mask_id}` → raw w·h bytes + rect | `NotFound`, `Corrupt` (not 8-bit grey, or the size disagrees with the manifest) |

- **Events:** `edt:tiles-progress {key, level}` once per completed level · `edt:tiles-ready {key}` · `edt:tiles-failed {key, error}`.
- **URI scheme `pf-tiles`** (`http://pf-tiles.localhost/…` on Windows). The UI takes `base_url` from the command result and never builds hosts itself, so Playwright mocks it with a static folder:
  - `GET /t/<key>/<level>/<col>_<row>.<jpg|webp>` → a tile.
  - `GET /i/<key>.<png|webp>` → a whole image from `image_open`.
  - Grammar: `key = [A-Za-z0-9_-]{1,80}`, and integers only.
  - 200 responses carry `Content-Type` and `Cache-Control: max-age=31536000, immutable`, which is safe because keys are immutable.
  - Keys are unregistered when the project closes, and then return 404.

## 10. Provider interaction
N/A. The canvas never calls the provider. Candidates reach it as image keys from RGN.

## 11. Edge cases
| Case | Behaviour |
|---|---|
| Crash mid-save | A mask file is written atomically by PRJ. A crash before `mask_submit` loses only the unsubmitted strokes. A crash mid-build leaves no `complete` marker, so the level is rebuilt (FR-EDT-10). |
| Sync conflict / file locked | The cache is outside the library, so unaffected. A locked mask write → `Locked`, the mask stays in memory, and Retry is offered. An original replaced by sync gets a new key on the next open. |
| Offline / provider error or timeout | No effect on the canvas. |
| Very large image | Up to 16384 per side is fully supported with tiles. Larger → `TooLarge` with a message. There's no full-image canvas anywhere (ADR 0003). |
| Corrupt or unsupported input | The build fails → `edt:tiles-failed` → banner, and the app keeps running. A corrupt cached tile → retry, then Rebuild (FR-EDT-12). A corrupt mask file → `Corrupt`, and the surface starts empty with a warning. |
| Undo across the feature boundary | Masks already submitted are immutable, and undo changes only the live surface. Entries pushed by other features follow OQ-EDT-05. |
| Region resized or removed with a mask painted | FR-EDT-21. Removing the region discards the surface as one undo step. |
| GPU reset / canvas context loss | The mask canvas is CPU-backed (OQ-EDT-12), so it can't lose content. The tile layer is redrawn from the LRU. |

## 12. Performance budget
Reference: the spike rig (iGPU, 1600×900) at DPR 1 and 1.5. These are measured by a non-gating bench, because CI has no GPU.

| Metric | Budget |
|---|---|
| Open → backdrop visible and interactive (cold cache) | typical (#30) ≤ 1.5 s · 16k ≤ 5 s · warm cache ≤ 0.5 s |
| Pan/zoom over 16k tiles | p95 frame ≤ 16.7 ms, no frame > 100 ms after first view |
| Mask stroke at 4096² | p95 frame ≤ 16.7 ms. A dab shows on the next frame. |
| Mask readback + `mask_submit` at 4096² | ≤ 0.5 s |
| Undo/redo step | ≤ 50 ms |
| Tile serve (release build) | ≤ 2 ms per tile |
| UI memory at 16k | tiles 384 MiB + mask 64 MiB + undo cap (OQ-EDT-06) |

## 13. Test plan
- **Unit (Vitest):** view math (anchored-zoom drift, round trip, clamps), level choice × DPR, the tile wanted-set, the stamp profile values, undo cap accounting, readback alpha→bytes, compare split geometry. Named `test("FR-EDT-NN …")`.
- **Unit (Rust):** URI grammar table (valid, malformed, traversal attempts such as `..`, `%2e`, absolute paths), key derivation, level dimensions, `mask_submit` validation, 8-bit PNG round trip. Named `fr_edt_NN_…`.
- **Integration (Rust):** build a pyramid from a generated fixture and serve every tile. Level 0 decodes **exactly** equal to the source, and the JPEG levels are within tolerance. An mtime change gives a new key. An aborted build after level k is rebuilt on reopen and never served while partial.
- **Thin e2e (Playwright, mocked core, KQ-33):** open → fit · wheel anchor · presets · `deviceScaleFactor: 2` → `pixelRatio` and level · stroke → undo/redo, byte-identical · fill/clear · `mask_submit` payload captured by the mock · reopen → mask restored · tile 404 → badge · compare split/flip · picker keys and events. Assertions use `__pfTest`, not canvas pixels (ADR 0003).
- **Tauri smoke (Windows):** open the generated typical fixture through the real scheme; the backdrop appears and the `/t/` responses are 200.
- **Fixtures (generated, KQ-22, no real art):** `fixtures/gen` battlemap generator (grid, rooms, noise) at 2048×2560 (committed if < 1 MB), 4200×5600 and 16384² (generated on demand). A **coordinate-probe** image (R/G encode x, y) for exact tile checks. Mask patterns (disc, ramp, checker). Candidates come from the fake provider (PRV).
- **App walkthrough** (test agent, generated 4200×5600 map):
  1. Open it. → Fit view within 1.5 s, and the backdrop sharpens.
  2. Wheel-zoom on a door to 800 %. → The door stays under the cursor, and the pixels are crisp at 100 % and 150 % Windows scaling.
  3. `Ctrl+1`, then `Ctrl+2`. → 1:1 device px, then display size (or disabled with a tooltip if EXP values aren't set).
  4. Select a region, paint a soft stroke, erase part of it, and undo twice. → Each step reverses exactly.
  5. Submit with the fake provider, then save and reopen. → The mask is pixel-identical.
  6. Use the picker: keys 1–3, split drag, hold `\`. → The compare follows the selection, and Pick emits once.
  7. Delete the tile cache while the project is closed, then reopen. → The pyramid rebuilds and nothing breaks.

## 14. Build split
| # | Slice | Shared files touched (AGENTS §9) | Depends on |
|---|---|---|---|
| 1 | Pyramid builder + cache + `tiles_open`/events + `pf-tiles` scheme | `commands.rs` (append). Scheme registration in the Tauri builder (orchestrator one-liner). | Skeleton; PRJ cache location + `project_open` |
| 2 | View, tile manager, presets, HiDPI, test hook (FR 01–08, 11–12, 28) | UI feature registry (append) | 1 (or the mock only) |
| 3 | Layer display + `image_open` (FR 13) | `commands.rs` (append) | 2; PRJ layer fields |
| 4 | Mask tools + undo (FR 14–18, 21–25) | none | 2; OQ-EDT-03 |
| 5 | Mask hand-off `mask_submit`/`mask_read` + reopen (FR 19–20) | `commands.rs` (append) | 4; PRJ mask storage (OQ-EDT-07) |
| 6 | Compare + candidate picker (FR 26–27) | none | 3; RGN wiring after |

## 15. Open questions
| ID | Question | Options (pick in **bold**) | Blocking? | Affects |
|---|---|---|---|---|
| OQ-EDT-01 | What does "display size" mean in tier S? | **A. Export px per square ÷ source px per square, both stored by [export](export.md)** (grid metadata is tier B) / B. a manual % per session | No | EXP |
| OQ-EDT-02 | What is "100 %" on a HiDPI display? | **A. 1 image px = 1 device px** (true pixels for seam review) / B. 1 CSS px (looks larger, but resampled) | No | — |
| OQ-EDT-03 | Does the region come before the mask? | **A. Yes: the DM draws the region, then paints inside it** (a fixed-size surface, as in ADR 0003) / B. the region is derived from the mask's bounding box (a growing surface, more code) | **Yes**, for slice 4 | [region-edit](region-edit.md) |
| OQ-EDT-04 | Does the mask cover the context margin? | **A. No: the mask is region-rect sized, and RGN pads the context with 0** (context is never editable) / B. it covers rect + context | No | region-edit |
| OQ-EDT-05 | Does undo cover more than the mask? | **A. One shared stack. RGN pushes "accept candidate", whose undo removes the patch layer as a new revision** / B. mask only; accept is reversed via the revision history | No | region-edit, asset-project |
| OQ-EDT-06 | Undo memory cap | **A. 256 MiB, storing alpha only (2 bytes/px per entry)** / B. 128 MiB (fewer full-region steps at 4096²) | No | — |
| OQ-EDT-07 | How masks are stored | **A. One immutable 8-bit greyscale PNG per `mask_submit`, written with PRJ's atomic write; the manifest holds `MaskRef`** / B. PRJ owns a separate save path for masks | No | [asset-project](asset-project.md) |
| OQ-EDT-08 | Are layer image files immutable (a new id per change)? | **A. Yes, so the image id is a valid cache key** / B. no; then EDT hashes the content (slower open) | No | asset-project |
| OQ-EDT-09 | Tile disk cache cap | **A. 2 GiB, least recently opened evicted first** (≈ 7 maps at 16k) / B. no cap; cleared manually | No | asset-project (#19) |
| OQ-EDT-10 | What "after" image does compare show? | **A. Rust's baked preview: the same feather + colour match as accept** (shows the real seam) / B. the raw candidate (cheaper, but misleading at the edge) | No | region-edit |
| OQ-EDT-11 | Persist the view per asset project? | **A. No; fit on reopen** (no churn in synced files) / B. yes, in the manifest | No | asset-project |
| OQ-EDT-12 | Make the mask canvas CPU-backed (`willReadFrequently: true`)? | **A. Yes, if the §12 stroke budget holds** (no context loss, fast readback) / B. GPU-backed, as in the spike | No | — |
| OQ-EDT-13 | Glossary: add **Display size** (the zoom at which the asset appears as it will in the VTT)? | **A. Add it** / B. keep it as a UI label only | No | glossary |

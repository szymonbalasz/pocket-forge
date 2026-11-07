# Research: canvas library for large battlemaps (KQ-32)

> Spike `spike/canvas-large-image`, measured 2026-11-07. Prototype and run steps: [spikes/canvas](../../spikes/canvas/README.md). Full numbers: [results/summary.md](../../spikes/canvas/results/summary.md).
> Rig: Windows 11 laptop, Intel UHD iGPU (the default) and RTX 4080 Laptop dGPU, 240 Hz panel, on AC power. Edge 157 = WebView2 157 (same Chromium). Viewport 1600×900, DPR 1, vsync on.
> Fixture: a procedural synthetic battlemap (grid at 100 px, rooms, flagstones, noise), 8192² (11.8 MB JPEG) and 16384² (53 MB JPEG), plus a 512 px tile pyramid. No real art.

## Recommendation

**Konva (with react-konva), drawing a tile pyramid served by the Rust core through a custom URI scheme.** Proposed in [proposed-canvas-library.md](../architecture/adr/proposed-canvas-library.md).

- **Tiles are required whatever the library is.** A single 16k bitmap takes 1.7–2.0 s to first view and 2.5–3.5 GB of browser memory, and in the Canvas2D libraries it causes 0.8–1.0 s hitches while panning. Tiles show first view in 20–60 ms, use 550–800 MB of browser memory and keep 240 fps.
- **With tiles, display performance is a tie.** All three hold 228–240 fps on the iGPU and on the dGPU, with worst frames of 4.5–21 ms. Coordinate accuracy is also a tie (about 1e-11 px).
- **Typical maps are smaller (#30: about 2.1k–5.6k px per side).** At that size a single bitmap would be acceptable (8k single: 0.4 s to first view, 220 ms worst frame). Tiles still keep one code path up to the 16k bound, and they're cheaper at every size.
- **Konva wins on what we'd otherwise build or maintain.** Free-form layer handles are tier A, while tier S needs only a positioned patch layer. That makes the Transformer a head start rather than a must-have. It has a built-in Transformer (move/scale/rotate handles), an official React binding that is current, and layers as separate canvases. A mask canvas is drawn live with no texture upload, and its bundle is the smallest of the three.

## Decisive findings

1. **Display: tiles vs single image** (16k, iGPU). Tiles: first view in 25–37 ms, ~240 fps, 360 MB shared GPU memory, ~800 MB browser private memory. Single image: first view in 1.7–2.0 s (decode 0.9 s), 1.6–2.1 GB shared GPU memory, 3.0–3.5 GB private, and in Konva/Fabric 5–6 frames over 33 ms with a worst frame of 0.84–0.99 s. Pixi handles a single texture smoothly once it's uploaded, but it is hard-capped at 16384 px per side (`MAX_TEXTURE_SIZE`).
2. **Mask painting** (r = 60 px, 50 % feather, eraser, 16k tiles underneath, iGPU). Konva/Fabric: 228–238 fps at 2048² and 4096², with 0.3–1.6 ms CPU per frame. **Pixi with one 4096² mask canvas: 139 fps (p95 12.5 ms)**, because WebGL re-uploads the whole 64 MB texture on every stroke. Pixi needs 512 px mask tiles to get back to 233 fps. Reading the mask back as 8-bit takes 66–265 ms in all libraries.
3. **Patch layer.** The transform read back from each library matches the image-px affine to 4.5e-13 px. Konva (`Transformer`) and Fabric (controls) ship move/scale/rotate handles. **Pixi has none**: handles, hit areas, cursors and keep-ratio would all be our code.
4. **Coordinates.** Round trip ≤ 5.1e-12 px, anchored-zoom drift ≤ 6e-11 px over 2000 wheel steps. A rendered probe at image (16000, 16000) lands on the predicted screen pixel at zoom 8 and 32 (0 mismatches out of 5496). This holds for all three libraries, because the view lives in one shared transform (`core/view.ts`).
5. **Tauri.** The winner behaves the same in the real WebView2 window: 240 fps on 16k tiles, 232 fps while painting the mask. Tile transport (debug Rust build, 8 tiles in parallel, ~49 KB JPEG tiles): **custom URI scheme 0.62 ms/tile**, raw-bytes IPC 0.77 ms, **base64 IPC 2.9 ms** (4.7× slower and 2.6× less throughput).
6. **Environment traps.**
   - WebView2/Edge uses the **iGPU by default** on a hybrid laptop. It's enough for everything above.
   - **On battery, Windows/Edge capped every library at 30 fps.** The battery saver was on at 17 %. The app would inherit this.
   - With vsync off, Pixi showed 0.3–5 s stalls inside `render()` (WebGL back-pressure) that didn't occur with vsync on, so all published numbers are vsync on.

## Comparison

| Criterion | Konva 10.9.1 | Fabric 7.4.0 | PixiJS 8.22.0 |
|---|---|---|---|
| Tiled 16k pan/zoom, iGPU (fps / worst frame) | 239 / 21 ms | 238 / 9 ms | 240 / 13 ms |
| Single 16k, iGPU (first view / worst frame) | 1.85 s / 837 ms | 1.67 s / 929 ms | 1.97 s / 13 ms (≤ 16384 only) |
| Mask 4096², one region canvas (fps / p95) | 228 / 4.5 ms | 229 / 4.4 ms | **139 / 12.5 ms** |
| Coordinate round trip / rendered probe | 5e-12 px / 0 misses | same | same |
| Move/scale/rotate handles | **built in** (`Transformer`) | **built in** (controls) | none: build it |
| Layer model | Stage → Layers (own canvases) → Groups/nodes | one scene canvas + one interaction canvas | scene graph, one WebGL canvas |
| React binding | **react-konva 19.3.2** (official, Oct 2026) | none official | @pixi/react 8.0.5 (last release Dec 2025) |
| TypeScript types | bundled | bundled (TS rewrite) | bundled |
| Licence | MIT | MIT | MIT |
| Bundle, tree-shaken (min / gzip) | **243 / 63 KB** | 342 / 93 KB | 642 / 164 KB |
| Maintenance | ~100 commits in 3 months, 4 open issues, monthly releases | 82 commits, 462 open issues, last release May 2026 | ~100 commits, 419 open issues, releases every 2–4 weeks |
| Docs (qualitative) | good, with React examples | patchy across v5→v7 (v7 changed the default origin to centre) | strong for rendering, nothing editor-specific |
| Testability | node tree queryable (`stage.find`), runs in Node with `canvas` | object list queryable, runs in Node | WebGL only; needs a browser for anything visual |

## Measured limits (Edge/WebView2 157)

| Limit | Value | Source |
|---|---|---|
| 2D canvas area | **268,435,456 px** (16384² works, 16385×16384 fails, 20000×13422 fails) | measured; Chromium `kMaxCanvasArea = 32768 * 8192` in [canvas_rendering_context_host.cc](https://chromium.googlesource.com/chromium/src/third_party/+/master/blink/renderer/core/html/canvas/canvas_rendering_context_host.cc) |
| 2D canvas side | **65535 px** (65536 fails) | measured; Chromium `kMaxSkiaDim`, raised from 32767 ([issue 40349850](https://issues.chromium.org/issues/40349850)) |
| WebGL2 `MAX_TEXTURE_SIZE` / renderbuffer | 16384 / 16384 (iGPU and dGPU, ANGLE D3D11) | measured |
| WebGPU `maxTextureDimension2D` | 16384 | measured |
| Custom URI scheme URL on Windows | `http://<scheme>.localhost/<path>` | [Tauri `Builder` docs](https://docs.rs/tauri/latest/tauri/struct.Builder.html) |

A canvas over these limits fails silently: drawing does nothing ([MDN](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/canvas)). A 16k map is exactly at the area limit, so **no full-image canvas anywhere in the UI.** Flatten and export belong in Rust (KQ-25).

## Risks

- **One machine, DPR 1.** A 150–200 % display scale means 2.25–4× the pixels to fill and more tiles per view. That's untested. Expect it to hold given the 4× headroom measured here.
- **Konva redraws a whole layer on every change.** That's fine at ~20 visible tiles. Keep tiles, patches, mask and UI on separate layers so that a mask stroke doesn't redraw the tiles.
- **The preview is not the export.** Canvas bilinear blending of a rotated or scaled patch only approximates the Rust composite, so golden-image tests are needed (KQ-25).
- **Toolchain.** Current Tauri crates (2.12) need rustc ≥ 1.90. The spike resolved to Tauri 2.11.6 with Cargo's MSRV-aware resolver instead of upgrading the global toolchain. The stack ADR should pin a toolchain.
- **Flaky bench.** One perf scenario (Konva, single 8k) failed once and passed on rerun. The cause wasn't captured because the log was lost. It's spike-only, not CI.

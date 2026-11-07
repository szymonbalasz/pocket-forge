# Gate: phase 1a (stack) + kickoff leftovers

> **Closed 2026-11-07.** D1 yes, D2 approved, rest accepted (#31–#35). OQ-IMG-12 is still with the owner.

One doc, all open calls. Answer **D1–D2** and say "accept the rest", or name the exceptions.

## Summaries

| Item | Summary | Verdict |
|---|---|---|
| [Canvas research](../research/canvas-library.md) → [proposed ADR](../architecture/adr/0003-canvas-library.md) | Konva + react-konva over a 512 px **tile pyramid** that Rust serves through a custom URI scheme. With tiles, all three libraries pan a 16k map at ~240 fps. Konva wins on mask painting (228 vs 139 fps for Pixi), built-in transform handles and bundle size. The Tauri window matched the browser. | Sound. Measured on AC power after discarding battery-capped runs. |
| [Imaging research](../research/rust-imaging.md) → [proposed ADR](../architecture/adr/0004-large-image-strategy.md) | Pure-Rust crates, plus libwebp (C) for lossy and animated WebP. 16k export composed in 512-row strips (PNG: 13 s, 388 MiB peak). A 48 px feather + colour match removes seams. Regions must fit the provider's maximum input, or the patch looks softer. Output is deterministic. | Sound. Typical maps (#30) are ~11× cheaper. macOS build unverified. |
| Toolchain | Current Tauri and crates need **Rust ≥ 1.90**. This machine has 1.86, and the spikes worked around it with older versions. | Needs D1 |

## Decisions

**D1: Desktop stack ADR** (KQ-30, KQ-31, KQ-32, OQ-IMG-11)

| Option | Tradeoff |
|---|---|
| **A. Tauri 2 + Rust core (pinned stable ≥ 1.90 via `rust-toolchain.toml`) + React + TypeScript + Vite + pnpm (via corepack) + Konva** | What the spikes proved. Pinning means `rustup` installs the pinned toolchain on your machine automatically. |
| B. Same, but stay on Rust 1.86 with older crates | No toolchain change, but older Tauri, `image` and `fast_image_resize`, and it drifts further over time. |

**D2: Production dependencies** (OQ-IMG-01, AGENTS §3.5)

| Option | Tradeoff |
|---|---|
| **A. Approve: `image`, `png`, `jpeg-encoder`, `fast_image_resize`, `tiny-skia`, `rayon`, `libwebp-sys` 0.14 (C, builds with the MSVC toolchain), `konva`, `react-konva`, `reqwest` (already #21); `gif` + `color_quant` later, with animation** | Meets lossy/animated WebP (#25). libwebp 1.6.0, not the CVE-affected 1.3.1. |
| B. Pure Rust only | Animated WebP is lossless-only, about 25× larger (32 MiB vs 1.2 MiB per 48-frame token). |

## Accept the remaining picks as written

| ID | Pick |
|---|---|
| OQ-CNV-01 / OQ-IMG-02 | Tiles: JPEG q90 for downsampled levels, lossless WebP at the 1:1 level |
| OQ-CNV-02 | Build the pyramid in the background on import/open; show the coarse level first |
| OQ-CNV-03 | Keep WebView2's default GPU (don't force the discrete GPU) |
| OQ-CNV-04 | Maximum region/mask 4096² |
| OQ-CNV-05 | Mask undo = per-stroke bounding-box snapshots, with a memory cap |
| OQ-CNV-06 | react-konva for structure; tiles and mask drawn imperatively |
| OQ-CNV-07 | Render at device pixels (sharp at 150–200 % scaling) |
| OQ-CNV-08 / OQ-IMG-10 | Glossary: add **Tile pyramid**, **View**, **Feather** |
| OQ-IMG-03 | PNG export uses `Fast` compression |
| OQ-IMG-04 | WebP export capped at 16383 px; a failed lossy encode retries lossless, with a notice |
| OQ-IMG-05 | Decode once per open project (no memory-mapped cache) |
| OQ-IMG-06 | A region larger than the provider's maximum input is allowed, with a "patch N× softer" warning |
| OQ-IMG-07 | Golden images compare decoded pixels (exact per platform, ±1 across platforms) |
| OQ-IMG-08 | Treat everything as sRGB; ignore ICC profiles at import |
| OQ-IMG-09 | GIF palette: `color_quant`, no dither (when animation lands) |
| OQ-IMG-13 | Export at another px per square: flatten at source size, then Lanczos3 resize |
| KQ-23 | Seams: inward feather + colour match, baked into the patch layer by Rust |
| KQ-25 | Rust is the authority for flatten/export; the canvas approximates; golden-image tests |
| KQ-33 | E2E: Playwright on the UI with mocked Rust commands for most flows; a few Tauri smoke tests on Windows CI |
| KQ-34 | CI: Rust core + UI tests on Linux runners, the app build + smoke on `windows-latest`, macOS later |
| **Kickoff batch** | KQ-05 Credential Manager · KQ-13 one machine at a time + conflict-copy warning · KQ-16 per-session undo · KQ-17 record everything per job · KQ-22 generated fixtures only · KQ-24 fake provider in CI + one manual real call · KQ-28 PNG/JPEG/WebP input · KQ-35 unsigned, no auto-update · KQ-36 no telemetry · KQ-38 Claude workers, max 4 |
| **Success criteria** | No visible seam at 100 % · typical map usable < 2 s (16k < 5 s) · mask → accepted patch < 5 min of owner time (excluding generation) · save/reopen is pixel-identical |

## Needs you, not urgent

- **OQ-IMG-12:** file the `image-webp` 16384 px decode bug upstream? This is outward-facing. Either you file it, or you OK an agent filing it.

## Resolved by the integrator

- Spike code stays under `spikes/` as reference and is deleted when the skeleton lands.

## Next after your answers

1. Number and accept ADRs 0002 (desktop stack), 0003 (canvas), 0004 (large-image strategy). Update the glossary and decisions log, and add `rust-toolchain.toml`.
2. **Phase 1b:** I draft AGENTS §9 build conventions. In parallel, spec agents write the foundation specs for the milestone loop: asset project format, editor canvas, region edit and patch, provider adapter, export. Then the review → your spec gate.

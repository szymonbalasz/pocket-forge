# Canvas spike (throwaway)

Prototype for KQ-32: Konva vs Fabric.js vs PixiJS on 8k/16k battlemaps, in Chromium (Edge = WebView2's engine) and a Tauri 2 window.
Findings: [docs/research/canvas-library.md](../../docs/research/canvas-library.md). Proposal: [proposed-canvas-library.md](../../docs/architecture/adr/proposed-canvas-library.md).
**Not production code.** Dependencies here live on the spike branch only (#22).

## Layout

| Path | What |
|---|---|
| `scripts/gen-fixtures.mjs` | Procedural synthetic battlemap (grid, rooms, flagstones, noise), 8k and 16k, plus a 512 px tile pyramid and a patch PNG. Output goes to `public/fixtures/` (gitignored). |
| `src/core/` | Library-agnostic parts: `view.ts` (the one view transform), `tiles.ts` (tile pyramid manager), `mask.ts` (mask storage + soft brush/eraser). |
| `src/adapters/` | One thin adapter per library (`konva.ts`, `fabric.ts`, `pixi.ts`) behind `types.ts`. |
| `src/bench.ts` | In-page benchmarks (`window.bench`): limits, load, pan/zoom, mask, coordinates, layers, Tauri tile transfer. |
| `src/main.tsx` | Manual playground (pick library/mode/size; pan, mask and layer tools) and the Tauri autorun. |
| `tests/` | Playwright: `view` (pure math), `coords`, `layers`, `limits`, `smoke`, `perf`, `mask`; diagnostics `stall` (long-frame timeline) and `upload` (texture upload cost). |
| `scripts/proc-mem.ps1` | Windows GPU/process memory of the test browser (perf counters). |
| `scripts/summarize.mjs` | `results/raw/*.json` → `results/summary.md`. |
| `src-tauri/` | Minimal Tauri 2 shell: `tiles://` custom URI scheme vs `tile_b64` / `tile_raw` IPC commands. |

## Run

Prerequisites: Node 24 (corepack), Microsoft Edge (Playwright uses `channel: 'msedge'`), Rust for the Tauri part.

```sh
cd spikes/canvas
corepack pnpm install
corepack pnpm fixtures            # ~30 s; writes public/fixtures/ (8k + 16k, ~140 MB)
corepack pnpm dev                 # keep running (port 5174); open it for the manual playground
corepack pnpm exec playwright test view coords layers limits   # correctness + limits (~1 min)
corepack pnpm exec playwright test perf mask                    # benchmarks (~15 min, opens Edge windows)
GPU=dgpu corepack pnpm exec playwright test perf                # same on the discrete GPU (hybrid laptops)
node scripts/summarize.mjs        # → results/summary.md
```

Benches open headed Edge windows (a hidden window throttles rendering); don't cover them while they run.
**Run on AC power**: on battery, Windows/Edge capped every library at 30 fps.
Benches run with vsync on (realistic). `UNCAPPED=1` turns vsync and the frame cap off; with WebGL that causes back-pressure stalls, so those numbers aren't comparable.
Env filters: `LIBS=konva,pixi`, `MODES=tiles`, `SIZES=16384`, `GPU=dgpu`.

### Tauri smoke

With `corepack pnpm dev` running:

```sh
SPIKE_QUERY="autorun=tauri&lib=konva" corepack pnpm tauri dev   # PowerShell: $env:SPIKE_QUERY="autorun=tauri&lib=konva"
```

The window runs the transfer, tile, pan/zoom and mask benches, writes `results/raw/tauri-konva.json` and exits.
Without `SPIKE_QUERY` it opens the manual playground. `SPIKE_BROWSER_ARGS` passes extra WebView2 flags (e.g. `--force_high_performance_gpu`).
Current Tauri crates need rustc ≥ 1.90; `src-tauri/.cargo/config.toml` + `rust-version = "1.86"` make Cargo pick Tauri 2.11.x that builds on rustc 1.86 instead of upgrading the global toolchain.

`node scripts/bundle-size.mjs` prints the tree-shaken size of each adapter.

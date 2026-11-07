# ADR 0002: Desktop stack

> Status: **accepted** 2026-11-07 (#31). Input: KQ-30, KQ-31, KQ-32, OQ-IMG-11, the [canvas research](../../research/canvas-library.md), the [imaging research](../../research/rust-imaging.md), #2, #3, #9.

## Context

- An installed desktop app, Windows first and macOS later (#2). No external browser and no separately managed localhost server (#3).
- Image processing has to be fast and deterministic (KQ-25, [ADR 0004](0004-large-image-strategy.md)), and the UI needs a rich interactive canvas ([ADR 0003](0003-canvas-library.md)).
- The spikes proved the combination below end to end in a Tauri window on Windows.

## Decision

| Layer | Choice |
|---|---|
| Desktop shell | **Tauri 2** (WebView2 on Windows, WKWebView on macOS later) |
| Core logic and image processing | **Rust**. The toolchain is pinned in `rust-toolchain.toml` (stable, ≥ 1.90) |
| UI | **React + TypeScript + Vite** |
| Package manager | **pnpm**, run via corepack (`corepack pnpm …`); no global install |
| Canvas | **Konva + react-konva** ([ADR 0003](0003-canvas-library.md)) |
| Provider access | Rust REST client ([ADR 0001](0001-provider-integration.md)) |
| Tiles to UI | A Tauri custom URI scheme served by the Rust core ([ADR 0003](0003-canvas-library.md)) |

## Principles

- **Every capability is a Rust core command.** The UI is a client of those commands and holds no domain logic, so automation (including a future MCP endpoint, #36) can drive the same commands.
- Deterministic seams (IDs, clock, randomness, provider) are injected, so tests and fixtures are reproducible.

## Consequences

- Changing the toolchain version is a one-line change to `rust-toolchain.toml`, and `rustup` installs it automatically.
- Approved dependencies are listed in #32. Anything new still needs the owner's OK (AGENTS §3.5).
- AGENTS §9 (build conventions) builds on this ADR.

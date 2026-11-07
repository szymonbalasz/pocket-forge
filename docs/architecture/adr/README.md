# Architecture decision records

**Accepted ADRs are law.** Change one only through a superseding ADR, and only the owner accepts.
Agents draft `proposed-<slug>.md`. The orchestrator numbers an ADR (`NNNN-<slug>.md`) when it is accepted.

| # | Area | Status | Input |
|---|---|---|---|
| [0001](0001-provider-integration.md) | Provider integration: REST adapter in the Rust core | **accepted** | [research](../../research/scenario-api.md), KQ-04 |
| [0002](0002-desktop-stack.md) | Desktop stack: Tauri 2, Rust, React + TS + Vite, pnpm, Konva | **accepted** | KQ-30–32 |
| [0003](0003-canvas-library.md) | Canvas: Konva over a Rust-served tile pyramid | **accepted** | [research](../../research/canvas-library.md) |
| [0004](0004-large-image-strategy.md) | Large-image strategy: tiles, regions, patch blend, strip export, animated export | **accepted** | [research](../../research/rust-imaging.md) |
| — | Asset project format and library storage | planned (phase 1b spec) | #13, #16, #18, #19 |
| — | Secrets (provider credentials) | planned | KQ-05 (Credential Manager) |
| — | Test strategy and CI | planned (AGENTS §9) | KQ-24, KQ-33, KQ-34 |

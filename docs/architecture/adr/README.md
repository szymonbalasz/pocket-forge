# Architecture decision records

**Accepted ADRs are law.** Change one only through a superseding ADR, and only the owner accepts.
Agents draft `proposed-<slug>.md`. The orchestrator numbers an ADR (`NNNN-<slug>.md`) when it is accepted.

| # | Area | Status | Input |
|---|---|---|---|
| — | Desktop shell (Tauri) and Rust core | planned | provisional direction |
| — | Frontend framework | planned | KQ-30 |
| — | Canvas library | [proposed](proposed-canvas-library.md) | `spike/canvas-large-image`, [research](../../research/canvas-library.md) |
| — | Asset project format and library storage | planned | KQ-10 to KQ-15 |
| [0001](0001-provider-integration.md) | Provider integration: REST adapter in the Rust core | **accepted** | [research](../../research/scenario-api.md), KQ-04 |
| — | [Large-image strategy](proposed-large-image-strategy.md) (tiles, regions, patch blend, flatten, animated export) | **proposed** | [research](../../research/rust-imaging.md), `spike/rust-imaging` |
| — | Secrets (provider credentials) | planned | KQ-05 |
| — | Test strategy and CI | planned | KQ-33 |

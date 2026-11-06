# Decisions log

Append-only. Cite decisions as `#N`. Changing a decision takes a new row that supersedes the old one.

| # | Decision | Rationale | Source / ADR |
|---|---|---|---|
| 1 | Public repository from day one. No personal or machine-specific content is ever committed. | Open project; safe to share | Owner, kickoff |
| 2 | Windows first; macOS later. Keep code portable. | Owner's platform first | Owner, kickoff |
| 3 | An installed desktop app. Normal use needs no external browser and no separately managed localhost server. The UI may use web tech inside the window. | Simple, self-contained UX | Owner, kickoff |
| 4 | Primary asset types: battlemaps, tokens, item art. Cards are a possible extension. | Session-prep needs | Owner, kickoff |
| 5 | The library is ordinary local folders, including cloud-synced ones. | The user owns plain files | Owner, kickoff |
| 6 | Scenario is the generation provider for now. | Selected by owner | Owner, kickoff |
| 7 | Specs before code. One orchestrator, with fresh worker agents per task in their own worktrees. Merge only on green CI. | Proven agent workflow | Owner, kickoff; [process.md](process.md) |
| 8 | Built for the owner's own use. Others may install it, but their needs don't drive the design. | Personal tool; no polish for other users | Owner, KQ-01 |
| 9 | No AI/LLM features in the app. The only AI is the generation provider. Prompts are assembled from recipes and edited by the user. | Owner preference; control and predictability | Owner, KQ-03 |
| 10 | No existing tool workflow to carry over. The starting material is a prompt-helper doc, reference images and ad-hoc chat-based generation. Art recipes are seeded from that by hand. | Nothing to stay compatible with | Owner, KQ-07 |
| 11 | Size targets: battlemaps of 5–10k px per side typical, up to about 16k; 70–140 px per grid square. | Sets the canvas, tiling and export budgets | Owner, KQ-20 |
| 12 | No integration with external notes or vault tools. Pocket Forge starts with a fresh, standalone library. | Keep scope tight; no import paths to maintain | Owner, KQ-18 |
| 13 | The user configures the library folder (expected to be cloud-synced, e.g. OneDrive). Asset projects are plain folders and files, never single-file packages or databases. Expect a very large number of images. | Sync-friendly, recoverable with normal file tools, scales to large libraries | Owner, KQ-10 |

The tech stack (Tauri, Rust, a TS web UI, the canvas library, Scenario via MCP) is **provisional**. It is not logged until the stack ADRs are accepted.

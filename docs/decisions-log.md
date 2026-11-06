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

The tech stack (Tauri, Rust, a TS web UI, the canvas library, Scenario via MCP) is **provisional**. It is not logged until the stack ADRs are accepted.

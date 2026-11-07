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
| 14 | Provider integration: the app calls Scenario's REST API from the Rust core through a provider-agnostic adapter. MCP and the TS SDK are not used in the app. | Full coverage, stable, secret stays in the core, easy to fake in tests | Owner; [ADR 0001](architecture/adr/0001-provider-integration.md) |
| 15 | Masked edits are offered for the models Scenario's guidance documents as supporting them. The per-model mask convention follows Scenario's docs. | Follow the provider's guidance, not our own allowlist | Owner, OQ-PRV-01 |
| 16 | Originals outside the library are never modified. The app works only inside the configured library folder. Bringing images in (copying them into the library) must be as frictionless as possible. | Safety of originals, one synced home | Owner, KQ-11 |
| 17 | Licence: MIT. Pintura is dropped (its licence key can't live in a public repo). | Open source; no commercial SDK | Owner, KQ-02 |
| 18 | Retention: accepted revisions and starred candidates are kept forever. Unstarred candidates go to a project trash that is emptied after 30 days. Per-project disk usage is visible. | Bounded OneDrive growth without losing anything chosen | Owner, KQ-15 |
| 19 | Library index (thumbnails, search) is a local, rebuildable cache outside the synced folder. Tags and collections live in synced files. The app never forces downloads of cloud-only files for thumbnails. | Avoid database corruption on sync; respect files-on-demand | Default accepted, KQ-12/KQ-14 |
| 20 | Local-first provider use: uploads are automatic and invisible. The local asset project is the source of truth, and provider-side assets are a disposable cache that is re-uploaded when missing. The user never manages provider profile state. | Work locally; don't depend on provider state | Owner, OQ-PRV-03 |
| 21 | ADR 0001's remaining picks are accepted as written (OQ-PRV-02, 04–10), including the `reqwest` HTTP client dependency and the glossary term "Cost". | Implementation details | Owner |
| 22 | Canvas and Rust-imaging spikes approved. Throwaway dependencies are allowed on spike branches only; the spikes trial React + TS + Vite and pnpm. | De-risk the provisional stack before ADRs | Owner |
| 23 | Spend guardrails: every job shows its free pre-flight cost estimate. There is a per-session spend cap, plus an extra confirmation for any single job above a user-set threshold. | Visible, bounded cost | Owner, KQ-06 |
| 24 | The export target is the owner's own custom VTT, not a third-party VTT. | Owner controls the import side | Owner, KQ-21 |
| 25 | Animated output is required: **animated WebP**, and possibly GIF. The source of the motion and the tier are still open (KQ-39). | Animated tokens and map elements | Owner |
| 26 | Export is plain image files, with no metadata sidecar. The owner sets the grid size in their VTT, whose size limits are generous. The app exports at a user-chosen px per square. | Simple; the VTT owns grid config | Owner, KQ-29 |
| 27 | Animation covers every asset type eventually (tokens, map elements, whole maps, item art). It comes after the first milestone, **tokens first**. Motion source still open (KQ-39). | Static loop first; animation is the next big step | Owner, KQ-39 |
| 28 | Merging is local: branches are pushed and CI runs on every branch push. The orchestrator merges with `--no-ff` once the branch is green. No PRs required, no branch protection: a single maintainer using common sense. | Simple single-maintainer flow | Owner, KQ-37 |
| 29 | Feature tiers accepted as proposed. | Owner review | Owner |

The tech stack (Tauri, Rust, a TS web UI and the canvas library) is **provisional**. It is not logged until the stack ADRs are accepted.

# Gate: phase 1b (foundation specs + build conventions)

Answer **D1–D3** and say "accept the rest", or name the exceptions. After your answers, a revision round applies your decisions plus the [integrator rulings](rulings-phase1b.md) to the specs. Then I bring them back for you to mark `complete`.

## Summaries

| Spec | Summary | Review verdict |
|---|---|---|
| [asset-project](../specs/asset-project.md) (38 FRs) | `<file>.pforge/` next to the original. Every change commits atomically (the manifest rename is the commit point). Write-once revisions, layers, masks and candidates. Imports go to `Imported/`. Trash with a 30-day purge. Conflict-copy detection. Versioned manifest. | Solid core. Owns the schema, so most cross-spec fixes land here (R2, R8–R13). |
| [editor-canvas](../specs/editor-canvas.md) (28) | View and tile URI contract, mask brush/eraser/hardness, mask hand-off, per-stroke undo, before/after compare, candidate picker. | Good. Needs the region tool (R18), a transient preview path (R16), the inner-rect mask rule (R1). |
| [region-edit](../specs/region-edit.md) (32) | Region → fit to the model's maximum input → final prompt → estimate → confirm/spend cap → job → candidates → accept as a feathered, colour-matched patch → revision. Resumes jobs on reopen. | The core of the MVP. Job states and the mask size clash with the other specs (R1, R2). |
| [provider](../specs/provider.md) (28) | The ADR 0001 trait, the Scenario REST implementation, a deterministic fake provider, credentials in the OS store, capability normaliser plus overlay, automatic uploads, recording format, a 3-lock real-call mode. | Strong. Needs more fake cases (R17) and upload ownership (R15). |
| [export](../specs/export.md) (27) | One Rust compositor (shared with region-edit), strip export, px-per-square scaling (up to 4×), PNG/JPEG/WebP with limits and fallbacks, `Exports/` folder, progress and cancel. | Good. Should compose the current layers, not the head revision (R6). |
| [AGENTS §9](../../AGENTS.md#9-build-conventions) | Layout (`pf-core`, `pf-imaging`, `pf-provider`, `app/`), per-worktree data dirs and ports, commands, test naming, shared-files table, "done". | Add R22. |

All five specs are over the five-minute target (168–200 lines). R26 moves the schema tables into `docs/architecture/manifest-schema.md`.

## Decisions

**D1: Model controls in the MVP** (both reviewers flagged this as tier-A creep)

| Option | Tradeoff |
|---|---|
| **A. The MVP uses a fixed control set (prompt, candidate count, seed), plus only what's needed from capabilities: mask support, input limits, seed support. Fully capability-driven controls stay tier A.** | Smaller MVP. The normaliser is built partially now and finished later. |
| B. Promote "controls reflect the model" to tier S | Honest controls from day one, but more UI and test work before the first loop works. |

**D2: New dependencies** (AGENTS §3.5)

| Option | Tradeoff |
|---|---|
| **A. Approve `keyring` (OS credential store: Windows now, macOS later) and `sha2` (content hashes for import dedupe and cache keys). Plus `tokio`, `serde`/`serde_json` and `base64`, which mostly arrive with Tauri and reqwest anyway.** | Standard, widely used crates |
| B. Approve with changes | — |

**D3: Sync conflicts in the MVP** (KQ-13: one machine at a time)

| Option | Tradeoff |
|---|---|
| **A. Simple: detect the conflict, show both versions, you pick one, and the other becomes a revision (nothing lost). No record-by-record merge.** | Much less code. It fits "one machine at a time". |
| B. Merge records by id (as specced) | Automatic, but complex and rarely needed |

## Accept the remaining picks as written

| ID | Pick |
|---|---|
| OQ-PRJ-01 | Project folder `<file>.pforge/` next to the original |
| OQ-PRJ-02 | Imports go to `<library-root>/Imported/` |
| OQ-PRJ-04 | Every change saves immediately. Drafts save after 2 s idle. Revisions on accept, Ctrl+S, restore, re-tune, conflict |
| OQ-PRJ-07 + scope | Asset-project owns a minimal revision list, a recent-projects list and "relink original" (small, needed for save/reopen) |
| OQ-EXP-01 | Exports go to `<library-root>/Exports/`, mirroring the original's folder |
| OQ-EXP-04 | Export settings saved per asset project |
| OQ-EXP-05/06 | Upscale up to 4×; fixed quality (q90) |
| OQ-RGN-02 | The region is cut from what you see (the original plus accepted patches) |
| OQ-RGN-04 | No default spend cap. The app asks for a cap and a threshold on your first submit |
| OQ-RGN-06 | One active job per asset project |
| OQ-RGN-08 | Feather and colour match can be re-tuned after accept (as a new revision) |
| OQ-RGN-10 | A crash between submit and saving the job → marked "lost", with a "you may have been charged" notice |
| OQ-PRV-20 | Release builds use the real Scenario provider; dev and test builds use the fake |
| OQ-EDT-11 | Zoom and pan aren't saved with the project (no sync churn) |
| All other OQ picks in the five specs | As written, unless overridden by a ruling |
| AGENTS §9 | Approve the draft plus R22 |
| Integrator rulings R1–R27 | As written |

## Needs you, not urgent
- **MCP order (#36):** first after the MVP, or after animated tokens? (My pick: first.)
- **OQ-IMG-12:** file the `image-webp` 16384 px bug upstream?

## Next after your answers
1. Revision round: one agent per spec applies your decisions and the rulings. The asset-project agent also writes `manifest-schema.md`. I update the glossary and §9.
2. A quick consistency re-check, then you set the specs to `complete`.
3. **Phase 2 skeleton:** one builder sets up the workspace, Tauri shell, core command plumbing, project load/save, the fake provider, the test harness and the CI jobs (Linux plus Windows, split evenly).

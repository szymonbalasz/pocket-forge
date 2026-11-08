# Status (scratch board, overwritten as things change)

**Phase:** 3, wave 3 (agent cap 8, #50). Specs complete, including reference packs (#54).

| Task | Agent | Worktree / branch | State | Blockers |
|---|---|---|---|---|
| Specs: asset-project, editor-canvas, region-edit, provider, export, reference-packs | — | merged | complete (#54) | — |
| Skeleton; provider 2–4; editor-canvas 1–4, 6; export 1–2, 4; region-edit 1–2, 6 | — | merged | done | provider slice 5 needs the owner |
| Asset-project core + UI (slices 1–4) | — | merged | done | walkthrough pending |
| Native file dialog (#53) | — | merged | done | — |
| Backlog: BL-04/06/07/09/11 | `backlog` | `fix/backlog-1` | BL-07 in-app smoke (R46) | — |
| Region-edit jobs (RGN 3–5) | — | merged | done | walkthrough needs fake job persistence (R48) |
| Export commands (EXP 3, 5) | `exp-cmd` | `build/export-commands` | in progress | — |
| Editor mask hand-off (EDT 5 + BL-10/14/16) | `edt-mask` | `build/editor-mask-handoff` | in progress | — |
| Asset-project retention + conflicts (PRJ 5–6) | `prj-ret` | `build/asset-project-retention` | in progress | — |
| Reference packs core (REF 1, 4) | `ref-core` | `build/reference-packs-core` | in progress | — |
| Provider references (REF 2) | — | merged | done | — |
| Backlog: BL-12/15/19/20 | `backlog2` | `fix/backlog-2` | in progress | — |
| Packs UI + References block (REF 3, 5) | `ref-ui` | `build/reference-packs-ui` | in progress | — |
| Asset-project walkthrough (steps 1–6, 9) | `walk-prj` | `test/prj-walk` | in progress | — |
| Later: REF 6 (RGN integration) | — | — | waiting | REF 4, 5 and RGN 3 merged |

## Pending owner decisions
- OQ-IMG-12: file the image-webp bug upstream? (not urgent)
- MCP endpoint order within tier A (#36)

## Integrator rulings (since last gate)
- R32–R46 (see `rulings-phase1b.md`)

## Shared-file locks
- _none_

## Reserved FR ranges
- _none_

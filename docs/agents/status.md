# Status (scratch board, overwritten as things change)

**Phase:** 3, wave 3 (agent cap 8, #50). Specs complete, including reference packs (#54).

| Task | Agent | Worktree / branch | State | Blockers |
|---|---|---|---|---|
| Specs: asset-project, editor-canvas, region-edit, provider, export, reference-packs | — | merged | complete (#54) | — |
| Skeleton; provider 2–4; editor-canvas 1–4, 6; export 1–2, 4; region-edit 1–2, 6 | — | merged | done | provider slice 5 needs the owner |
| Asset-project core + UI (slices 1–4) | — | merged | done | walkthrough pending |
| Native file dialog (#53) | — | merged | done | — |
| Region-edit jobs (RGN 3–5) | — | merged | done | walkthrough needs fake job persistence (R48) |
| Export commands (EXP 3, 5) | — | merged | done | — |
| Export walkthrough (steps 1–2, 6–12) | `walk-exp` | `test/exp-walk` | in progress | steps 3–5 need EDT 5 |
| Editor mask hand-off (EDT 5 + BL-10/14/16) | — | merged | done | — |
| Asset-project retention + conflicts (PRJ 5–6) | — | merged | done | walkthrough steps 7–8 pending |
| Reference packs core (REF 1, 4) | — | merged | done | — |
| Provider references (REF 2) | — | merged | done | — |
| Packs UI + References block (REF 3, 5) | — | merged | done | — |
| Asset-project walkthrough (steps 1–6, 9) | — | — | passed with defects (spinner race, draft rewrite, rename-while-open) | — |
| Walkthrough fixes (spinner, draft) | — | merged | done | — |
| Asset-project re-walk | — | — | spinner fixed (44 opens); N1–N4 found | — |
| Re-walk fixes N1–N4 | `fix-prj2` | `fix/walk-prj-2` | in progress | — |
| REF 6 (RGN integration) | — | merged | done | — |
| Generate-loop walkthrough (RGN, EDT 5, PRV fake, EXP 3–5) | `walk-rgn` | `test/rgn-walk` | in progress | — |
| Reference packs walkthrough | `walk-ref` | `test/ref-walk` | in progress | — |

## Pending owner decisions
- OQ-IMG-12: file the image-webp bug upstream? (not urgent)
- MCP endpoint order within tier A (#36)

## Integrator rulings (since last gate)
- R32–R46 (see `rulings-phase1b.md`)

## Shared-file locks
- _none_

## Reserved FR ranges
- _none_

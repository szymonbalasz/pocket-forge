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
| Export walkthrough (steps 1–2, 6–12) | — | — | passed; D1–D5 UX | steps 3–5 in the RGN walk |
| Export walk fixes D1–D5 | — | merged | done | — |
| Editor mask hand-off (EDT 5 + BL-10/14/16) | — | merged | done | — |
| Asset-project retention + conflicts (PRJ 5–6) | — | merged | done | walkthrough steps 7–8 pending |
| Reference packs core (REF 1, 4) | — | merged | done | — |
| Provider references (REF 2) | — | merged | done | — |
| Packs UI + References block (REF 3, 5) | — | merged | done | — |
| Asset-project walkthrough (steps 1–6, 9) | — | — | passed with defects (spinner race, draft rewrite, rename-while-open) | — |
| Walkthrough fixes (spinner, draft) | — | merged | done | — |
| Asset-project re-walk | — | — | spinner fixed (44 opens); N1–N4 found | — |
| Re-walk fixes N1–N4 | — | merged | done | — |
| REF 6 (RGN integration) | — | merged | done | — |
| Generate-loop walkthrough (RGN, EDT 5, PRV fake, EXP 3–5) | — | — | passed with defects D1–D7 | — |
| RGN walk fixes D1–D7 + BL-34 | — | merged | done | — |
| **Provider real calls (PRV 5)** | `prv-real` (kept for the session) | merged | owner's first real edit works (upload `entityId` fix merged) | drafting PRV gate amendments |
| Reference packs walkthrough | — | — | passed with defects R1–R7 | — |
| REF walk fixes R1–R7 | — | merged | done | BL-34 open |
| Single-output cap, empty-mask hint, Mask whole region, blend instruction (#55) | — | merged | done (R72, R73); walkthrough PASS; fixes merged (R74); BL-38/39 done | — |
| Keep every result, easy discard (#56) | — | merged | done (R65–R67) | walkthrough with rgn-ux |
| Backlog BL-30/31/32/35/36, BL-27 StrictMode | — | merged | done (R69, R70) | — |
| Seam asymmetry (BL-33) | — | merged | done: no asymmetry; smoothed metric (R68) | FR-RGN-32 / EXP step 5 wording at the gate |
| Conflict view draws layer stacks (BL-22) | — | merged | done (R71) | walkthrough step 7 later |

## Pending owner decisions
- OQ-IMG-12: file the image-webp bug upstream? (not urgent)
- MCP endpoint order within tier A (#36)

## Integrator rulings (since last gate)
- R32–R46 (see `rulings-phase1b.md`)

## Shared-file locks
- _none_

## Reserved FR ranges
- _none_

## Phase 4 (#58, #59)

| Task | Agent | Branch state | Result | Next |
|---|---|---|---|---|
| Spec: new-asset generation (GEN) | — | merged | complete (#60) | — |
| Spec: prompt helper (PRM) | — | merged | complete (#60) | — |
| GEN 1: PRV `generate` op, fake gen models | `prv-real` | building | — | owner: free dry-run-generate recordings |
| GEN 2: image-less project, base layer, pick commit | `gen-2` | building | — | — |
| GEN 5: + New menu, New image tab (mocked) | `gen-5` | building | — | — |
| PRM 0: `pf-chat` crate + probe | `prm-0` | building | — | owner runs the probe |
| PRM 1: helpers core | `prm-1` | building | — | — |

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
| GEN 1: PRV `generate` op, fake gen models | — | merged | done (R76) | owner: free dry-run-generate recordings |
| GEN 2: image-less project, base layer, pick commit | — | merged | done (R79) | — |
| GEN 3: core generate jobs | — | merged | done (R80) | — |
| GEN 4: preview, pick, re-pick, undo | — | merged | done (R82) | — |
| GEN 6: prompt block in New image, GenContextSource | — | merged | done (R83) | — |
| GEN walkthrough (fake) + fixes | — | merged | PASS with defects; all fixed (R84, R85) | BL-40 done (R86); owner: dry-run-generate, 2 paid checks |
| GEN 5: + New menu, New image tab (mocked) | — | merged | done (R77) | slice 4 picker routing, slice 6 prompt block |
| PRM 0: `pf-chat` crate + probe | — | merged | done; owner probe passed (R87, R88; min 2.1.319) | — |
| PRM 4: pf-chat adapter | — | merged | done (R89) | — |
| PRM 5: chat core | — | merged | done (R90) | — |
| PRM 6: chat window + Settings login | — | merged | done (R92); BL-41/42 (R93); walkthrough PASS, fixes merged (R94, R95) | owner: try the real chat |
| PRM 1: helpers core | — | merged | done (R78) | — |
| PRM 2: prompt block + Settings UI (mocked) | — | merged | done | — |
| PRM 3: prompt block in Region edit | — | merged | done (R81) | PRM 4 waits on the owner's probe |

**Phase 4 state (2026-11-09):** GEN slices 1–6 and PRM slices 0–6 merged; both walkthroughs passed and their fixes are merged (R76–R95). No agents running. Owner next: real-provider generation (dry-run-generate recording, 2 cheap paid checks) and the real prompt chat (`corepack pnpm dev:real`).

## Overnight hardening (2026-11-09/10)

| Task | Result |
|---|---|
| Phase 4 security review | 3 med + 5 low; fixed (R96, R99) |
| Phase 4 correctness review | 16 findings; fixed (R97, R98, R100) |
| Phase 4 FR-to-test audit | 4 missing + 5 weak; closed |
| pf-chat Windows hardening | npm `.cmd` shim `%` expansion bug fixed; process tree, error mapping, caps (R99) |
| Old-UI review (phases 1–3) | 12 + 8 unverified; fixed (R101) |
| Old-core review (phases 1–3) | 16 incl. 3 money + 2 data HIGH; fixed (R102, R104–R106) |
| Regression walk (phase 4) | PASS; 6 low fixed (R103) |
| Night walk (all fixes) | PASS; 2 med + 4 low fixed (R107, R108) |
| Review of the night's own fixes | 2 HIGH (X could wedge; X dead after a UI crash) + 5 MED; fixed (R109) |
| Spec + schema sync | specs follow R96–R109 |

**Open owner questions (picks in bold):**
- Chat child environment (FR-PRM-34): **strip `ANTHROPIC_BASE_URL`/`AUTH_TOKEN`, Bedrock/Vertex switches, `NODE_OPTIONS` and warn at detection** · allow-list · leave as is.
- Pre-release of the minimum CLI (`2.1.319-beta`): **fails closed** · passes (FR-PRM-33 "first triple").
- FR-PRM-32 backend switch: **amend the spec to the code** · change the code.
- "Mask saved" status after a pick: **keep (live mask status)** · hide.
- FR-PRM-21 window singleton test needs Tauri's mock runtime (new dev feature, §3.5): **later** · now.
- Glossary candidates: quarantine (spool), Fetch again, unconfirmed submit.
- Confirm or change rulings R96–R108, especially R101 (app quit = close), R102 (lost, never Retry-to-resubmit) and R98 (undo leaves the New image draft alone).
- After a Dismiss of a job whose fetch failed, it leaves the tracker, so `fetch_failed` isn't visible afterwards: **fine** · keep a row.
- A References trash file whose move and rollback both failed stays in the trash folder with no item (logged only): **acceptable** · adopt such files at startup.
- Chat "busy in another project" is UI-only; a core `other_turn_running` field would also cover the first second: **later** · now.

# Status (scratch board, overwritten as things change)

**Phase:** 3, wave 1 (agent cap 6, #47). Skeleton merged.

| Task | Agent | Worktree / branch | State | Blockers |
|---|---|---|---|---|
| Specs: asset-project, editor-canvas, region-edit, provider, export | — | merged | complete (#41) | — |
| Skeleton | — | merged | done (walkthrough passed) | — |
| Provider slices 2–4 | — | merged | done (walkthrough after RGN UI) | slice 5 needs the owner |
| Editor-canvas slices 1–4, 6 + UI foundation | `walk-edt` (tester) | merged | walkthrough running | slice 5 waits for PRJ store |
| Export slices 1–2 | — | merged | done | slices 3–5 wait for PRJ store |
| Spec revision: folder model (#44) | — | merged | in-review | owner re-mark + 6 picks |
| Asset-project core + UI builds | — | — | ready | owner approval |
| Region-edit build | — | — | waiting | export slice 1, PRJ store API |

## Pending owner decisions
- OQ-IMG-12: file the image-webp bug upstream? (not urgent)
- MCP endpoint order within tier A (#36)

## Integrator rulings (since last gate)
- R32–R36 (re-pick, corrupt images, original-layer fields, temp sweep, CSP)
- Skeleton build calls: feature registry at `app/src/shell/features.ts`; `ulid` std feature; schema §4.11 revision file; `PF_FAULT=after_temp|after_rename[:<file>]`

## Shared-file locks
- `app/src/shell/{theme.css,Layout.tsx,icons.tsx}`, `app/package.json` (radix-ui), `tauri.conf.json` (CSP, scheme): editor-canvas
- Root `Cargo.toml` (#32 imaging crates): export

## Reserved FR ranges
- _none_

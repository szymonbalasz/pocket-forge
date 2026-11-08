# Status (scratch board, overwritten as things change)

**Phase:** 3, wave 2 (agent cap 6, #47). Specs complete again (#49).

| Task | Agent | Worktree / branch | State | Blockers |
|---|---|---|---|---|
| Specs: asset-project, editor-canvas, region-edit, provider, export | — | merged | complete (#41) | — |
| Skeleton | — | merged | done (walkthrough passed) | — |
| Provider slices 2–4 | — | merged | done (walkthrough after RGN UI) | slice 5 needs the owner |
| Editor-canvas slices 1–4, 6 + UI foundation | — | merged | walkthrough done, D1/D2 fixed | slice 5 waits for PRJ store |
| Export slices 1–2 | — | merged | done | slices 3–5 wait for PRJ store |
| Spec revision: folder model (#44) | — | merged | complete (#49) | — |
| Asset-project core (slices 1–4) | `prj-core` | `build/asset-project-core` | in progress | — |
| Asset-project UI (rail, home, left panel) | `prj-ui` | `build/asset-project-ui` | in progress | — |
| Region-edit core (slices 1–2) | `rgn-core` | `build/region-edit-core` | in progress | — |
| Region-edit UI (slice 6) | `rgn-ui` | `build/region-edit-ui` | in progress | — |
| Next: RGN 3–5, EXP 3–5, EDT 5, PRJ 5–6 | — | — | waiting | PRJ store API (slice 4) |

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

# Status (scratch board, overwritten as things change)

**Phase:** 2, skeleton build. Specs for the milestone loop are complete (#41).

| Task | Agent | Worktree / branch | State | Blockers |
|---|---|---|---|---|
| Specs: asset-project, editor-canvas, region-edit, provider, export | — | merged | complete (#41) | — |
| Skeleton (workspace, plumbing, project minimum, fake provider, harness, CI) | `skeleton` | `build/skeleton` | in progress | — |

## Pending owner decisions
- OQ-IMG-12: file the image-webp bug upstream? (not urgent)
- MCP endpoint order within tier A (#36)

## Integrator rulings (since last gate)
- R32: re-pick keeps the job `closed` (OQ-RGN-13 = B, #41)

## Shared-file locks
- Root `Cargo.toml`, `package.json`, `pnpm-workspace.yaml`, `tauri.conf.json`, `.github/workflows/*`: created by the skeleton builder (orchestrator-delegated)

## Reserved FR ranges
- _none_

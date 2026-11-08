# Backlog

Small items for idle slots (see [process.md](../process.md)). The planned work always gets the next free slot first.

| ID | Item | Size | Files | Avoid while | Status |
|---|---|---|---|---|---|
| BL-01 | Truncated PNG imports and opens (FR-PRJ-06): cheap end-of-file check on import, full refusal on open (R33) | S | `pf-imaging` probe, `pf-core` import | PRJ build | open, PRJ builder |
| BL-02 | Orphan temp files outside project folders (library import folder, data dir) are never swept: cover them in the open-time sweep (R35) | S | `pf-core` atomic write / sweep | PRJ build | open, PRJ builder |
| BL-03 | A missing original shows the raw OS error text on the canvas; map `not_found` to a readable message | XS | `app/src` editor | EDT build | open, EDT builder |
| BL-04 | After `pnpm dev` exits, the PowerShell pane stops accepting input (seen in herdr only) | XS | `scripts/dev.mjs` | — | open |
| BL-05 | A garbage file named `.png` reports `unsupported` instead of `corrupt` | XS | `pf-imaging` probe | PRJ build | open, PRJ builder |
| BL-06 | `fast_image_resize` scratch is allocated infallibly (~400 MiB at 16k): an OOM aborts instead of returning `out_of_memory` | S | `pf-imaging` resize | EXP build | open |
| BL-07 | Tauri smoke doesn't check that a tile loads through the `/t/` scheme | XS | `scripts/tauri-smoke.mjs` | — | open |
| BL-08 | WebView GPU memory ~1.5 GiB at 16k (tiles LRU correct at 384 MiB); measure on the owner's iGPU machine (#48) | S | — | — | open, owner hardware |
| BL-09 | Mask undo 58–60 ms and stroke p95 25 ms at a 3840² mask (budgets 50 / 16.7 ms), indicative only | S | `app/src/features/editor` | EDT fix | open |
| BL-10 | A missing original offers Rebuild (useless) and leaves tools enabled over an empty canvas | XS | `app/src/features/editor` | PRJ relink | open |
| BL-11 | A deleted tile is requested 5× instead of 3 (FR-EDT-12) | XS | tile manager | — | open |
| BL-12 | The provider chip sits on the rail; move it to a status-bar segment (ui-guidelines) | XS | `features/provider`, shell | PRJ UI build | open, PRJ UI builder |
| BL-13 | Flaky: `fr_prj_15_changed_original_opens_read_only` (size+mtime misses a rewrite within one mtime tick) | XS | `pf-core` project | PRJ core build | open, PRJ core builder |
| BL-14 | EDT doesn't redraw the region outline when the margin changes, or when Retry fills in a job's region: add an EDT bridge setter | S | `features/editor` bridge, `features/region-edit` | — | open |
| BL-15 | The right panel is empty on Home: show a hint or collapse it | XS | shell | PRJ UI merge | open |
| BL-16 | EDT `library` keys serve user-folder images at full size: add a `max_side` copy (OQ-PRJ-16 said ≤ 512 px) | XS | `pf-core` tiles.rs | — | open |
| BL-17 | Flaky: `fr_prj_15_changed_original_opens_read_only` depends on an mtime tick (seen by the PRJ UI builder after the racily-clean fix) | XS | `pf-core` project tests | PRJ retention build | open |
| BL-18 | FR-PRJ-21: closing with `unsaved` shows the error but doesn't ask first | XS | `features/project` | PRJ retention build | done (PRJ retention: Close anyway) |
| BL-19 | Conflict view draws each layer stack as text (OQ-PRJ-11 B fallback): EDT has no read-only "draw this layer stack" component yet; swap it into `ConflictView.tsx` panes when EDT exposes one (OQ-PRJ-11 A) | S | `features/editor` export, `features/project/ConflictView.tsx` | EDT build | open |

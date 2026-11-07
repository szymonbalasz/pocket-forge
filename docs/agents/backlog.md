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

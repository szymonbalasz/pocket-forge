# Backlog

Small items for idle slots (see [process.md](../process.md)). The planned work always gets the next free slot first.

| ID | Item | Size | Files | Avoid while | Status |
|---|---|---|---|---|---|
| BL-01 | Truncated PNG imports and opens (FR-PRJ-06): cheap end-of-file check on import, full refusal on open (R33) | S | `pf-imaging` probe, `pf-core` import | PRJ build | done (PRJ core) |
| BL-02 | Orphan temp files outside project folders (library import folder, data dir) are never swept: cover them in the open-time sweep (R35) | S | `pf-core` atomic write / sweep | PRJ build | done (PRJ core) |
| BL-03 | A missing original shows the raw OS error text on the canvas; map `not_found` to a readable message | XS | `app/src` editor | EDT build | done |
| BL-04 | After `pnpm dev` exits, the PowerShell pane stops accepting input (seen in herdr only) | XS | `scripts/dev.mjs` | — | done |
| BL-05 | A garbage file named `.png` reports `unsupported` instead of `corrupt` | XS | `pf-imaging` probe | PRJ build | done (PRJ core) |
| BL-06 | `fast_image_resize` scratch is allocated infallibly (~400 MiB at 16k): an OOM aborts instead of returning `out_of_memory` | S | `pf-imaging` resize | EXP build | done |
| BL-07 | Tauri smoke doesn't check that a tile loads through the `/t/` scheme | XS | `scripts/tauri-smoke.mjs` | — | done |
| BL-08 | WebView GPU memory ~1.5 GiB at 16k (tiles LRU correct at 384 MiB); measure on the owner's iGPU machine (#48) | S | — | — | open, owner hardware |
| BL-09 | Mask undo 58–60 ms and stroke p95 25 ms at a 3840² mask (budgets 50 / 16.7 ms), indicative only | S | `app/src/features/editor` | EDT fix | done |
| BL-10 | A missing original offers Rebuild (useless) and leaves tools enabled over an empty canvas | XS | `app/src/features/editor` | PRJ relink | done |
| BL-11 | A deleted tile is requested 5× instead of 3 (FR-EDT-12) | XS | tile manager | — | done |
| BL-12 | The provider chip sits on the rail; move it to a status-bar segment (ui-guidelines) | XS | `features/provider`, shell | PRJ UI build | done |
| BL-13 | Flaky: `fr_prj_15_changed_original_opens_read_only` (size+mtime misses a rewrite within one mtime tick) | XS | `pf-core` project | PRJ core build | done (PRJ core) |
| BL-14 | EDT doesn't redraw the region outline when the margin changes, or when Retry fills in a job's region: add an EDT bridge setter | S | `features/editor` bridge, `features/region-edit` | — | done |
| BL-15 | The right panel is empty on Home: show a hint or collapse it | XS | shell | PRJ UI merge | done |
| BL-16 | EDT `library` keys serve user-folder images at full size: add a `max_side` copy (OQ-PRJ-16 said ≤ 512 px) | XS | `pf-core` tiles.rs | — | done |
| BL-17 | Flaky: `fr_prj_15_changed_original_opens_read_only` depends on an mtime tick (seen by the PRJ UI builder after the racily-clean fix) | XS | `pf-core` project tests | PRJ retention build | done (PRJ retention: hash until the first commit, R45) |
| BL-18 | FR-PRJ-21: closing with `unsaved` shows the error but doesn't ask first | XS | `features/project` | PRJ retention build | done (PRJ retention: Close anyway) |
| BL-19 | Move the feather/colour-match bake from `pf-core` `region_edit/blend.rs` into `pf-imaging` (ADR 0004 puts blend there) | S | `pf-imaging`, `pf-core` region_edit | RGN, REF 6 builds | done |
| BL-20 | FR-RGN-18 `long_running` at 180 s has no test (clock seam) | XS | `pf-core` region_edit tests | RGN builds | done |
| BL-21 | `cargo clippy --release -D warnings` fails on an unused field (`Faults.rule`, `pf-core` context.rs); CI runs only debug clippy | XS | `pf-core` context.rs | — | done |
| BL-22 | Conflict view draws each layer stack as text (OQ-PRJ-11 B fallback): EDT has no read-only "draw this layer stack" component yet; swap it into `ConflictView.tsx` panes when EDT exposes one (OQ-PRJ-11 A) | S | `features/editor` export (`LayerStack`), `features/project/ConflictView.tsx`, `pf-core` `resolve_image` (either version's layers while in conflict) | EDT build | done |
| BL-23 | A purged candidate keeps a `trash_id` that points at nothing | XS | `pf-core` project retention | — | done |
| BL-24 | Restore only `candidates`, `job_inputs` and `conflict_copy` trash items (R51): restoring an orphan or mask item is undone by the next sweep | XS | `pf-core` project, `features/project` | — | done |
| BL-25 | RGN spool replay drops an entry the store refuses for good, after logging it: surface it to the user | XS | `pf-core` region_edit runner | — | done |
| BL-26 | A restored draft's region isn't pushed back into EDT (EDT owns the region); needs the BL-14 bridge setter | XS | `features/region-edit`, `features/editor` bridge | EDT mask hand-off | done |
| BL-27 | Walkthrough polish: the "¤" cost marker (design call), Choose again's empty field, no feedback on an unchanged Save, lower-case error text, corrupt-body error wording, double `tiles_open` in dev StrictMode | S | `features/*` | — | partly done; left: "¤" marker (owner design call). StrictMode double `tiles_open` is dev-only and harmless (core guards a second build) |
| BL-28 | Flaky by design: `draftSave.test.tsx` waits two 2 s debounces inside a 5 s timeout; use fake timers | XS | `features/region-edit` tests | — | done |
| BL-29 | Flaky: `project::tests::fr_prj_21_edge_folder_moved_while_open_keeps_the_draft_and_says_so_without_paths` failed 1 of 3 local Windows runs (tests.rs:382), passes alone | XS | `pf-core` project tests | — | done |
| BL-30 | A 2-file trash item shows only the first name; Storage shows "Working 0 / Total 0 B" while the folder is moved | XS | `features/project` | — | done |
| BL-31 | EDT reload after Keep other races its 2 s mask-retry timer (no hard cancel) | XS | `features/editor` MaskSync | — | done |
| BL-32 | PRJ's open-time crash sweep deletes even a fresh `.pf-tmp` in `exports/` (EXP's own FR-EXP-17 sweep keeps fresh ones); align the age rule | XS | `pf-core` project sweep | — | done |
| BL-33 | FR-RGN-32: the second mask edge of the seam fixture measured 0.512 levels/px (limit 0.5 on the spec's edge, 0.459 there); check whether feather/colour match is asymmetric | S | `pf-imaging` blend | — | done (R68; FR-RGN-32 wording at the gate) |
| BL-34 | REF: with a default selection (draft `references: null`), a default-ticked project reference that vanishes before submit is re-derived silently (R1 covers only saved selections): send the resolved selection at submit so `prepare` can refuse it | S | `features/region-edit` store, `pf-core` references | — | done |
| BL-35 | REF: the role select is blank for `missing` items; pack card titles show the absolute path (tooltip should be relative to the library root) | XS | `features/references` | — | done |
| BL-36 | A first submit right after the spend-setup dialog once returned `estimate_stale` on the real app (estimate expired while the dialog was open): re-estimate silently or say so | XS | `features/region-edit` | — | done |
| BL-37 | **Prompt helper** (owner idea, to flesh out): predefined prompt helpers (no AI) plus an optional pop-out prompt chat that sees the region, mask and prompt. Part B supersedes #9. Scope and open questions in [scope-prompt-helper.md](scope-prompt-helper.md) | M–L | `features/region-edit`, new `pf-core` assistant seam | — | scoping (owner) |
| BL-38 | Status bar wraps the provider chip to a second line while a long save-state text shows ("Save failed, project folder moved, retrying") | XS | `shell/` status bar | — | done |
| BL-39 | After an accept the prompt clears (FR-RGN-11) and the open "Final prompt (as sent)" shows the blend instruction alone next to "Required: prompt"; show it as an empty prompt with "+ blend instruction at send" instead | XS | `features/region-edit` | — | done |

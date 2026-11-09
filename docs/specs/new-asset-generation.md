---
feature: new-asset-generation
prefix: GEN
owner: spec-new-asset-generation
status: in-review   # unclaimed → claimed → draft → in-review → complete (owner only)
depends_on: [asset-project, provider, region-edit, reference-packs, editor-canvas, export, prompt-helper]
---

# New-asset generation

## 1. Summary
Generate a whole new image (a battlemap, token or item art) from a **final prompt**, **project references**, **reference packs** and, for maps, a **structural sketch** from the project's `structure/` folder (#52, #59). It starts from **+ New** on a section's **home** or from a **New image** tab inside any **asset project**. The **job** reuses RGN's pipeline as is: **estimate** → confirm → submit → poll → fetch, the **spend cap**, the **iteration tracker**, keep-all and Discard (#56), Retry. Picking a **candidate** makes it the project's bottom image, in one undoable **revision**. GEN adds no region, no **mask** and no **patch layer**.

## 2. Scope
"Text-to-image generation of a new asset" ([feature-tiers](../feature-tiers.md), next phase, #52, #59), with the tier S rules it inherits: controls reflect the model (#37), references and packs (#51), visible cost and user-controlled retry (#23). It hosts PRM's prompt block (#58). Using a structural sketch from `structure/` is in scope because #59 names it; *drawing* one stays tier A.

## 3. Goals
- A new map, token or item card in ≤ 4 clicks plus the prompt, with the cost visible before any spend.
- Maps come out at the grid the user asked for (cols × rows) whenever the model allows it.
- Same guarantees as RGN: no paid result is lost to a crash, a conflict or a closed app; nothing resubmits by itself.
- One project holds everything about an asset: references, sketch, every iteration, every picked image.

## 4. Non-goals
- Drawing a structural sketch, art recipes, token and item presets beyond the defaults in §6 (tier A). Upscaling, background removal or tiling jobs (other ADR 0001 operations). Alpha in the canvas and in exports (ADR 0004, EXP: tier A).
- Several active jobs per project (#40). Batch generation into several projects at once.
- Owned elsewhere: the job runner, spend guard and candidate commands (RGN) · the wire format, the overlay and the fake models (PRV) · the References block and `prepare_for_send` (REF) · the prompt block, helpers and prompt chat (PRM) · folders, commits, revisions and trash (PRJ) · the canvas, compare and picker (EDT).

## 5. User stories
- As a DM preparing a session, I drop a rough room layout into a new map project's `structure/`, set the grid to 30 × 40, type "flooded crypt, torchlight", tick my "Crypt" pack and generate four maps. I pick one, then fix a room with a region edit.
- On Tokens I click **+ New → Generate new…**, name it "Goblin archer" and get transparent-background tokens on a model that supports it.
- Inside a finished item project I generate a variant, pick it, and undo when the old one was better. The variant stays in `results/`.

## 6. Functional requirements
| ID | Requirement (testable) |
|---|---|
| FR-GEN-01 | **Entry from home** (OQ-GEN-10): **+ New** (`prj-home-new`) opens a menu: **From image…** (FR-PRJ-03, unchanged) and **Generate new…**. The latter asks for a name, prefilled `New map` / `New token` / `New item` by section and checked with FR-PRJ-44's rules (` (2)` on collision), then calls `gen_project_create` and opens the project with the New image tab active. |
| FR-GEN-02 | `gen_project_create {section, name}` builds the project like FR-PRJ-09 (temp folder → rename): every subfolder, `project.json` with `kind` from the section, `original: null`, `layers: []`, and `r0` (reason `create`). No image is copied (OQ-GEN-02). |
| FR-GEN-03 | **Project with no image:** the canvas shows "No image yet. Generate one in New image." The region tool, the Region edit tab and Export are disabled with the reason `no_image`. The home card shows a placeholder and a "No image yet" badge, with its iteration count as usual. |
| FR-GEN-04 | **Entry inside a project** (OQ-GEN-09): the New image tab is available in every project, with or without an image, and uses the project's `kind`. |
| FR-GEN-05 | The model picker lists `provider_list_models{operation: generate}` (OQ-GEN-01). Offline → the cached list, marked stale. |
| FR-GEN-06 | Controls are rendered from `provider_get_capabilities{model_id, operation: generate}` per RGN Appendix A, bound by `role`. `gen_controls_evaluate` applies PRV's `evaluate` as FR-RGN-33 does. Sent values follow FR-RGN-34. A model with no `candidate_count` gets 1 result per job (R72). Source and mask inputs are never controls and never sent. |
| FR-GEN-07 | **Size block, Maps** (OQ-GEN-06): cols and rows (integers 1–200) and px per square (pps, 16–512), default 30 × 40 at 70 (#30, OQ-GEN-11). The target is `cols × pps` by `rows × pps`. |
| FR-GEN-08 | **Width/height models** (`output_size.mode = wh`): if a side exceeds `max`, pps is lowered to the largest integer with every side ≤ `max`, with the notice "model limit". Each side is then cells × pps rounded down to `step`, and at least `min`. The panel shows the sent w × h and the resulting px per square (`w / cols` × `h / rows`, 1 decimal). |
| FR-GEN-09 | **Aspect models** (`output_size.mode = aspect`): GEN sends the allowed ratio nearest to cols : rows (smallest \|ln a − ln t\|; non-ratio values such as `auto` and `source` are never picked). Off by > 1 % → the warning "cells won't be square". The px per square is shown once the result is in (`w / cols`, `h / rows`). |
| FR-GEN-10 | **Size block, Tokens and Items:** an aspect select (default 1:1, OQ-GEN-11). Aspect models offer their own ratio values (non-ratio values such as `auto` excluded). Wh models list no ratios, so the select offers 1:1, 4:5, 5:4, 3:4, 4:3, 2:3, 3:2, 9:16 and 16:9, plus a long side (default 1024, within the model's limits, snapped to `step`). |
| FR-GEN-11 | The inputs behind `output_size` are never shown as raw controls. Other size-like inputs (e.g. Ideogram `resolution`) render per Appendix A. |
| FR-GEN-12 | **Transparency** (OQ-GEN-07): when the capabilities have `background` with a transparent value, it renders as a control; on Tokens it defaults to transparent. Returned alpha is kept in the candidate file and the picked file. Every RGB8 use of an image with alpha (canvas tiles, preview, thumbnail, region crop, export) composites it over the **flatten colour**, mid grey `#808080` (`FLATTEN_BG` in `pf-imaging`, OQ-GEN-16), so the canvas, the region sent to the provider and the export agree pixel for pixel. |
| FR-GEN-13 | **References:** REF's References block, with its own draft `draft.generate.references`. FR-REF-15–27 apply unchanged. When a sketch is chosen (not **None**) and uses the same slot, that slot's `max` shown to REF is reduced by 1; with **None** it is not. |
| FR-GEN-14 | **Structural sketch block** (OQ-GEN-05): shown when the capabilities have `structure`. It lists `structure/` images (FR-REF-01 rules, sorted by name) as one choice or **None**, defaulting to **None**; the draft remembers the last choice, so no sketch is sent (or charged) unless the user chose it. **Add…** copies files in through `project_folder_add` (#53). Badges: `cloud`, `corrupt`. When the model takes no sketch but the folder holds images: "This model takes no structural sketch". |
| FR-GEN-15 | The sketch is prepared by REF's `prepare_for_send` as one more item (`source: structure`, path `structure/<file>`; OQ-GEN-12): hydrate, hash, the FR-REF-22 size rule, `reference_unavailable` with no provider call. It is sent **first** in its slot. |
| FR-GEN-16 | **Prompt** (depends on OQ-PRM-11 and OQ-GEN-15): the New image tab hosts PRM's prompt block (typed prompt, helpers, "Final prompt (as sent)", chat button). Final prompt = PRM's `prompt::compose(kind, typed, ticks, helpers, blend: none)`: the typed prompt plus the ticked helpers, never the blend instruction (#55 is RGN-only). This relies on OQ-PRM-11 redefining *final prompt*, which #57 still defines as typed prompt + blend instruction. With `prompt_required`, submit is disabled while the final prompt is empty. |
| FR-GEN-17 | **Chat context** ([seams-phase4](../agents/seams-phase4.md), FR-PRM-23–25): PRM defines `ChatContext` and the drop rule; GEN implements PRM's `ChatContextSource` for host `gen` (`GenContextSource`). While the New image tab is the host (FR-GEN-36), its prompt block publishes `draft.generate` with `prm_context_publish{host: gen}`. `build` runs only when a turn is sent and reads only that published input. **Text:** kind; model name and id; every visible control with its value and range or allowed values; size (cols × rows and pps, or ratio; sent w × h); typed prompt; ticked helpers; final prompt; each `on` reference (file, `project` or pack name, role); the sketch file name or "none". **Images**, in this order: the sketch, then the `on` references in send order. Over PRM's per-turn limits, images are dropped from the end of that order (so the sketch goes last), and the context strip names them. No GEN command, no provider call. |
| FR-GEN-18 | Seed as FR-RGN-10. |
| FR-GEN-19 | **Draft:** `draft.generate` holds the model, controls, seed, size, sketch choice, references and PRM's prompt fields, saved by PRJ's debounced draft save (FR-PRJ-19). A remembered value the capabilities reject resets to its default with a notice (FR-RGN-11). The region-edit draft is untouched. |
| FR-GEN-20 | **Estimate** 500 ms after a cost-affecting change: model, sent size or ratio, a `cost_affecting` control, or the contents of a cost-affecting slot (sketch included, FR-REF-24). The estimate id hashes exactly these. `estimate_stale` is handled as FR-RGN-13. |
| FR-GEN-21 | Session spend, cap, per-job threshold, unknown cost and the confirm dialog follow FR-RGN-14–16 unchanged. The dialog says "New image · W × H" and "with N references". |
| FR-GEN-22 | **Submit** commits the job record (`requested`) before the provider submit: `operation: generate`, model, capability snapshot, `final_prompt` as sent, `typed_prompt` (it drives the slug, the pick label and Retry, as schema §4.6 says), `blend_instruction: false`, PRM's `prompt_helpers[]` (FR-PRM-09), sent controls, `candidate_count`, seed, `output`, `references[]` (sketch row first) and the estimate. `region`, `fit`, `mask_id` and `inputs` are null (a schema §4.6 change: nullable when `operation` is `generate`, §8). One active job per project is shared with RGN: a second submit → `job_active`. |
| FR-GEN-23 | The job runs through RGN's job runner unchanged: FR-RGN-17–20, 22, 23, 35 and Appendix B (poll, cancel, resume on open, `lost` before the handle, never resubmitted, error messages). |
| FR-GEN-24 | Candidates are stored untouched in `results/NNN <slug>/` (FR-RGN-24, 39), the slug taken from `typed_prompt`. An empty slug → `new`. A size differing from the sent one by > 1 % gets a warning badge and is **not** resized. Invalid candidates as FR-RGN-24. |
| FR-GEN-25 | **Iteration tracker:** generate iterations sit in the same tracker with a `new image` badge and the sent size; the job strip, the `N refs` chip and Cancel / Retry / Dismiss are RGN's. |
| FR-GEN-26 | **Preview** (OQ-GEN-13): `gen_candidate_preview` registers the candidate as a transient image (FR-EDT-31). Same size as the working image → EDT's compare (split, flip). Otherwise, or with no image → EDT shows the candidate alone, fit to view; holding `\` shows the working image. |
| FR-GEN-27 | **Pick** (`gen_candidate_pick`, `Enter` / `edt-picker-accept`) is one PRJ commit (OQ-GEN-03, 04): the candidate file is copied write-once to `working/NNN <slug> NN.<ext>`; the new stack is `[base layer]`; the candidate → `accepted`, the job → `closed`, its other `new` candidates → `dismissed` (kept, #56); a revision with reason `accept` and label `New: ` + the first 40 chars of the typed prompt. The original file, if any, is unchanged. The thumbnail is rebuilt (FR-PRJ-50). |
| FR-GEN-28 | Patch layers of the previous stack are not carried over. They stay in the earlier revisions, and the notice says "N patch layers kept in history (undo restores them)". |
| FR-GEN-29 | Pick pushes one undo entry: undo = `project_restore_revision(parent)`, redo = restore the later one (FR-RGN-30). Undoing a project's first pick returns it to "No image yet" (FR-GEN-03). |
| FR-GEN-30 | **Pick from any generate iteration** (OQ-GEN-08): any valid, non-discarded candidate of any generate iteration can be picked, with no provider call and no spend; a trashed one is restored in the same commit (as FR-RGN-36). The job stays `closed`. |
| FR-GEN-31 | Star, Discard, Discard the rest, Dismiss and the statuses follow FR-RGN-28 unchanged. A generate job has no input files to trash. |
| FR-GEN-32 | **Retry** (`gen_job_retry_draft`) pre-fills the tab from the job: model, controls, size, `typed_prompt` and the helper ticks (FR-PRM-10), references (FR-REF-26) and the sketch, found by path and hash (`changed since NNN` or `missing`); a new random seed unless pinned. Then estimate → confirm again. |
| FR-GEN-33 | (Depends on OQ-GEN-04, OQ-GEN-14.) After a pick, region edit and export work on the picked image (proposed *base image*) as they do on an original. This needs PRJ to decode `layers[0]`'s file instead of the original; the amended texts (FR-PRJ-42, FR-RGN-04, FR-EXP-04, schema §4.3) are listed in §8. |
| FR-GEN-34 | Conflict or lock during a job: FR-RGN-37. Read-only: the tab is shown, no estimates, submit disabled with the reason; pick, star and discard fail fast with `read_only` (FR-RGN-38). |
| FR-GEN-35 | Every capability is a `pf-core` command (§9) that works without the UI, inputs validated at the boundary. A failed command leaves the project unchanged. |
| FR-GEN-36 | **Chat host = the visible tab.** Only the visible tab's prompt block is mounted. A tab switch unmounts the hidden one, which clears its published context (`prm_context_clear{host}`), and the visible one publishes its own. A chat send builds from the visible tab's host, and PRM records `host` on each turn. A `prm:insert` for a host that isn't visible fails with `no_prompt_block`. PRM's spec takes the same rule. |

## 7. UI/UX notes
- **Right panel** (OQ-GEN-09): a tab strip on the Generate area, **Region edit** (RGN) | **New image** (GEN, `app/src/features/new-asset-generation/`). A project with no image shows only New image. The visible tab is the prompt chat's host (FR-GEN-36). Tab label "New image" (OQ-GEN-15). Top to bottom: Model (picker, controls, warnings) · Size · Structural sketch (hidden when the model has none) · References (REF's block) · Prompt (PRM's block) · Cost (estimate, spend / cap, balance) · **Generate**.
- **Bottom strip:** RGN's iteration tracker and EDT's picker; `Enter` = Pick. After a pick, **Discard the rest** as in RGN.
- **States:** *empty project* FR-GEN-03 · *no model* "Pick a model" · *loading* spinner, Generate disabled · *job running* the strip is active, Generate disabled, the canvas usable · *error* inline message + hint + Retry · *offline* models stale, no estimate, Generate disabled · *provider unavailable* (`auth`) "Set up provider" · *read-only* FR-GEN-34.
- **Test ids:** `prj-home-new-{image,generate}`, `gen-new-{name,ok}`, `gen-tab`, `gen-panel-{model,estimate,submit,warning,count-single}`, `gen-size-{cols,rows,pps,aspect,long-side,result}`, `gen-sketch-{item,none,add}` (+ `data-record-id` = file), `gen-control-input` (+ `data-record-id` = input name), `gen-tracker-badge`. Confirm, job and picker buttons are RGN's and EDT's.

## 8. Data and file effects
PRJ owns storage and commits; fields go into [manifest-schema](../architecture/manifest-schema.md) (add only, OQ-GEN-02, 04, 12).

| What | Where | Recoverable |
|---|---|---|
| New project with no image | `<section>/<name>/` per FR-PRJ-12; `original: null`, `layers: []`, `r0` | with the project |
| Draft | `project.json` (schema §4.5, a new field, add only) `draft.generate{model_id, controls, seed, size{mode, cols?, rows?, pps?, aspect?, long_side?}, structure{file} \| null, references, typed_prompt, prompt_helpers (PRM)}` | with the project |
| Job | `.pforge/jobs/<id>/job.json` only (no input PNGs): `operation: generate`, `output{mode, w?, h?, aspect?, grid?{cols, rows, pps}}`, `typed_prompt`, `blend_instruction: false`, `prompt_helpers[]` (PRM), `references[]` with a `source: structure` row; `region`, `fit`, `mask_id`, `inputs` null; the rest as §4.6 | yes, authoritative |
| Candidates | `results/NNN <slug>/NN.<ext>`, untouched | kept unless discarded (#56), trash 30 days |
| Picked image | `working/NNN <slug> NN.<ext>`, write-once, never modified or deleted | yes, referenced by revisions |
| Base layer | `layers[0]` kind `base`: `{file, content_hash, width, height, job_id, candidate_id}` | yes |
| Revision | reason `accept`, label `New: …` | never deleted |
| Prepared sketch, previews | memory, per session | rebuilt |

- A crash between the `working/` copy and the manifest leaves an unreferenced file there. It is harmless: a later pick of the same candidate reuses it when the hash matches. `working/` is never swept.
- Sketch files are the user's (`structure/` is user-owned, FR-PRJ-47). A job keeps path and hash only (as OQ-REF-02).

**Amendments to accepted text** (none applies until the owner accepts the OQ named):

| Where | Accepted text | Change | OQ |
|---|---|---|---|
| schema §4.1 | `original` is always set | nullable: `null` for a project made by Generate new | 02 |
| schema §4.3 | `kind: original\|patch`; `[0]` is the original, `file: null` | adds kind `base` `{file, content_hash, width, height, job_id, candidate_id}`; `[0]` is `original` or `base`; `layers[]` may be empty | 02, 04 |
| schema §4.5 | flat draft | adds `generate{…}` (add only) | — |
| schema §4.6 | `operation: edit_masked`; `region`, `fit`, `mask_id`, `inputs` always set; `references[].source: project\|pack` | adds `generate` and `output{}`; the four fields nullable for `generate`; `source` adds `structure` | 01, 12 |
| FR-PRJ-42 | `DecodedOriginal` decodes the original | decodes `layers[0]`'s file (original or base image), alpha over the flatten colour | 04, 16 |
| FR-PRJ-47 | `working/` written only at creation and relink | also at a pick: one new write-once file, never overwritten | 04 |
| FR-PRJ-03, 49, 50 | + New opens the file dialog; home card thumbnail of the working image | + New becomes a menu; placeholder + "No image yet" with no layers | 02, 10 |
| FR-RGN-04 | region pixels from "PRJ's decoded original" | from PRJ's decoded `layers[0]` | 04 |
| FR-RGN-40 | no accept from older iterations | generate iterations exempt; `new image` badge; rows gain `operation` | 08, 09 |
| FR-EXP-04 | "equals the original's pixels" | "equals `layers[0]`'s pixels (alpha over the flatten colour)"; Export disabled with `no_image` | 02, 04, 16 |
| FR-PRV-11, FR-PRV-25 | `capabilities[]` is not read; `list_models` returns masked-edit models only | `list_models` takes an operation; for `generate` it reads `txt2img` | 01 |
| EDT (FR-EDT-24, §7 states) | compare draws `{key, rect}`; no empty-project state | whole-image preview; "No image yet" state; tiles of an alpha image over the flatten colour | 02, 13, 16 |
| REF (FR-REF-21, 25) | `prepare_for_send` over references only | also the sketch item | 12 |
| ui-guidelines | Generate (RGN) section | tab strip Region edit \| New image | 09 |
| glossary | *Original*; *Working image* = original + patches | adds *Base image*; *Working image* = base image + patches | 15 |
| ADR 0004 | "decode the original once (RGB8)"; RGB8 originals | a note for the base image (no change of decision) | 14 |
| PRM | `ChatContext` drop order; one `host` per thread; PRM slice 7 | GEN implements `ChatContextSource`; `host` per turn; PRM slice 7 dropped (§14) | OQ-PRM-10 |

## 9. Commands and events
| Command | Input → result | Errors (plus `read_only`, `locked`, `conflict` on every write) |
|---|---|---|
| `gen_project_create` | `{section, name}` → project path | `invalid_name`, `name_taken`, `library_unavailable` |
| `gen_size_fit` | `{model_id, kind, size}` → `{mode, w?, h?, aspect?, pps, notices[]}` | `unknown_model`, `invalid_input` |
| `gen_controls_evaluate` | model id, values → per control `{visible, allowed, required, error?}` | `unknown_model` |
| `gen_job_estimate` | draft → `{estimate_id, cost\|unknown, spend, cap, needs[]}` | `invalid_control`, `input_too_large`, `reference_unavailable`, `references_required`, `too_many_references`, `no_project`, PRV errors |
| `gen_job_submit` | draft, estimate_id, confirmations → job id | `estimate_stale`, `over_spend_cap`, `confirmation_missing`, `job_active`, the estimate's errors |
| `gen_job_retry_draft` | job id → pre-filled draft | `not_found`, `wrong_operation` |
| `gen_candidate_preview` | job, candidate → `{key, rect, same_size}` | `invalid_candidate`, `not_found` |
| `gen_candidate_pick` | job, candidate → `{layer_id, revision_id}` | `invalid_candidate`, `not_found`, `wrong_operation` |

- **Reused as is:** `rgn_job_cancel`, `rgn_job_dismiss`, `rgn_job_discard_rest`, `rgn_candidate_star` / `_discard`, `rgn_iterations_list` (rows gain `operation`), `rgn_spend_get` / `_set`; PRJ `project_folder_list` / `_add`, `project_restore_revision`; REF `ref_resolve` and `prepare_for_send`; EDT `register_transient_image`.
- **Rust API:** `GenContextSource` implements PRM's `ChatContextSource` (FR-GEN-17). **Uses** PRM's `prompt::compose` and `prm_context_publish` / `_clear`.
- **Events:** RGN's `rgn:job-state`, `rgn:candidate-ready`, `rgn:spend`; PRJ's `project:committed`, `project:thumb-updated`.

## 10. Provider interaction
- **Operation `generate`** (OQ-GEN-01): same route as edits (`POST /generate/custom/{modelId}`, ADR 0001), so estimate (`?dryRun=true`), submit, status, cancel and fetch reuse PRV unchanged. A generate job sends no source and no mask.
- **Proposed PRV amendment:** `list_models{operation: generate}` = models whose Scenario `capabilities[]` lists `txt2img` **and** that have a generate overlay row. `get_capabilities` takes the operation. New capability fields: `output_size` (`{mode: wh, min, max, step}` or `{mode: aspect, allowed[]}`), `structure` (`{slot}` or null), and a `background` role. Roles, `fixed` values and slot roles may differ per operation. Two new fake models (below).
- **Generate overlay rows** (from the recorded `get-model` files, 2026-11-08):

| Model | `output_size` | Count | Sketch slot | Reference slots | Transparent |
|---|---|---|---|---|---|
| GPT Image 2 | wh 16–3840, step 16 (`width`, `height`, cost-affecting) | `numOutputs` 1–10 | none until the paid check (candidate: first `referenceImages`) | `referenceImages` ≤ 10, no role, cost-affecting | no (`auto`, `opaque`) |
| GPT Image 2.5 Flare / Sunburst | same | 1–10 | none until the paid check (candidate: first `referenceImages`) | ≤ 16 | yes (`background: transparent`) |
| Ideogram 4.5 | aspect (`aspectRatio`, 23 ratios) | `numOutputs` 1–8 | none until a paid check | `referenceImages` ≤ 5, no role | no |
| Ideogram 3 Turbo / Balanced / Quality | aspect (`aspectRatio`, 15 ratios) | none → 1 per job | none | `styleReferenceImages` ≤ 4, style | no |

- No recorded model has a role-tagged structure input. Until a paid check confirms one, `structure` is null on every real row, so the sketch block shows only on the fake model (OQ-GEN-05).
- Character and 4.5 Precise Edit list only `img2img` and are not offered. Ideogram's `magicPrompt` may rewrite the prompt on the provider side; it is an ordinary control and the job records the prompt as sent.
- **Fake models:** `fake-gen-wh` (wh 64–2048 step 16, count 1–4, a `background` enum with transparent, one slot ≤ 4 that also takes the sketch, cost-affecting +1 CU per image) and `fake-gen-aspect` (ratios 1:1, 3:4, 4:3, 2:3; no count; a style slot ≤ 2). Candidates are a deterministic pattern from seed, final prompt, reference and sketch hashes, at the sent size; transparent → an alpha disc. Every Appendix D case applies.
- **Owner recordings needed** (FR-PRV-28 runbook): a free `dry-run-generate` per row; one paid generate with a sketch on GPT 2.5 (confirms the sketch slot and transparency) and one on Ideogram 3 Turbo (≈ 4 CU).

## 11. Edge cases
| Case | Behaviour |
|---|---|
| Crash mid-save | Project creation: temp folder + rename (FR-PRJ-09). Job: RGN's rules (`lost` before the handle). Pick: the `working/` file first, then the manifest; old or new state on reopen (§8). |
| Sync conflict / locked / read-only | FR-GEN-34. Paid candidates are spooled, never dropped (FR-PRJ-40). |
| Offline | Models stale, no estimate, Generate disabled. A cloud-only sketch → `reference_unavailable(not_available_offline)`. Mid-job: `unknown`, then the next status decides (FR-RGN-23). |
| Provider error / timeout | Normalised error + hint + Retry. A submit timeout → `lost` with the possible-charge notice. |
| Model limit vs grid | wh: pps lowered with a notice (FR-GEN-08). Aspect: nearest ratio with a warning (FR-GEN-09). |
| Very large or corrupt input | Sketch ≤ 16384 px, downscaled when sent (FR-REF-22); corrupt → badged, not selectable. A candidate that won't decode → `invalid` (FR-GEN-24). |
| Transparent token | Alpha is kept in `results/` and `working/`. The canvas, preview, thumbnail, region crop and export show it over mid grey `#808080` (FR-GEN-12) until alpha export (tier A). |
| Sketch or pack changed after a job | The job keeps hashes; Retry flags `changed since NNN` (FR-GEN-32). |
| Undo across the boundary | FR-GEN-29. A pick after region edits drops the patch layers from the stack but not from history (FR-GEN-28). |

## 12. Performance budget
- `gen_size_fit` and `gen_controls_evaluate` < 10 ms. The tab renders the cached capabilities in < 100 ms.
- Preview of a 3840² candidate (decode + register) ≤ 400 ms; picker thumbnails per FR-EDT-32.
- Pick (copy ≤ 50 MB + commit) ≤ 1 s; thumbnail in the background (FR-PRJ-50).
- Extra memory: ≤ 4 transient previews (FR-EDT-31, ≤ 256 MiB) plus the prepared sketch (REF's cache).
- From a section's home to a confirmed job: ≤ 4 clicks plus the name and the prompt.

## 13. Test plan
- **Rust unit** (`fr_gen_NN_*`): project with no image created atomically (02); size fit for wh, limit lowering and aspect nearest-ratio with the 1 % warning (07–10); output-size inputs never controls (11); estimate id inputs (20); job record fields and a null region (22); slug `new` (24); pick commit, base layer and copy reuse by hash (27); re-pick from an older iteration and from the trash (30); retry draft with sketch `changed` / `missing` (32); sketch slot max − 1 only when chosen (13); job `typed_prompt`, `blend_instruction: false`, `prompt_helpers[]` (22); `GenContextSource` text and image order, and the drop order over PRM's limits with a sketch and 16 references (17); alpha over `#808080` golden for decode, tiles and thumbnail (12).
- **Integration** (pf-core + **fake provider**): `+ New → generate → pick → region edit → export` on `fake-gen-wh`; the fake received the sketch hash first in its slot (15); kill between the `working/` copy and the manifest, then reopen (§8); resume mid-`running`; `failure`, `insufficient_balance`, `offline`, `bad_candidate`, `aspect_mismatch(2)`, `submit_timeout`; undo of the first pick → no image (29); one active job across RGN and GEN (22); spool replay after a conflict (34); `read_only`.
- **PRV contract** (with OQ-GEN-01): generate bodies over `ReplayTransport` from synthetic recordings until the owner's are in; `list_models{generate}` over all ten recorded `get-model` files gives exactly the seven `txt2img` models.
- **Vitest:** Size block per mode, incl. the fixed ratio list on wh models (10); the sketch block's visibility and **None** default (14); a tab switch clears and publishes the chat context (36); the tab strip with no image; the confirm text. **Thin e2e** (Playwright, mocked commands): + New menu → name → generate → pick → undo.
- **Fixtures** (generated, KQ-22, `fixtures/gen`): a 768 × 1024 line-art room layout (the sketch), REF's swatches, `capabilities-generate.json` for both fake models. No personal art.
- **Walkthrough** (dev build, fake provider, `PF_DATA_DIR=.pf-dev/data`, `PF_LIBRARY_DIR=.pf-dev/library`):

| # | Step | Expected |
|---|---|---|
| 1 | Maps home → + New → Generate new… → "Crypt" | `Maps/Crypt/` with every subfolder; empty canvas, "No image yet"; only the New image tab |
| 2 | Pick `fake-gen-wh`; grid 30 × 40, pps 70 | "model limit": pps 51, sent 1520 × 2032, "50.7 × 50.8 px per square"; estimate in CU |
| 3 | Structural sketch → Add… the layout fixture and select it; tick a pack; type a prompt; Generate | Sketch **None** until selected; confirm "New image · W × H, with N references"; `results/001 <slug>/` fills; tracker badge `new image` |
| 4 | Quit mid-`running`, reopen | Job resumes; candidates appear |
| 5 | Select #2 (preview), pick it | Canvas shows it; `working/001 <slug> 02.png`; revision `r1`; home card thumbnail updated |
| 6 | Region edit a room, accept; then pick #3 of `001` | Patch accepted (`r2`); then `r3`: stack = #3 only, notice "1 patch layer kept in history" |
| 7 | Undo twice | `r2`'s stack with the patch is back, then `r1` |
| 8 | Tokens: + New → Generate new…, `fake-gen-wh` | `background` defaults to transparent; candidate PNGs have alpha; after a pick the canvas and thumbnail show the token over mid grey, and export pixels where alpha = 0 are `#808080` |
| 9 | Switch to `fake-gen-aspect`, grid 25 × 40 | Nearest ratio 2:3, "cells won't be square"; 1 result per job |
| 10 | Retry `001` after editing the sketch file in Explorer | Pre-filled; sketch `changed since 001`; a new estimate is required |

## 14. Build split
| # | Slice | Shared files (AGENTS §9) | Depends on |
|---|---|---|---|
| 1 | PRV: `generate` operation, `output_size` / `structure` / `background`, per-operation overlay rows, fake generate models | none (pf-provider) | OQ-GEN-01 |
| 2 | PRJ: project with no image, `base` layer kind, decoded layer 0, pick commit, `draft.generate`, home placeholder | manifest schema (add), command registry (append) | OQ-GEN-02–04 |
| 3 | GEN core: size fit, controls, estimate, submit through RGN's runner, retry draft, sketch via REF | command registry (append), job fields (add) | 1, 2, REF slice 4 |
| 4 | Preview, pick, re-pick, undo | none | 3, EDT whole-image preview (OQ-GEN-13) |
| 5 | UI: + New menu, name dialog, tab strip, New image tab, tracker badge (mocked commands, parallel to 3–4) | UI feature registry (append) | ui-guidelines (OQ-GEN-09) |
| 6 | Prompt block in the New image tab, tab-host publish / clear (36), `GenContextSource` (17). **Owned by GEN**; PRM's slice 7 is the same work and is dropped there | none | PRM slices 1, 2, 5 |
| 7 | Owner recordings, contract tests, walkthrough | fixtures | 1–6 |

## 15. Open questions
| ID | Question | Options (pick in **bold**) | Blocking? | Affects |
|---|---|---|---|---|
| OQ-GEN-01 | How does PRV support generation? | **A. Amend [provider](provider.md): a `generate` operation, `output_size`, `structure`, a `background` role, per-operation overlay rows, two fake models (§10). This reverses FR-PRV-11 ("`capabilities[]` is not read") and FR-PRV-25 (`list_models` returns masked-edit models only) for the `generate` operation**: one adapter, same job pipeline / B. A separate generation adapter: no PRV change, but two code paths for one route | Yes (build) | PRV, slice 1 |
| OQ-GEN-02 | Where does a "Generate new" asset live before the first pick? | **A. The project folder is created at once with no image (`original: null`, `layers: []`)**: everything in one visible place across restarts; PRJ must allow a project with no image (EDT, EXP disable with `no_image`) / B. A hidden staging folder per section, the project is made at the first pick: no PRJ change, but results move and are invisible in Explorer | Yes (build) | [asset-project](asset-project.md), schema, EDT, EXP |
| OQ-GEN-03 | What does a pick do in a project that already has an image? | **A. It replaces the working image: new stack = the picked image only, one revision, undoable; patches stay in history** / B. Always a new sibling project: no stack change, but projects multiply and references must be re-added / C. Ask each time: flexible, one more dialog per pick | Yes | PRJ, RGN |
| OQ-GEN-04 | How is a picked image stored, and what is it called? | **A. Copied write-once to `working/`; a new layer kind `base`. Amends schema §4.1 and §4.3, FR-PRJ-42, FR-PRJ-47, FR-RGN-04, FR-EXP-04 and the glossary (*Original*, *Working image*), listed in §8; ADR 0004 per OQ-GEN-14** / B. The layer points at the file in `results/`: no copy, but breaks if the user moves results / C. Widen *Original* to mean the current bottom image: changes an accepted definition | Yes | PRJ, schema, glossary, EXP, RGN |
| OQ-GEN-05 | How is a structural sketch sent? | **A. One image from `structure/`, sent first in the slot the overlay names for that model; the block is hidden otherwise. No real model is confirmed: `structure` stays null on every real row until a paid call confirms one (GPT 2.5 first), so the block may ship with only the fake model behind it. In a role-less slot like GPT's `referenceImages` a sketch is just one more reference (#51: roles only where exposed), set apart only by going first; meanwhile a layout can go into `references/`** / B. Treat sketch files as references with role `structure` in REF's block: no new block, but no recorded model has a structure-role slot / C. Also add models with a control input (FLUX `controlImage`): stronger layout control, more overlay rows and recordings | Yes | PRV overlay, REF |
| OQ-GEN-06 | How is a map's size chosen? | **A. Cols × rows × px per square, mapped to the model: exact w × h on wh models (pps lowered at the limit), the nearest ratio with a warning on aspect models; no upscale** / B. The model's raw size controls, with the grid shown read-only: simpler, but the user does the maths | No | PRV `output_size` |
| OQ-GEN-07 | Token transparency | **A. A `background` role in the overlay; on Tokens transparent is the default where allowed (GPT 2.5 Flare / Sunburst); alpha kept in files; canvas, thumbnail, region crop and export flatten over the colour of OQ-GEN-16 until alpha export (tier A)** / B. No default; the user sets it like any control | No | PRV overlay, EXP |
| OQ-GEN-08 | Pick from older iterations? | **A. Yes for generate iterations, no spend (unlike OQ-RGN-15): a pick replaces the whole image, so the stack can't be stale** / B. Same as RGN: only the newest job, as a re-pick after undo | No | RGN FR-RGN-40 |
| OQ-GEN-09 | Where does the New image panel go? | **A. A tab strip on the Generate area: Region edit \| New image; only New image when there is no image** / B. A separate collapsible "New image" section under Generate: no tabs, but a long panel and two Generate buttons | No | [ui-guidelines](../architecture/ui-guidelines.md), RGN |
| OQ-GEN-10 | "+ New" on home | **A. A menu: From image… \| Generate new…**: one button, one more click for images / B. A separate Generate button next to + New: one click each, a busier header | No | PRJ |
| OQ-GEN-11 | Kind defaults | **A. Maps 30 × 40 at 70 px per square (#30); Tokens and Items 1:1 at 1024; Tokens transparent (OQ-GEN-07); nothing else until token and item presets (tier A)** / B. No defaults beyond the model's | No | — |
| OQ-GEN-12 | Who prepares the sketch for sending? | **A. REF's `prepare_for_send` takes it as one more item (`source: structure`, path `structure/<file>`), same hydrate, hash and size rules; schema `references[].source` gains `structure`** / B. GEN prepares it itself: no REF change, but its rules are duplicated | Yes (build) | [reference-packs](reference-packs.md), schema |
| OQ-GEN-13 | Preview a candidate of another size | **A. EDT shows the transient image alone, fit to view; `\` shows the working image** / B. Stretch it over the working image's rect: no EDT change, but distorted | No | [editor-canvas](editor-canvas.md) |
| OQ-GEN-14 | Does ADR 0004 need a superseding note? Its pick A says "decode the original once (RGB8)" and "RGB8 for originals". | **A. A note on ADR 0004, no change of decision: "original" there means the image at `layers[0]` (original or base image), alpha over the flatten colour; strategy, memory budget and RGB8 unchanged** / B. A superseding ADR restating the decode rule: literal AGENTS §3.2, more paperwork | Yes (owner, §3.2) | ADR 0004, PRJ |
| OQ-GEN-15 | Glossary and labels | **A. Add *Base image* (the bottom image of the stack: the original or a picked generated result) and redefine *Working image* = base image + patch layers; tab and chat host label "New image" (PRM's chat header "New asset" changes to match, since the tab also works in existing projects); GEN uses PRM's *prompt block* and *final prompt* as OQ-PRM-11 decides** / B. "New asset" everywhere: a glossary word, but misleading inside an existing project | No | glossary, PRM |
| OQ-GEN-16 | Flatten colour for images with alpha (canvas, preview, thumbnail, region crop, export) | **A. Neutral mid grey `#808080`**: reads on the dark UI and on a light VTT, and is neutral for models / B. White `#FFFFFF`: natural for item art, harsh on the dark UI / C. A checkerboard on the canvas only: shows alpha, but the canvas no longer matches the export or the region sent | No | ADR 0004, EDT, EXP, PRJ |

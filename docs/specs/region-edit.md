---
feature: region-edit
prefix: RGN
owner: spec-region-edit
status: in-review   # unclaimed → claimed → draft → in-review → complete (owner only)
depends_on: [asset-project, editor-canvas, provider]
---

# Region edit and patch layers

## 1. Summary
This spec covers MVP scenario steps 2–5. The user marks a **region** and paints a **mask** in it, writes the **final prompt**, and picks a model whose **model capabilities** support masked edits (#15). The **job** runs as estimate → confirm → submit → poll → fetch (ADR 0001), inside the spend guardrails (#23). The user compares the **candidates** in place and accepts one. Rust bakes the **feather** and colour match into a positioned **patch layer** (ADR 0004, KQ-23) and a **revision** is written. The **original** is never touched.

## 2. Scope
Tier S ([feature-tiers](../feature-tiers.md)): localised edit of a masked region via Scenario · region extraction with context · candidate comparison · accept as a patch layer · visible job status and cost, with user-controlled retry · editing the final prompt.

## 3. Goals
- Only the region plus its context is sent, unresampled whenever it fits the model's maximum input.
- Cost is always visible before and after a job. Spend is bounded and never surprises the user.
- An accepted patch shows no visible seam at 100 % zoom (vision success criterion).
- Mask to accepted patch takes under 5 minutes of the owner's time.
- No job, candidate or accepted state is lost to a crash or a closed app.

## 4. Non-goals
- Prompt drafting or rewriting of any kind (#9). Art recipes and reference roles (tier A). Text-to-image (tier A).
- Splitting one region into several jobs (OQ-IMG-06 C). Free move, scale or rotate of a patch layer (tier A).
- Mask painting, the view, the compare/picker components (EDT). Manifest storage, trash, sync detection (PRJ). The Scenario wire format and polling (PRV). Flatten and export (EXP).

## 5. User stories
- As a DM preparing a session, I drag a box around one room, paint the floor and type "turn this room into a kitchen", so that only that room changes.
- I see the estimated cost and my session total before I spend anything, and a large job asks me twice.
- I watch the job progress, can cancel it, and find it still running when I reopen the app after closing it.
- I flip between before and after for each candidate in place, star the ones I like, and accept one as a patch layer.
- When a result is poor, I retry with the same inputs, or change the prompt first. Nothing resubmits by itself.

## 6. Functional requirements
| ID | Requirement (testable) |
|---|---|
| FR-RGN-01 | The user creates a region by dragging a box (OQ-RGN-01). The inner rect is snapped to integer image px, clamped to the image and at least 16×16 px. |
| FR-RGN-02 | Context margin: default 128 px, adjustable 0–512 px (OQ-RGN-03). The outer rect is the inner rect grown by the margin and clamped to the image, so it may be asymmetric at edges. An outer rect over 4096 px on either side is rejected with `region_too_large` (OQ-CNV-04). |
| FR-RGN-03 | The mask from EDT is accepted only if it is 8-bit, exactly the outer rect's size, white = editable, and has at least one pixel > 0. Otherwise the command fails with `mask_invalid(reason)`. |
| FR-RGN-04 | Region pixels are extracted at full resolution from the composite of the visible layers inside the outer rect (OQ-RGN-02). With no patch layers they are bit-identical to a crop of the original. |
| FR-RGN-05 | Fit: if both outer sides are ≤ the model's maximum input px, the region is sent unresampled. Otherwise it is downscaled with Lanczos3 to fit, and the panel shows "patch N.N× softer" (N = outer ÷ sent, 1 decimal; OQ-IMG-06). The fit is recomputed whenever the region or model changes. |
| FR-RGN-06 | If the encoded region or mask exceeds the model's byte limit, submit is blocked before any upload with `input_too_large(limit)`. |
| FR-RGN-07 | The model list shows only models whose capabilities include `edit_masked` with mask support in the per-model overlay (#15). Offline, the last cached list is shown and marked stale. |
| FR-RGN-08 | Controls shown are exactly those in the model capabilities, with their type, range, allowed values and default. Unknown input types are hidden with a warning. Out-of-range values are rejected at the command boundary with `invalid_control(name)`. |
| FR-RGN-09 | The prompt field is the final prompt. Its text is sent byte for byte with nothing added (#9) and recorded on the job. If the capabilities mark the prompt as required, submit is disabled while it is empty. |
| FR-RGN-10 | If the model supports seeds, the seed is "random" or a pinned value. The seed actually used is recorded on the job (KQ-17). |
| FR-RGN-11 | The panel remembers the last model and control values per asset project. A new region starts with an empty prompt. |
| FR-RGN-12 | An estimate is requested 500 ms after the last request change and again just before submit. It shows the cost in provider units, or "unknown". |
| FR-RGN-13 | Submit carries the estimate id. If the request has changed since that estimate, submit fails with `estimate_stale` and the panel re-estimates. |
| FR-RGN-14 | Session spend = the sum of the cost of each job submitted since app start (the actual cost when known, otherwise the estimate). If spend + estimate > cap, submit fails with `over_spend_cap`. The dialog shows spend, estimate and cap, and offers an explicit "raise cap" field. |
| FR-RGN-15 | An estimate above the single-job threshold needs a second explicit confirmation. An unknown cost always needs an explicit confirmation (ADR 0001). |
| FR-RGN-16 | If no cap or threshold is set, the first submit asks for both before confirming (OQ-RGN-04). |
| FR-RGN-17 | Before the provider `submit`, the job record is written to the asset project with all inputs, the region geometry, the fit, the mask and region files and a capability snapshot. The job handle is written right after. Each asset project has at most one active job at a time (OQ-RGN-06). |
| FR-RGN-18 | The job strip shows state, progress %, elapsed time and cost (estimate, then actual). A "still running" notice appears after 3 min (OQ-PRV-05). Every state change emits `rgn:job-state`. |
| FR-RGN-19 | Cancel is offered in `queued` and `running`. On a provider ack the job becomes `canceled`. If the job can't be cancelled, the panel says so and the job continues. |
| FR-RGN-20 | A failed job shows the normalised error and hint (ADR 0001). `insufficient_balance` gets its own message. Nothing is ever resubmitted automatically. |
| FR-RGN-21 | Retry pre-fills the panel from the job (region, mask, prompt, model, controls, and a random seed unless one was pinned). It then runs estimate → confirm again. |
| FR-RGN-22 | When an asset project opens, its jobs in `submitted`, `queued`, `running` or `unknown` resume polling. A `succeeded` job whose candidates aren't stored yet fetches them. A job record with no handle becomes `lost`, with a notice that a charge may have occurred (OQ-RGN-10). |
| FR-RGN-23 | When the app goes offline mid-job, the job shows `unknown`. Polling resumes on reconnect. The job is never resubmitted. |
| FR-RGN-24 | Each candidate is stored untouched, as returned. One that won't decode is marked `invalid` and the others stay usable. If all are invalid, the job fails with `unreadable_candidates`. If a candidate's aspect ratio differs from the sent one by more than 1 %, it gets a warning badge (OQ-RGN-07). |
| FR-RGN-25 | For the selected candidate, Rust bakes a preview patch (resize to the outer rect, colour match, feather, alpha). The canvas shows it in place, and the EDT compare component toggles before/after. |
| FR-RGN-26 | Blend controls: feather 0–256 px, default 48 (0 = the painted edge only), and colour match, default on (KQ-23). If fewer than 1 % of the region's pixels are unmasked, colour match is skipped with a notice. |
| FR-RGN-27 | Accept writes the baked RGBA patch (cropped to its alpha > 0 bounds) as a patch layer at that rect: scale 1, rotation 0, opacity 1, visible. It records the source job, the candidate and the blend parameters, and writes a revision in one atomic save (PRJ). The original's bytes are unchanged (hash test). Flattened pixels outside the mask equal the pre-accept flatten. |
| FR-RGN-28 | Star toggles keep-forever. Discard moves a candidate to the project trash immediately. Accepting or dismissing a job moves its remaining unstarred, unaccepted candidates to the trash (#18). An accepted candidate is never trashed. |
| FR-RGN-29 | The blend parameters of an accepted patch layer can be re-tuned. Rust re-bakes from the untouched candidate and writes a new revision (OQ-RGN-08). |
| FR-RGN-30 | Accept and re-tune are each one step in the in-session undo (EDT). Undo never deletes revision or candidate files. A job keeps its own mask copy, so later mask undo doesn't change it. |
| FR-RGN-31 | Every capability in this spec is a `pf-core` command (§9) that works without the UI, with its inputs validated at the boundary. |
| FR-RGN-32 | No visible seam: on the seam fixture (a candidate offset by +12 levels), the mean edge step across the mask boundary after the bake is ≤ 0.5 levels/px and the residual offset is ≤ 0.1 levels (ADR 0004 measured 0.28 and 0.03). |

## 7. UI/UX notes
- **Region edit panel** (right side, `app/src/features/region-edit/`), top to bottom: Region (size, margin slider, fit and softness warning) · Model (picker plus capability controls) · Prompt (the final prompt) · Cost (estimate, session spend / cap, balance or "unknown", OQ-PRV-09) · Submit.
- **Job strip:** state, progress, elapsed time, cost, Cancel. **Candidates:** the EDT picker with star, discard and before/after; blend controls (feather, colour match); Accept · Retry · Dismiss.
- **Confirm dialog:** estimate, spend after this job, cap, and a threshold warning or an unknown-cost warning. Enter never confirms an over-threshold job.
- **States:** *empty*: "Drag a box to start a region edit" · *loading*: models loading, estimate pending (spinner, submit disabled) · *job running*: strip active, a new job is disabled, the view stays usable · *error*: inline message plus hint, Retry · *offline*: cached models marked stale, estimate unavailable, submit disabled · *provider unavailable* (`auth`): "Set up provider" opens the PRV credential screen.
- Test ids: `rgn-panel-margin`, `rgn-panel-model`, `rgn-panel-prompt`, `rgn-panel-estimate`, `rgn-panel-submit`, `rgn-confirm-ok`, `rgn-job-cancel`, `rgn-job-retry`, `rgn-candidate-star|discard|accept` (+ `data-record-id`), `rgn-blend-feather`, `rgn-blend-colour`.

## 8. Data and file effects
PRJ owns the storage, the names and the atomic save. This section states what RGN **needs** stored (seams).

| Entity | Fields RGN needs | Where | Recoverable? |
|---|---|---|---|
| Job record | id, created_at, operation `edit_masked`, model id, capability snapshot, final prompt, control values, candidate count, seed (requested/used), region `{inner, outer, margin}`, fit `{sent_w, sent_h, scale}`, estimate, actual cost, state, provider handle, provider asset ids (cache only, #20), error | asset project (PRJ) | Yes. It is the job's source of truth. |
| Job inputs | the sent region PNG and the mask PNG, exactly as sent (OQ-RGN-09) | asset project, job folder | Yes. Used for re-upload (#20) and to reproduce the job (KQ-17). |
| Candidate | id, job id, index, untouched image file, returned px size, provider metadata, state (`ready`/`invalid`), starred, discarded, accepted | asset project | Yes. Kept forever when accepted or starred, otherwise trash after 30 days (#18). |
| Patch layer | baked RGBA PNG, `cx, cy, scale, rotation, opacity, visible` (ADR 0003), source job + candidate ids, blend `{feather_px, colour_match, fitted gain/offset}` | asset project | Yes. Re-bakeable from the candidate. |
| Revision | written on accept and re-tune, labelled with the start of the prompt | asset project (PRJ) | Yes |
| Spend settings | cap, threshold (provider units) | app settings (OQ-RGN-05) | User-set |
| Session spend, preview patches, model list cache | — | memory / local cache (#19) | Rebuildable, or reset per session |

## 9. Commands and events
| Command | Input → result | Errors |
|---|---|---|
| `rgn_region_fit` | project, inner rect, margin, model id → `{inner, outer, sent_w, sent_h, scale}` | `out_of_bounds`, `region_too_large`, `unknown_model` |
| `rgn_models_list` | — → edit-capable models, `stale` flag | `offline` (returns the cache), `auth` |
| `rgn_capabilities_get` | model id → capabilities (PRV shape) | `offline`, `unknown_model` |
| `rgn_job_estimate` | draft `{region, mask, model, prompt, controls, seed}` → `{estimate_id, cost \| unknown, spend, cap, needs: [threshold \| unknown_cost \| set_cap]}` | `mask_invalid`, `invalid_control`, `input_too_large`, `offline` |
| `rgn_job_submit` | draft, estimate_id, confirmations → job id | `estimate_stale`, `over_spend_cap`, `confirmation_missing`, `job_active`, `insufficient_balance`, `invalid_request(field)`, `offline`, `save_failed` |
| `rgn_job_cancel` | job id → `canceled \| not_cancellable` | `not_found`, `offline` |
| `rgn_job_retry_draft` | job id → a pre-filled draft (nothing submitted) | `not_found` |
| `rgn_jobs_resume` | project → resumed job ids (called by project open) | per-job errors go into the job state |
| `rgn_candidate_preview` | job, candidate, blend → preview patch URI + rect | `invalid_candidate` |
| `rgn_candidate_accept` | job, candidate, blend → `{layer_id, revision_id}` | `invalid_candidate`, `save_failed`, `sync_conflict` (PRJ) |
| `rgn_candidate_star` / `_discard` | job, candidate, flag → ok | `not_found`, `save_failed` |
| `rgn_job_dismiss` | job id → number trashed | `job_active`, `save_failed` |
| `rgn_patch_retune` | layer id, blend → revision id | `source_missing`, `save_failed` |
| `rgn_spend_get` / `_set` | — / `{cap, threshold}` → spend state | `invalid_value` |

**Events:** `rgn:job-state` `{job_id, state, progress, cost, error?}` · `rgn:candidate-ready` `{job_id, candidate_id, state}` · `rgn:spend` `{spend, cap}`. A failed command leaves the asset project unchanged.

```mermaid
stateDiagram-v2
  [*] --> estimating --> awaiting_confirmation
  awaiting_confirmation --> [*]: cancel
  awaiting_confirmation --> submitted: confirm (record saved first)
  submitted --> queued --> running --> succeeded --> fetched: candidates stored
  queued --> canceled
  running --> canceled
  running --> failed
  queued --> unknown: offline
  running --> unknown: offline
  unknown --> running: reconnect
  submitted --> lost: no handle after crash
  fetched --> closed: accept / dismiss
```

## 10. Provider interaction
- **Operations** (ADR 0001 only): `list_models`, `get_capabilities`, `upload` (region, then mask), `estimate`, `submit` (`edit_masked`), `status`, `cancel`, `fetch_candidates`, `balance`. RGN never sees Scenario fields. Mask polarity is handled by the overlay in PRV.
- **Capabilities used:** the `edit_masked` operation, mask support, max input px and bytes, controls (candidate count, seed, others), whether the prompt is required, and cost-affecting flags. The estimate re-runs when a cost-affecting control changes.
- **Retries:** PRV's bounded policy applies to uploads and polling only. A submit is never retried automatically (FR-RGN-20). Expired candidate URLs are re-resolved by PRV.

## 11. Edge cases
| Case | Behaviour |
|---|---|
| Crash mid-save (accept) | PRJ's atomic save gives the old or the new state. The baked PNG is written before the manifest, and an orphan PNG is cleaned up by PRJ. |
| Crash between submit and handle write | The job becomes `lost` on reopen (FR-RGN-22). Cost may have been charged, and the notice says so. |
| App closed mid-job | It resumes on project open (FR-RGN-22). |
| Sync conflict / file locked | The save fails with `save_failed` / `sync_conflict`. Fetched candidates are kept in the local cache and the save is retried. The panel shows the error, and accept stays possible after the conflict is resolved. |
| Offline | See the *offline* state in §7 and FR-RGN-23. |
| Provider error / timeout | Normalised error plus hint, Retry offered. `rate_limited` backs off inside PRV. |
| Very large image (16k) | Extraction works on the open original (ADR 0004). The region is capped at 4096² outer. |
| Corrupt candidate, odd size | FR-RGN-24. Every candidate is resized to the outer rect. |
| Region at the image edge | The margin is clamped (FR-RGN-02). Colour match fits on whatever context exists. |
| Undo across the boundary | FR-RGN-30. Undoing an accept hides the layer, and the revision stays in history. |

## 12. Performance budget
| Step | Typical (≤ 23.5 MP map, 1–2k region) | Worst (16k map, 4096² region) |
|---|---|---|
| `rgn_region_fit` + extraction | ≤ 50 ms | ≤ 200 ms (compositing patches) |
| Encode region + mask for upload | ≤ 300 ms | ≤ 1.5 s |
| Bake a preview patch | ≤ 150 ms | ≤ 1 s |
| Accept (bake + write + revision) | ≤ 1 s | ≤ 3 s |
| Extra memory during a job | ≤ 64 MiB | ≤ 400 MiB |
| UI interactions from mask to accept | ≤ 8 clicks plus typing the prompt (5-minute criterion) | — |

## 13. Test plan
- **Unit (Rust):** geometry clamping and the 4096 cap (`fr_rgn_02_*`); fit and softness maths (`fr_rgn_05_*`); mask validation (`fr_rgn_03_*`); estimate staleness hashing (`fr_rgn_13_*`); spend arithmetic with cap, threshold and unknown cost (`fr_rgn_14_*`–`16_*`); the job state machine, including `lost` and `unknown` (`fr_rgn_22_*`); the seam metric (`fr_rgn_32_*`).
- **Unit (TS, Vitest):** the panel renders exactly the fixture capabilities (`FR-RGN-08`); debounced estimate; confirm-dialog logic; softness label.
- **Integration (pf-core + fake provider):** the full loop to an accepted revision; reopen gives a pixel-identical patch; the original's hash is unchanged (`FR-RGN-27`); resume after dropping and recreating the core context mid-`running` (`FR-RGN-22`); cancel; `insufficient_balance`; offline then reconnect; a candidate that won't decode; aspect mismatch; the prompt received by the fake provider equals the typed prompt (`FR-RGN-09`); trash and star (`FR-RGN-28`).
- **Thin e2e (Playwright, mocked core commands, KQ-33):** region → prompt → estimate → confirm → progress → pick → accept; the over-cap dialog; cancel.
- **Fixtures (generated, KQ-22):** `fixtures/gen` scripts for a synthetic "rooms" battlemap (4200×5600, plus a 16384² for perf, not committed), a seam fixture (a candidate offset by +12 levels with texture noise), and a fixture mask. Fake provider recordings go in `fixtures/provider/<model>/edit_masked_{ok,failed,canceled,slow,unknown_cost,bad_candidate}.json`. The candidate images are generated tints of the sent region.
- **App walkthrough** (dev build, fake provider, `PF_LIBRARY_DIR=.pf-dev`):

| # | Step | Expected |
|---|---|---|
| 1 | Open the rooms fixture and drag a box around one room | Box drawn; size, margin and "sent unresampled" shown |
| 2 | Paint the floor, type a prompt, pick the fake edit model | Only the fake model's controls appear; estimate in CU |
| 3 | Set the cap below the estimate and submit | Over-cap dialog. Raise the cap, confirm |
| 4 | Watch the job, then quit the app mid-`running` and reopen | The job resumes and finishes; candidates appear |
| 5 | Toggle before/after, star #2, set feather to 96, accept #1 | The patch appears in place with no seam at 100 %; #2 starred; the rest in trash |
| 6 | Undo, redo, save, close, reopen | The patch layer, blend values and revision are intact |
| 7 | Retry from the job with a new prompt | Pre-filled panel; a new estimate is required |

## 14. Build split
| # | Slice | Shared files touched (AGENTS §9) | Depends on |
|---|---|---|---|
| 1 | Region geometry, fit and extraction (`rgn_region_fit`, pf-imaging crop + Lanczos3) | command registry (append) | skeleton, PRJ open, EXP compose fn (OQ-RGN-02) |
| 2 | Draft, estimate and spend guard (`rgn_job_estimate`, `rgn_spend_*`, models, capabilities) | command registry (append), app settings (via PRJ, OQ-RGN-05) | PRV fake provider + capabilities |
| 3 | Submit, lifecycle, events, cancel, resume, retry draft | command registry (append), manifest schema: job fields (add only, via PRJ) | 2, PRJ job record |
| 4 | Candidate fetch, preview bake, blend (feather + colour match) | manifest schema: candidate fields (add only) | 3, pf-imaging blend |
| 5 | Accept → patch layer → revision, star/discard/dismiss, re-tune | manifest schema: patch layer fields (add only) | 4, PRJ revisions + trash |
| 6 | Region edit panel UI, job strip, confirm dialog (against mocked commands, in parallel with 2–5) | UI feature registry (append) | EDT region box, mask hand-off, compare/picker |

## 15. Open questions
| ID | Question | Options (pick in **bold**) | Blocking? | Affects |
|---|---|---|---|---|
| OQ-RGN-01 | How is a region created? | **A. Drag a box first, then paint inside it** / B. derive the box from the mask's bounds — A costs one drag. B needs a mask surface as big as the image, which breaks the 4096² cap (OQ-CNV-04) at 16k. | No | editor-canvas |
| OQ-RGN-02 | Region source pixels | **A. Composite of the visible layers in the outer rect** / B. original only — A makes editing over an earlier patch work. B is simpler, but re-edits ignore accepted patches. | No | export (compose fn on a rect), asset-project |
| OQ-RGN-03 | Default context margin | **A. 128 px, adjustable 0–512** / B. 15 % of the longer side — A is predictable and ≥ 2× the default feather. B scales with the region but is less obvious. | No | — |
| OQ-RGN-04 | Spend cap and threshold defaults | **A. No default: ask on the first submit** / B. fixed defaults (the owner names the values) — A means one prompt, ever. B is silent but guesses CU prices. | No | — |
| OQ-RGN-05 | Where do the spend settings live? | **A. App settings under `PF_DATA_DIR`, per machine** / B. a synced file in the library root — A keeps credentials and spend config together. B syncs across machines. | No | asset-project (app settings owner) |
| OQ-RGN-06 | Concurrent jobs | **A. One active job per asset project** / B. several — A is simpler (state, cost, UI). B is faster when batching rooms, which is tier A territory. | No | provider (polling load) |
| OQ-RGN-07 | Candidate aspect ratio differs from what was sent | **A. Resize to the outer rect, warn above 1 %** / B. reject it — A never loses a paid result. B is strict but wastes cost. | No | provider |
| OQ-RGN-08 | Re-tune feather and colour match after accept? | **A. Yes: re-bake from the candidate into a new revision** / B. only before accept — A honours "tunable per patch" (KQ-23). B saves one command. | No | asset-project |
| OQ-RGN-09 | Store the job inputs (region + mask PNG)? | **A. Yes, in the job folder, with the job's retention** / B. mask only; re-derive the region — A keeps jobs reproducible after layers change (KQ-17), at ~2–6 MB per typical job. B saves disk but re-derived pixels can differ. | No | asset-project (retention) |
| OQ-RGN-10 | A crash between submit and saving the handle | **A. Mark it `lost` with a possible-charge notice** / B. recover through a provider "list recent jobs" — A is honest and simple. B needs an operation that isn't in ADR 0001's table. | No | provider |
| OQ-RGN-11 | Model input constraints beyond max px (minimum size, multiple-of, aspect ratios) | **A. PRV exposes `input_px {min, max, multiple_of}` and RGN pads the outer rect to fit** / B. ignore them; the provider rejects — A avoids paid failures. B is simpler but fails late. | No | provider |
| OQ-RGN-12 | Glossary: add **Context margin**, **Colour match** and **Spend cap**? | **A. Add all three** / B. keep them as implementation words — A keeps specs consistent (§2). B keeps the glossary small. | No | glossary (orchestrator) |

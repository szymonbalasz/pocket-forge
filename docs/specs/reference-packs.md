---
feature: reference-packs
prefix: REF
owner: spec-reference-packs
status: complete   # unclaimed → claimed → draft → in-review → complete (owner only)
depends_on: [asset-project, provider, region-edit, editor-canvas]
---

# Reference packs and project references

## 1. Summary
A **reference pack** is a folder `<library-root>/References/<pack>/` of images plus a `pack.json` (#51). The user manages packs from a **References** rail entry. When the selected model takes reference inputs, the Generate panel shows a **References block**. It attaches the project's `references/` files by default, plus any packs the user picks, within the model's limits. Each job records exactly which files it sent (path, content hash, role), so Retry can reproduce the request and the iteration tracker can show it. Uploads reuse PRV's content-hash cache (R15).

## 2. Scope
Tier S ([feature-tiers](../feature-tiers.md)): "Reference packs + project references attached to requests; roles where the model exposes them" (#51). It supersedes the #45 rule that `references/` only holds files. Native file dialogs: #53.

## 3. Goals
- A pack is a plain folder that works in Explorer. Files dropped there appear in the app, and originals are never touched (#16).
- Only what the model accepts is shown or sent: the count per slot, roles only where the model has them (#37).
- A job's references are recorded exactly. Retry restores them and flags anything that changed since.
- Nothing is hydrated or uploaded unless it is about to be sent (#19, #20).

## 4. Non-goals
- Text-to-image (#52, the first feature after the MVP), art recipes, `structure/` and `parts/` in generation (tier A).
- Notes or roles never change the prompt (#9). Pack notes are for the user only.
- Nested packs, custom image order, pack tags or search, sharing packs between users, moving packs between roots in the app.
- Copying sent references into the project (OQ-REF-02).

## 5. User stories
- As a DM preparing a session, I keep a "Weapons" pack of six sword images. When I edit any item, I tick that pack and the model gets those images as style references.
- I drop two mood images into `Maps/Crypt/references/` in Explorer. My next edit in Crypt sends them without my doing anything, and I can untick one for a single request.
- A model takes only four references. I see "6 available · 4 sent", the first four are ticked, and I swap one.
- A week later I press Retry on iteration `003` and get the same references back. A pack image I've since repainted is marked "changed since 003".

## 6. Functional requirements
| ID | Requirement (testable) |
|---|---|
| FR-REF-01 | A **pack** is a direct subfolder of `<library-root>/References/` (FR-PRJ-51) in any root. Its images are the supported files at its top level: PNG, JPEG or WebP, ≤ 16384 px, passing the end-of-file check (FR-PRJ-06, R33). Subfolders and other files are ignored and never swept. |
| FR-REF-02 | The pack **name** is its folder name, with FR-PRJ-44's rules. Names compare case-insensitively within a root (OQ-REF-01). |
| FR-REF-03 | `pack.json` ([schema §4.12](../architecture/manifest-schema.md#412-reference-pack-packjson)) holds `pack_id` (ULID), `notes` (≤ 2000 chars) and an optional `role` per image file name. It is written temp → rename and keeps unknown fields. Draft and job refer to a pack by `pack_id`, so renaming the pack in the app or in Explorer keeps them valid. |
| FR-REF-04 | A folder under `References/` without `pack.json` is **adopted** the first time it is listed: a `pack.json` with a new `pack_id` is written. If that write can't be made (offline, locked), the pack is listed as `not_ready` and can't be attached until a later listing succeeds. |
| FR-REF-05 | If two packs share a `pack_id` (a folder copied in Explorer), the one later in (root, name) order gets a new id at that listing. |
| FR-REF-06 | `ref_list_packs` reads only each pack's `pack.json`, its thumbnail and a directory listing. Each card has the name, `rel_path` (the folder relative to its library root, `References/<name>`, shown as the card's tooltip; BL-35), image count, the first line of the notes and badges (`not_ready`, `not_available_offline`, `conflict`). Cards are sorted by name. A cloud-only `pack.json` is hydrated (as `project.json` is, FR-PRJ-17). Nothing else is hydrated. |
| FR-REF-07 | `ref_pack_create {name, paths?}` builds `References/.<name>.pf-tmp-<ulid>/` (images, `pack.json`), then renames it into place, in the first root (as FR-PRJ-09 does). A taken name → `name_taken`. Dropping a folder on the packs home creates one pack named after it, with a ` (2)` suffix if the name is taken, and its supported images copied flat. |
| FR-REF-08 | `ref_pack_add_images` **copies** each source into the pack, from a drop, **Add…** (native file dialog, #53) or a pasted file list. The source is unchanged afterwards (#16). A taken file name gets ` (2)`. A source whose hash equals an image already in the pack is reported as `already_in_pack`. Unsupported files are skipped with FR-PRJ-06's reasons, and the batch continues. |
| FR-REF-09 | The pack view re-lists on open, on window focus and on Refresh, so files added in Explorer appear without a restart (as FR-PRJ-48 does). |
| FR-REF-10 | `ref_pack_rename` renames the folder under FR-REF-02's rules. Nothing inside the folder changes. Errors: `name_taken`, `invalid_name`, `locked`. |
| FR-REF-11 | `ref_pack_update` sets the notes and per-image roles (`style`, `structure`, `identity` or none) in one `pack.json` write. A role for a file that isn't in the pack is rejected with `invalid_input`. |
| FR-REF-12 | Deleting a pack, or removing images from one, moves the files to the **References trash** `<library-root>/References/.pforge/trash/<item-id>/` and records the item there first ([schema §4.13](../architecture/manifest-schema.md#413-references-trash-item)) (OQ-REF-03). Restore puts it back. If the name is taken by then, it comes back with a ` (2)` suffix. Items older than 30 days are purged at startup (clock seam, #18). |
| FR-REF-13 | The **pack thumbnail** `References/<pack>/.pforge/thumb.jpg` is a 2×2 mosaic of the first four *local* images (≤ 512 px, JPEG q80). It is built in the background after an add, a remove or a listing that finds it missing or older than the newest image. It never hydrates a file. With no local image, the card shows a placeholder. |
| FR-REF-14 | In the pack view, image thumbnails come from EDT's `library` key space at ≤ 256 px (OQ-PRJ-16, BL-16). Thumbnails are loaded only for visible images. Viewing one may hydrate it (as FR-PRJ-17 allows). |
| FR-REF-15 | The **References block** is shown only when the selected model's `slots[]` (FR-PRV-31) is non-empty. Otherwise it is hidden and nothing is sent. |
| FR-REF-16 | **Available items**, in order: the project's `references/` images (FR-REF-01 rules) sorted by name, then each picked pack in pick order with its images sorted by name. Sorting is case-insensitive. Item key: `project:<file>` or `pack:<pack_id>:<file>`. |
| FR-REF-17 | `ref_resolve` reconciles the draft's `references` ([schema §4.5](../architecture/manifest-schema.md#45-draft)) with the available items and the model's slots. Items keep their earlier `on` and `role`. A new item is `on` if its slot has room. Its default role is its `pack.json` role if the model has a slot for that role, otherwise the first slot's role. Items that are no longer available drop out, with a notice. |
| FR-REF-18 | **Limits** per slot are `min`–`max` (FR-PRV-31). Ticking beyond a slot's `max` is refused (the checkbox is disabled, "limit N reached"). If a model change lowers a limit, the last `on` items in list order are turned off, with a notice. A slot under its `min` disables submit and names the slot (`references_required(slot, min)`). |
| FR-REF-19 | **Roles** are shown only where the model exposes them. One slot without a role: no role UI. One slot with a role: a read-only role label in the header. Several slots: a role select per item, over the roles of the model's slots. |
| FR-REF-20 | Ticks, picked packs and role changes are written to the draft through PRJ's debounced draft save (FR-PRJ-19). They are not on the undo stack. |
| FR-REF-21 | **Prepare for send** (a Rust API that RGN calls in estimate and submit): for each `on` item, hydrate it if it is cloud-only, read it, hash it (`sha256`), and check it against the slot's limits. Items whose bytes are identical are sent once, with a notice. If an item can't be read → `reference_unavailable(key, reason: missing \| not_available_offline \| corrupt \| locked)`, and no provider call is made. Nothing is dropped silently. |
| FR-REF-22 | **Sent bytes**: a file is sent as-is if it is PNG, JPEG or WebP, its long side is ≤ 2048 px and ≤ the model's `input_px.max`, and it is within `max_bytes`. Otherwise it is downscaled with Lanczos3 to the smaller of those sides and encoded as PNG. The result is deterministic, so the same file always gives the same sent bytes (OQ-REF-04). |
| FR-REF-23 | Prepared references are cached in memory per session, keyed by (path, size, mtime), within ≤ 64 MiB (LRU). A cache hit reads nothing from disk. |
| FR-REF-24 | **Cost**: the estimate id includes the ordered `(slot, sent hash)` list only for slots whose control is `cost_affecting` (FR-RGN-42). Reference changes in other slots never re-estimate. |
| FR-REF-25 | The **job record** `references[]` ([schema §4.6](../architecture/manifest-schema.md#46-job-pforgejobsidjobjson)) lists each file sent: `source`, `pack_id`, `pack_name`, `path`, `content_hash`, `slot`, `role` and `sent{w, h, bytes, hash, resized}`. It is committed with the `requested` record before the provider `submit` (FR-RGN-43), and never changes afterwards. Paths are relative: `references/<file>` for the project, `References/<pack>/<file>` for a pack. |
| FR-REF-26 | **Retry** (FR-RGN-44) turns the job's `references[]` into the draft: its packs are picked, its items are `on` with their roles, and every other item is `off`. Each item is found again by `pack_id` (falling back to `pack_name`) and file name. A different hash marks it `changed since NNN`, still `on`, and the current bytes are sent. A file that can't be found is shown as `missing`, `off`; it still shows a role over the model's slots, as an available item does (the draft is unchanged, BL-35). |
| FR-REF-27 | Read-only project (FR-RGN-38): the block is shown but disabled. Packs can still be managed (they aren't part of the project). |
| FR-REF-28 | `pack.json` conflict copies (FR-PRJ-34's name rule) are detected at listing. The version with the later `updated_at` is kept, the other goes to the References trash, and a notice is shown (OQ-REF-05). Conflict copies of images are just other images. |
| FR-REF-29 | Every capability is a `pf-core` command (§9) with inputs validated at the boundary. A failed command leaves packs and the project unchanged. |

## 7. UI/UX notes
- **Rail:** a **References** entry between Items and Settings (`prj-rail-references`, FR-PRJ-52). It shows REF's packs home in the home slot.
- **Packs home** (`app/src/features/references/`): pack cards (`ref-home-card`, `data-record-id` = pack id; context menu Rename, Delete), **+ New pack** (`ref-home-new`, a name dialog), a full-area drop zone for folders (`ref-home-dropzone`), and **Recently deleted** (`ref-trash-list`, Restore `ref-trash-restore`). Empty: "Create a pack, or drop a folder of reference images".
- **Pack view:** name (`ref-pack-name`, click to rename), notes (`ref-pack-notes`), **Add…** (`ref-pack-add`), Refresh (`ref-pack-refresh`), Delete (`ref-pack-delete`), and an image grid (`ref-pack-image` + `data-record-id` = file) with a role select (`ref-pack-image-role`), Remove, and badges (cloud, corrupt). The whole view is a drop zone.
- **References block** (in RGN's Generate section, between Model and Prompt; RGN imports it from `features/references/index.ts`, R44): a header with the count per slot (`References 3 / 4`, or `Style 2/4 · Identity 1/1`); groups "This project", then each picked pack (× to unpick) from **+ Pack** (`ref-block-pack-picker`, a multi-select popover); items (`ref-block-item` + `data-record-id` = key) with a checkbox (`ref-block-item-check`), a role select where FR-REF-19 allows (`ref-block-item-role`), and badges `cloud`, `changed since NNN`, `missing`, `duplicate`, `corrupt`.
- **States:** *hidden* when the model has no slots · *empty* "No references. Add files to this project's references folder or pick a pack" · *over the limit* "6 available · 4 sent" · *under min* the slot named in red, submit disabled · *loading* skeleton thumbnails · *offline* cloud-only items badged, can't be prepared · *read-only* disabled.
- The confirm dialog adds "with N references". The tracker chip adds `N refs`, and its tooltip lists them (FR-RGN-45).

## 8. Data and file effects
Fields: [manifest-schema](../architecture/manifest-schema.md) §2, §4.5, §4.6, §4.12, §4.13.

| What | Where | Recoverable |
|---|---|---|
| Pack images | `<library-root>/References/<pack>/<file>`, copied in, never rewritten by the app | user files; References trash for 30 days |
| `pack.json` | `References/<pack>/pack.json`, mutable, temp → rename | yes: missing → re-adopted with a new id (drafts and jobs fall back to the name) |
| Pack thumbnail | `References/<pack>/.pforge/thumb.jpg` | rebuildable |
| References trash | `References/.pforge/trash/<item-id>/` + `item.json` | 30-day restore |
| Draft `references` | `project.json` draft (§4.5) | with the project |
| Job `references[]` | `.pforge/jobs/<id>/job.json` (§4.6), write-once content | with the project; it holds hashes, not copies (OQ-REF-02) |
| Uploaded refs | PRV's in-memory hash cache, `provider{}.asset_refs.references` | disposable (#20) |
| Prepared bytes | memory, per session (FR-REF-23) | rebuilt |

The app never writes inside a project's `references/` except through `project_folder_add` (FR-PRJ-47).

## 9. Commands and events
| Command | Input → result | Errors (plus `locked`, `library_unavailable` on every write) |
|---|---|---|
| `ref_list_packs` | — → cards (FR-REF-06) | — |
| `ref_pack_get` | `pack_id` → `{pack_id, name, notes, images[{file, bytes, mtime, w?, h?, role?, state: ok\|cloud_only\|corrupt\|unsupported}]}` | `unknown_id` |
| `ref_pack_create` | `{name, paths[]?}` → pack + per-path results | `invalid_name`, `name_taken` |
| `ref_pack_rename` / `ref_pack_update` | `{pack_id, name}` / `{pack_id, notes?, roles?{file: role\|null}}` → pack | `unknown_id`, `name_taken`, `invalid_name`, `invalid_input` |
| `ref_pack_add_images` | `{pack_id, paths[]}` → per item `added{file}` \| `already_in_pack{file}` \| `skipped(reason)` | `unknown_id` |
| `ref_pack_remove_images` / `ref_pack_delete` | `{pack_id, files[]}` / `pack_id` → trash item id | `unknown_id` |
| `ref_trash_list` / `ref_trash_restore` | — / item id → items / restored path | `unknown_id` |
| `ref_resolve` | `{model_id, references}` (open project) → `{slots[{name, role, min, max, on}], items[{key, source, pack_id?, pack_name?, file, role?, on, state, badges[], thumb_key?}], notices[]}` | `no_project`, `unknown_model` |

- **Rust API for RGN:** `references::prepare_for_send(core, caps, &draft.references) → Vec<SentReference>` (FR-REF-21–23). It returns the PRV `ReferenceImage`s and the job's `references[]` rows. Errors: `reference_unavailable`, `references_required`, `too_many_references(slot, max)`.
- **Events:** `ref:packs-changed`, `ref:add-progress{done, total}`, `ref:thumb-updated{pack_id}`, `ref:trash-purged{count, bytes}`.

## 10. Provider interaction
- No calls of its own. RGN's estimate and submit carry the references in `JobRequest.references` (FR-PRV-33). PRV uploads them inside estimate or submit, once per sent hash per session (FR-PRV-14, R15), and re-uploads a missing one (FR-PRV-15).
- Capabilities used: `slots[]{name, role, min, max, cost_affecting}`, `input_px.max`, `max_bytes` (FR-PRV-31).
- A model with a required slot (e.g. Ideogram Character, `characterReferenceImages` ≥ 1) is now offered (FR-PRV-32). REF enforces `min` (FR-REF-18).

## 11. Edge cases
| Case | Behaviour |
|---|---|
| Crash mid-save | Pack create: temp folder + rename. Image copy and `pack.json`: temp → rename. Trash: `item.json` first, then the move, rolled forward at startup. The job's `references[]` is in the `requested` commit, so it is all or nothing. |
| Sync conflict / locked | `pack.json` per FR-REF-28. A locked file is retried for ≤ 3 s, then `locked`. An image locked at send time → `reference_unavailable(locked)`. |
| Offline / cloud-only | Packs can be managed for local files. Cloud-only items are badged, and preparing one fails with `not_available_offline`. Estimate and submit are offline-disabled anyway (RGN). |
| Pack edited after a job | The job keeps its hashes. Retry flags the change (FR-REF-26). |
| Pack or file deleted or renamed in Explorer | `ref_resolve` drops it with a notice. A renamed pack keeps its `pack_id` and is found. Retry shows a missing file as `missing`. |
| Library root unavailable | Its packs aren't listed. Its items in a draft resolve as `missing`. |
| Provider error / upload failure | Normalised by PRV. RGN shows it and offers Retry. Nothing is resubmitted. |
| Very large or corrupt image | ≤ 16384 px is accepted and downscaled when sent (FR-REF-22). Corrupt or unsupported files are skipped at add, or badged if dropped in Explorer, and can't be ticked. |
| Undo | Pack edits aren't undoable (use the trash). Ticks are draft state. |

## 12. Performance budget
- `ref_list_packs`: 100 packs in < 300 ms. Pack view with 200 images: listing in < 200 ms, thumbnails lazy. `ref_resolve` in < 50 ms (it reads no image bytes).
- `prepare_for_send`, cold, 4 refs of ≤ 20 MB: < 1 s typical, ≤ 5 s for a 16k ref (decode + downscale). Warm (cache hit): < 5 ms. Memory: cache ≤ 64 MiB, peak per 16k ref ≤ 800 MiB transient (decode), released after.

## 13. Test plan
- **Rust unit** (`fr_ref_NN_*`): name rules and collisions (02, 07, 08), adoption and duplicate ids (04, 05), reconcile: defaults, kept state, limits, model change, roles (16–19), sent-bytes rule with a downscale golden (22), dedupe (21), cache keys (23), estimate-id inputs (24), job rows (25), Retry matching (26), conflict-copy pick (28), trash purge under the fake clock (12).
- **Integration** (pf-core + fake provider): the full loop with references, checking the fake received the exact sent hashes; Retry after an in-place edit, a pack rename and a delete; a cloud-only item offline (fake fs-attributes seam); a kill during pack create and during a trash move (`PF_FAULT`); read-only refusals.
- **Vitest:** block visibility, the header per slot layout, the tick limit, role UI per FR-REF-19 over `fixtures/rgn/capabilities-refs.json`. **Thin e2e** (Playwright, mocked commands): create → add → rename → delete → restore; block tick limit and pack picker.
- **Fixtures** (generated, KQ-22, `fixtures/gen`, no personal art): six 256 px pattern swatches (PNG, JPEG, WebP), a 5000 px JPEG (downscale path), a truncated PNG, a pack folder with `pack.json` and a conflict copy, `capabilities-refs.json` (style ≤ 2 cost-affecting, identity ≤ 1), and the recorded Ideogram Character fixture (required slot).
- **Walkthrough:** [Appendix A](#appendix-a-walkthrough).

## 14. Build split
Built around RGN slice 3 (submit, job records), which is in progress.

| # | Slice | Shared files (AGENTS §9) | Depends on |
|---|---|---|---|
| 1 | Packs core: `pf-core/src/references/`: list, adopt, create, rename, update, add, remove, delete, References trash + purge, thumbnail; `References/` on root set (FR-PRJ-51) | command registry (append), manifest schema (add §4.12/4.13) | PRJ store slices 1–2 |
| 2 | PRV references: slots `min/max/role` + overlay `slot_roles`, required slot offered, refs in the estimate/submit body + uploads + Appendix B, fake slots + determinism (FR-PRV-31–35) | none (pf-provider only) | — (parallel to 1) |
| 3 | Packs UI: rail entry, packs home, pack view, dialogs, mocked commands | UI feature registry (append); `tauri-plugin-dialog` (#53, orchestrator adds) | PRJ rail (parallel to 1) |
| 4 | Resolve + prepare: `ref_resolve`, `prepare_for_send` (hydrate, hash, downscale, cache, dedupe), pure, unit-tested; no RGN edits | command registry (append) | 1, 2 |
| 5 | References block UI on mocked `ref_resolve`, mounted in GeneratePanel (one import) | none | 3 (parallel to 4) |
| 6 | RGN integration: draft field, estimate id, `references[]` on the job, retry draft, tracker + confirm text (FR-RGN-41–45); e2e + walkthrough | job + draft fields (add only) | 4, 5, **RGN slice 3 merged** |

## 15. Open questions
| ID | Question | Options (pick in **bold**) | Blocking? | Affects |
|---|---|---|---|---|
| OQ-REF-01 | Where the pack name lives | **A. The folder name, like projects (#44); `pack.json` has no name**: Explorer renames just work / B. A `name` field in `pack.json` (as #51 lists): display names free of path rules, but they can drift from the folder | No | 1, schema §4.12 |
| OQ-REF-02 | Does a job keep copies of the references it sent? | **A. No, path + hash only; Retry flags changes**: no extra synced bytes / B. Content-addressed copies in `.pforge/refs/<hash>`: exact reproduction, but every pack image used is duplicated into each project | No | 6, schema §4.6 |
| OQ-REF-03 | Where deleted packs and images go | **A. A References trash in the library root, 30 days (#18)**: like the project trash, no new dependency / B. The OS recycle bin: familiar, but needs a new crate (`trash`) and is per machine | No | 1 |
| OQ-REF-04 | Long-side cap for sent references | **A. 2048 px (and ≤ the model max)**: small uploads, enough for style / B. The model's `input_px.max`: more detail, larger uploads / C. Always as-is up to the model's limits | No | 4 |
| OQ-REF-05 | `pack.json` sync conflict | **A. The later `updated_at` wins; the other is trashed and restorable**: notes and roles are small / B. Show both and let the user pick, as #39 does for projects: more UI for little data | No | 1 |

**Resolved:** OQ-REF-01 ✅ A, the folder name · OQ-REF-02 ✅ A, path + hash only · OQ-REF-03 ✅ A, a References trash in the library root · OQ-REF-04 ✅ A, 2048 px · OQ-REF-05 ✅ A, the later `updated_at` wins (#54)

## Appendix A: walkthrough
Dev build, fake provider, `PF_DATA_DIR=.pf-dev/data`, `PF_LIBRARY_DIR=.pf-dev/library`.

| # | Step | Expected |
|---|---|---|
| 1 | Launch, click References | `References/` exists in the root; empty packs home |
| 2 | + New pack "Swords"; Add… three swatches; drop two more and the truncated PNG on the view | 5 images, 1 `corrupt` skipped; `pack.json` with a `pack_id`; mosaic thumbnail on the card |
| 3 | Copy a swatch into the pack folder in Explorer, focus the app; set one image's role to identity, add notes; rename the pack to "Blades" | The new image is listed; the folder is renamed and `pack_id` is unchanged |
| 4 | Copy two images into a Maps project's `references/`; open it, draw a region, paint a mask, pick `fake-alpha-edit` | No References block |
| 5 | Pick `fake-white-edit` (style ≤ 3), then + Pack → Blades | 2 project items + 1 pack item ticked (3/3); the rest unticked; a 4th can't be ticked |
| 6 | Pick `fake-black-edit` (style ≤ 2 cost-affecting, identity ≤ 1) | Role selects shown; the identity image defaults to identity; ticking a style item re-estimates |
| 7 | Pin the seed, untick all items and submit; then tick the 3 items again and submit (R61: the run with references is the newest, so Retry is offered on it) | `002`'s `job.json` `references[]` lists 3 files with path, hash, role; chip "3 refs"; candidates differ between the two runs |
| 8 | Overwrite a sent pack image in Explorer, Retry `002` | Same items ticked; that one shows `changed since 002` |
| 9 | Delete Blades; check the Retry draft; restore it from Recently deleted | Its items show `missing`; after restore they resolve again |
| 10 | Manual (OneDrive, R13): make a project reference cloud-only, go offline, submit | The item is badged; `reference_unavailable(not_available_offline)`; no spend |

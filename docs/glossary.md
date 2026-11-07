# Glossary

> Status: **draft.** Use only these terms. Propose new ones as open questions.

| Term | Meaning |
|---|---|
| **Asset** | One piece of campaign art: a battlemap, token, item art or (later) card. |
| **Asset type** | The kind of asset: map, token or item (manifest field `kind`). It decides the section, presets and specialised controls. |
| **Asset project** | The working record of an asset: source images, references, prompts, settings, masks, layers and revisions. |
| **Library** | The set of local folders the app indexes. It holds assets and asset projects. |
| **Library root** | A folder the user adds to the library. It holds the sections. It can be cloud-synced. |
| **Collection** | A user-defined group of assets. |
| **Campaign subject** | A location, NPC or item in the user's campaign that assets can link to. |
| **Original** | The image an asset project started from, copied into the project's `working/` folder. Neither the copy nor the source is ever modified. |
| **Layer** | An image placed in an asset project with position, scale, rotation, opacity and visibility. |
| **Patch layer** | A layer made by accepting a candidate for a region. It is positioned over the region it edits. |
| **Mask** | Painted pixels that mark which part of a region the provider may change. |
| **Region** | The rectangle the user selects for a localised edit (the *inner* rect). It is extracted at full resolution together with its **context margin**. |
| **Context margin** | Pixels around a region that are sent to the provider for context but are never editable. Region + context margin = the outer rect. |
| **Job** | One request to the provider, for example a generation or an edit. It has a status and a cost. |
| **Provider asset ref** | The provider's id for an uploaded image. A disposable cache: re-uploaded from local files when missing (#20). |
| **Balance** | The provider credit remaining on the user's account, if the provider reports it. |
| **Job handle** | The provider's id for a submitted job, used to poll and fetch it. |
| **Estimate** | The provider's pre-flight cost for a job, fetched without running it. |
| **Session spend** | The provider cost accumulated in the current app session, checked against the spend cap. |
| **Job runner** | The core component that drives a job through its states (submit, poll, fetch) and persists each transition. |
| **Capability snapshot** | A copy of the model capabilities used for a job, stored on the job record for reproducibility (KQ-17). |
| **Spend cap** | The user-set limit on cost per session, plus a per-job confirmation threshold (#23). |
| **Candidate** | One image a job returns, waiting to be compared, picked or discarded. |
| **Revision** | A saved state of an asset project (on accept, explicit save, restore, re-tune or conflict resolution). Revisions form a recoverable history. |
| **Re-tune** | Changing a patch layer's feather or colour match after accept. It creates a new layer file and a revision. |
| **Manifest** | `project.json` in a project folder. It records the asset project's current state (layers, masks, jobs, candidates, revisions). |
| **Draft** | Unsubmitted work in an asset project (prompt, settings, mask), saved automatically after a short idle. |
| **Head revision** | The most recent revision of an asset project. |
| **Import destination** | Retired by #44: images are copied into the new project folder's `working/`. |
| **Spool** | Per-machine holding area for writes that can't reach the library yet (e.g. during a sync conflict); replayed afterwards. |
| **Trash** | Per-project holding area for discarded items; emptied after 30 days (#18). |
| **Conflict copy** | A duplicate file created by a sync service when two machines changed the same file. |
| **Read-only** | An asset project that can be viewed (and masks painted) but not changed, e.g. when it was written by a newer app version. |
| **Local cache** | Per-machine, rebuildable data outside the library (tiles, index). Losing it loses no project data. |
| **Reference image** | An image supplied to a job with a **reference role**. |
| **Reference role** | What a reference image may influence: *style*, *structure* or *identity*. |
| **Structural sketch** | A drawn or imported image that fixes layout or pose (the *structure* role). |
| **Art recipe** | Reusable production guidance: style text, approved references, composition constraints, output requirements and tested settings. |
| **Final prompt** | The exact prompt text submitted to the provider. It is always visible and editable. |
| **Provider** | The generation service. Currently Scenario. |
| **Fake provider** | A deterministic stand-in provider used in dev and tests. It never spends credits. |
| **Mask convention** | How a given model reads a mask (e.g. white = edit, black = edit, transparent = edit). The app converts from its canonical mask. |
| **Model overlay** | The hand-maintained per-model table for facts the provider doesn't expose (e.g. mask convention). |
| **Model capabilities** | The settings and inputs a provider model actually supports. They drive which controls are shown. |
| **Cost** | The provider-reported charge for a job, in provider units (Scenario: Compute Units). |
| **Tile pyramid** | Multi-resolution 512 px tiles of an image, used for display. A rebuildable cache. |
| **View** | The editor's current zoom and pan. |
| **Feather** | The soft inward edge Rust applies to a patch layer's mask so it blends without seams. (Brush softness while painting a mask is called *hardness*.) |
| **Export** | A flattened image, written for use in a VTT. |
| **Flatten** | Compositing an asset project's visible layers into one image. Rust is the authority (KQ-25). |
| **Px per square** | Pixels per grid square of a map. Export scales from the source px per square to a target. |
| **Display size** | Viewing an asset at the size it will appear in the VTT. |
| **Colour match** | An automatic colour adjustment that makes a patch layer match its surroundings. |
| **Section** | One asset type's folder in a library root (`Maps/`, `Tokens/`, `Items/`) and its rail entry (#44). Not the same as section-based editing, which means editing by region. |
| **Project folder** | The visible folder of one asset project: `<section>/<name>/` with `project.json`, `working/`, `references/`, `structure/`, `parts/`, `results/`, `exports/` and a hidden `.pforge/` (#44). |
| **Working image** | The original plus its accepted patch layers: what the canvas shows and the home thumbnail depicts. |
| **Iteration** | One job that returned candidates, stored as one `results/NNN <slug>/` folder. |
| **Iteration tracker** | The bottom strip listing a project's iterations; it hosts the candidate picker for the selected one. |
| **Part** | A reusable cut-out (door, statue, cart) kept in `parts/` and placed on other projects later (tier A, #45). |
| **Home** | The screen shown when no project is open: the project grid of the selected section. |
| **Thumbnail** | The small preview of a project's working image shown on home (`.pforge/thumb.jpg`). |
| **User-owned folder** | A project folder the user fills from Explorer (`references/`, `structure/`, `parts/`); the app picks up files there. The others are app-owned. |

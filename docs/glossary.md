# Glossary

> Status: **draft.** Use only these terms. Propose new ones as open questions.

| Term | Meaning |
|---|---|
| **Asset** | One piece of campaign art: a battlemap, token, item art or (later) card. |
| **Asset type** | The kind of asset. It decides the presets and specialised controls. |
| **Asset project** | The working record of an asset: source images, references, prompts, settings, masks, layers and revisions. |
| **Library** | The set of local folders the app indexes. It holds assets and asset projects. |
| **Library root** | A folder the user adds to the library. It can be cloud-synced. |
| **Collection** | A user-defined group of assets. |
| **Campaign subject** | A location, NPC or item in the user's campaign that assets can link to. |
| **Original** | The image an asset project started from. It is never modified. |
| **Layer** | An image placed in an asset project with position, scale, rotation, opacity and visibility. |
| **Patch layer** | A layer made by accepting a candidate for a region. It is positioned over the region it edits. |
| **Mask** | Painted pixels that mark which part of a region the provider may change. |
| **Region** | A rectangle of the asset, extracted at full resolution with surrounding context and sent for a localised edit. |
| **Job** | One request to the provider, for example a generation or an edit. It has a status and a cost. |
| **Candidate** | One image a job returns, waiting to be compared, picked or discarded. |
| **Revision** | A saved, accepted state of an asset project. Revisions form a recoverable history. |
| **Reference image** | An image supplied to a job with a **reference role**. |
| **Reference role** | What a reference image may influence: *style*, *structure* or *identity*. |
| **Structural sketch** | A drawn or imported image that fixes layout or pose (the *structure* role). |
| **Art recipe** | Reusable production guidance: style text, approved references, composition constraints, output requirements and tested settings. |
| **Final prompt** | The exact prompt text submitted to the provider. It is always visible and editable. |
| **Provider** | The generation service. Currently Scenario. |
| **Model capabilities** | The settings and inputs a provider model actually supports. They drive which controls are shown. |
| **Export** | A flattened image, written for use in a VTT. |

# Seams: phase 1b foundation specs

Who owns what between this round's specs. Authors may push back **once**, through an open question in their spec. Otherwise this table is binding for the round.

## Specs in this round

| Spec | Prefix | Owns |
|---|---|---|
| [asset-project](../specs/asset-project.md) | PRJ | Library folder config. Import (copy in, frictionless, #16). On-disk layout and **manifest schema**: asset, layers incl. patch layers, masks, job records, candidates, revisions. Atomic save, crash safety, reopen. Retention and trash (#18). Sync-conflict detection (KQ-13). Location of the local cache (#19). |
| [editor-canvas](../specs/editor-canvas.md) | EDT | The **view** (zoom, pan, display at intended size). Tile-pyramid display through the URI scheme (ADR 0003). Layer display. **Mask painting** tools and mask hand-off. In-session undo (KQ-16, OQ-CNV-05). The before/after compare and candidate-picker UI components. |
| [region-edit](../specs/region-edit.md) | RGN | Region selection → extraction with context (ADR 0004). Job request composition (final prompt, model, controls from model capabilities). Estimate → confirm → spend cap (#23). Job lifecycle UI. Accepting a candidate → **patch layer** (feather + colour match baked in by Rust) → revision. |
| [provider](../specs/provider.md) | PRV | The ADR 0001 adapter: the Scenario implementation and the **fake provider**. Credentials in the Credential Manager (KQ-05). Capability normalisation plus the per-model overlay (#15). Error normalisation, polling, automatic uploads (#20). Fixture recording format. OQ numbering continues from **OQ-PRV-11**. |
| [export](../specs/export.md) | EXP | Flattened export (strip compose, ADR 0004). Export at a chosen px per square (#26). Formats and limits (OQ-IMG-03/04/13). Export destination and naming. |

## Interfaces

| Interface | Provider side | Consumer side | Rule |
|---|---|---|---|
| Manifest schema (layers, patch layer fields, masks, jobs, revisions) | PRJ | EDT, RGN, EXP | PRJ defines the fields. RGN states what a patch layer *needs*, and PRJ owns how it's stored. |
| Mask hand-off | EDT | RGN | A region-sized 8-bit mask, white = editable (ADR 0001 canonical), in image px coordinates. |
| Region geometry | RGN | EDT, PRJ | Rect in image px plus a context margin. EDT draws it; PRJ stores it on the job record. |
| Provider adapter operations | PRV | RGN | Exactly the ADR 0001 operation table. RGN never sees Scenario-specific fields. |
| Tile pyramid build and serve | EDT (contract), ADR 0004 (method) | EDT | The URI scheme path format and invalidation when layers change are defined in EDT. |
| Flatten | EXP | RGN (patch blend preview?), golden tests | Rust is the authority (KQ-25). One compose function, shared by export and golden tests. |
| Core commands | every spec | the UI and the future MCP endpoint (#36) | Every capability is a Rust core command with validated input (ADR 0002). List yours in spec §9. |

## Out of scope for this round (tier A+)

Library browsing UI, thumbnails and search, recipes, reference roles, layer transforms beyond patch placement, sketches, tokens, items, animation, the MCP endpoint.

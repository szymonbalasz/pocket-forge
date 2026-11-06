# Vision

> Status: **draft**, under owner review during kickoff. Open items are tracked in [kickoff-questions.md](kickoff-questions.md).

## Positioning

A desktop **asset studio** for preparing D&D sessions. It produces high-quality, consistent campaign art through deliberate iteration and precise control, rather than one-shot generation.

- Primary asset types: **battlemaps, tokens, item art**. **Cards** are a possible extension.
- All asset types share one **library** and one editing foundation, with specialised controls and presets where they help.
- Generation provider: **Scenario** (for now).

## Users

> TODO (kickoff): KQ-01. Is it the owner only, or any DM who installs it with their own Scenario account?

## What it replaces

> TODO (kickoff): the current workflow (the provider's web UI, a separate image editor, loose folders, and prompt docs kept elsewhere?).

## Principles

- **Quality comes from controlled iteration.** Preserve approved details, compare alternatives, reuse successful recipes, and review art at its intended display size.
- **Any image is editable immediately**, wherever it was made.
- **The user owns plain files.** The library is ordinary local folders, including cloud-synced ones.
- **Nothing accepted is lost.** Originals and previously accepted versions stay recoverable.
- **Reference roles stay distinct.** A style reference never dictates layout or replaces a character's identity.
- **Honest controls.** The app shows only the controls the selected model actually supports.
- **Visible cost.** Generation cost and job status are always shown. Retries are bounded and under the user's control.
- **No browser, no server to manage.** It is an installed app. Its UI may use web technology inside the app window.

## Non-goals (proposed, owner to confirm)

- It is not a virtual tabletop. It exports *to* VTTs.
- It is not a general-purpose image editor. It has a focused, practical toolset.
- No hosted service, accounts or cloud sync of its own. Sync comes from the folder the user picks.
- No multi-user collaboration.

## Success criteria

> TODO (kickoff): measurable signs that it works (e.g. a battlemap room edit takes N minutes, no visible seams at 100% zoom, the library opens in under N s with N assets).

## MVP scenario: the first milestone loop (draft)

This scenario is the acceptance test and the regression fixture.

1. Open a real battlemap image from a library folder. → It displays and can be zoomed and panned at full fidelity.
2. Paint a mask over one room. → The mask is visible, adjustable and undoable.
3. Enter a localised change ("turn this room into a kitchen") and submit. → Only the masked region (plus context) is sent. Cost and job status are visible.
4. Compare the returned candidates against the original, before/after. → Pick one.
5. Accept it. → It becomes a positioned **patch layer**. The original pixels are untouched.
6. Save, close and reopen the **asset project**. → The layers, mask, prompt, settings and revision history are all intact.
7. Export a flattened image for the VTT. → The patch blends without visible seams, at the original resolution.

> TODO (kickoff): KQ-20 to KQ-24 refine this scope (map sizes, export targets, which steps may use a fake provider).

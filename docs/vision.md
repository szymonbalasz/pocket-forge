# Vision

> Status: **accepted** at the phase 1a gate (2026-11-07). Change it only with the owner's OK.

## Positioning

A desktop **asset studio** for preparing D&D sessions. It produces high-quality, consistent campaign art through deliberate iteration and precise control, rather than one-shot generation.

- Primary asset types: **battlemaps, tokens, item art**. **Cards** are a possible extension.
- All asset types share one **library** and one editing foundation, with specialised controls and presets where they help.
- Generation provider: **Scenario** (for now).

## Users

- **The owner**, a DM preparing their own sessions, is the only design target (#8).
- Others may install it with their own provider account. That's welcome, but it doesn't drive any decisions.

## What it replaces

There's no established tool workflow to carry over (#10). Today's material is:
- a written prompt helper kept in the owner's campaign notes,
- a handful of reference images,
- one-off generations in a general-purpose chat assistant.

Pocket Forge turns that into **repeatable production**. The prompt helper becomes art recipes, the loose references become role-tagged reference images, and one-off generations become iterative, recoverable edits.

## Principles

- **Quality comes from controlled iteration.** Preserve approved details, compare alternatives, reuse successful recipes, and review art at its intended display size.
- **Any image is editable immediately**, wherever it was made.
- **The user owns plain files.** The library is a user-configured folder, typically cloud-synced. Each asset project is plain folders and files (#13). It must stay fast with a very large number of images.
- **Nothing accepted is lost.** Originals and previously accepted versions stay recoverable.
- **Reference roles stay distinct.** A style reference never dictates layout or replaces a character's identity.
- **Honest controls.** The app shows only the controls the selected model actually supports.
- **Visible cost.** Generation cost and job status are always shown. Retries are bounded and under the user's control.
- **No AI beyond the provider.** The app assembles and shows prompts and never writes them for you (#9).
- **No browser, no server to manage.** It is an installed app. Its UI may use web technology inside the app window.

## Non-goals

- It is not a virtual tabletop. It exports *to* VTTs.
- It is not a general-purpose image editor. It has a focused, practical toolset.
- No hosted service, accounts or cloud sync of its own. Sync comes from the folder the user picks.
- No multi-user collaboration.
- No in-app AI assistant (prompt drafting, critique) (#9).
- No integration with external notes or vault tools (#12).

## Success criteria

Accepted at the phase 1a gate (#35):

- **No visible seam** at 100% zoom after an accepted patch.
- **A typical map** (#30) opens and is usable in **under 2 s**; a 16k map in under 5 s.
- **Mask to accepted patch** takes under **5 minutes** of the owner's time, excluding generation time.
- **Save and reopen is pixel-identical**: layers, masks, prompts, settings and history.

## MVP scenario: the first milestone loop (draft)

This scenario is the acceptance test and the regression fixture.

1. Open a real battlemap image from a library folder. → It displays and can be zoomed and panned at full fidelity.
2. Paint a mask over one room. → The mask is visible, adjustable and undoable.
3. Enter a localised change ("turn this room into a kitchen") and submit. → Only the masked region (plus context) is sent. Cost and job status are visible.
4. Compare the returned candidates against the original, before/after. → Pick one.
5. Accept it. → It becomes a positioned **patch layer**. The original pixels are untouched.
6. Save, close and reopen the **asset project**. → The layers, mask, prompt, settings and revision history are all intact.
7. Export a flattened image for the VTT. → The patch blends without visible seams, at the original resolution.

Scope notes: typical maps are ~30×40 cells (#30). Export is plain image files at a chosen px per square (#26). CI uses the fake provider, and acceptance includes one manual real call (KQ-24).

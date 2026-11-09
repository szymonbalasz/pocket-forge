# UI guidelines

> Owner decisions #44–#46. Every UI builder follows this. Change it through the orchestrator.

## Look
- Neutral dark grey, Photoshop-like. Nothing competes with the art. Colour only for state (accent, warning, error).
- Radix primitives (`radix-ui`) for dialogs, menus, selects, sliders, tooltips, toggles and scroll areas. Our own CSS; no Tailwind, no other component kit.
- Icons: inline SVG components in `app/src/shell/icons.tsx` (no icon package).
- Font: `system-ui`, 13 px base, 11 px for meta text. Tabular numbers for sizes, costs and zoom.

## Tokens (`app/src/shell/theme.css`, CSS variables only, no hard-coded colours in features)
| Token | Value | Use |
|---|---|---|
| `--pf-bg-0` | `#1b1b1b` | canvas surround |
| `--pf-bg-1` | `#232323` | app, rails |
| `--pf-bg-2` | `#2b2b2b` | panels |
| `--pf-bg-3` | `#353535` | inputs, hover |
| `--pf-border` | `#3d3d3d` | dividers |
| `--pf-text` | `#e4e4e4` | text |
| `--pf-text-dim` | `#9a9a9a` | meta |
| `--pf-accent` | `#4a90e2` | selection, primary button, focus ring |
| `--pf-warn` | `#e0a030` | cost and threshold warnings |
| `--pf-danger` | `#e05050` | errors, destructive |
| `--pf-ok` | `#4caf6a` | succeeded |
| `--pf-radius` | `4px` | |
| `--pf-space-1..4` | `4/8/12/16px` | |

## Layout
```
┌────┬─────────────┬──────────────────────────────┬──────────────┐
│rail│ left panel  │ toolbar │ canvas             │ right panel  │
│    │ (project)   │         │                    │ (generate,   │
│    │             ├─────────┴────────────────────┤  export)     │
│    │             │ bottom: picker / iterations   │              │
│    │             │ status bar: zoom, mask, job   │              │
└────┴─────────────┴──────────────────────────────┴──────────────┘
```
- **Section rail** (56 px): Maps, Tokens, Items, **References**, then Settings at the bottom. Owner: asset-project (PRJ). References is not a section: selecting it shows the packs home (reference-packs, REF) in the home slot.
- **Home** (no project open): a grid of project cards for the selected section (thumbnail of the working image, name, iteration count, last edited) and **+ New**, a menu: **From image…** | **Generate new…** (OQ-GEN-10, GEN's name dialog). Owner: PRJ.
- **Left panel** (240–360 px, resizable): the project's folders (Working, References, Structure, Parts, Results), then History (revisions) and Storage (disk use). Owner: PRJ.
- **Canvas, toolbar, status bar, candidate picker:** editor-canvas (EDT).
- **Right panel** (300–380 px): Generate, then Export (EXP), as collapsible sections. Inside Generate, the **References block** (owned by REF, rendered through REF's `index.ts`) sits between Model and Prompt in each tab and is hidden when the model has no reference slots. The multi-select pack picker uses Radix Popover.
- **Prompt block** (prompt-helper, PRM): the Prompt part of each Generate tab is `PromptBlock`, owned by PRM and rendered by RGN and GEN through PRM's `index.ts`, like the References block. It is controlled: the host owns the typed prompt and the helper ticks and saves them with its draft; the host passes its own rows (RGN's blend toggle) as children. Helpers are toggle chips (`aria-pressed`) ticked like references. "Final prompt (as sent)" comes from `prm_compose_preview`, never from a provider call.
- **Settings sections:** features add a section to the settings dialog with `registerSettingsSection` (`app/src/shared/settings.ts`), e.g. RGN's **Region edit** (blend instruction) and PRM's **Prompt helpers**.
- **Generate tab strip** (OQ-GEN-09): **Region edit** (RGN) | **New image** (GEN). RGN's Generate section draws the strip (`gen-tab`, `role="tab"`, ←/→ move) and its own tab; other features add tabs with `registerGenerateTab` (`app/src/shared/generateTabs.ts`). Only the visible tab is mounted (FR-GEN-36). A project with no image shows only New image. A project opens on Region edit, except after **+ New → Generate new…** (New image). Retry on a generate job in the tracker switches to New image.
- The right panel hides itself while every component in it reports idle through `useSlotIdle(idle)` (`app/src/shared/slot.ts`), for example on Home with no project open (BL-15). A component that doesn't call it keeps the panel open.
- **Iteration tracker:** the bottom strip lists iterations (`results/NNN`). Owner: RGN, using EDT's picker for the selected iteration.
- **Status bar** is shared: features register **segments** (EDT zoom and mask, PRJ save state, RGN job state, PRV provider chip) through the feature registry, ordered by a `priority` number. EDT owns the bar itself.
- Slots live in `app/src/shell/Layout.tsx`; features register into a slot (or a status-bar segment) through the feature registry. A feature never renders into another feature's slot.

## Behaviour
- Keyboard first: every tool has a shortcut (see each spec). Shortcuts are ignored while a text field has focus (R23).
- Focus ring is always visible (`--pf-accent`). Every dialog has Esc to close and a default focus.
- Long operations show progress in the status bar, never a blocking modal, except the spend confirm dialog.
- `data-testid` per AGENTS §9.
- **Prompt chat window** (R92): its own window (`prompt-chat`, created from Rust), same tokens and components as the main window, no slots or rail; it follows the main window's project and visible tab.

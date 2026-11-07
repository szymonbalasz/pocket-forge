# Kickoff questions

**Kickoff is closed** (phase 1a gate, 2026-11-07). Only KQ-39 (motion source) stays open, for the animation spec. The owner answered these before any spec was written. Each has a **pick** we keep unless the owner overrides it. Answers move into the vision, tiers, ADRs or decisions log, and the question is then marked done.
Questions marked ✅ are answered, ⏳ are in progress. Each question is tagged by what it blocks: **V** (vision/tiers), **S** (spikes/ADRs), **M** (milestone scope).

## A. Product and audience

| ID | Question | Options (pick in **bold**) | Blocks |
|---|---|---|---|
| KQ-01 ✅ | Who is it for? **Answered: built for the owner. Others are welcome, but they don't drive the design (#8).** | **A. You first, but installable by any DM with their own Scenario account** / B. strictly personal / C. a polished product for others from the start | V (onboarding, key handling, installer, docs) |
| KQ-02 ✅ | **Answered: MIT; Pintura dropped (#17).** Licence? Pintura is commercial and can't ship in an open-source build without a per-app licence key. | **A. MIT or Apache-2.0 and drop Pintura** / B. a copyleft licence (AGPL) / C. source-available, keep Pintura | S |
| KQ-03 ✅ | **Answered: no AI in the app (#9).** Is any LLM inside the app (prompt drafting from recipes, critique of results)? | **A. None in the MVP. Prompts are assembled from the recipe and visible** / B. optional assistant later (tier B) | V, S |
| KQ-04 ✅ | **Answered: REST API from the Rust core, per [ADR 0001](architecture/adr/0001-provider-integration.md) (#14).** "Scenario via MCP": MCP is built for LLM agents. Should the app call Scenario directly (REST) and keep MCP for agent-driven automation, or speak MCP itself? | **A. Decide in `spike/scenario-api`. My default is a REST adapter in Rust** / B. MCP client in-app regardless | S |
| KQ-05 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Where do Scenario credentials live? | **A. OS credential store (Windows Credential Manager, later macOS Keychain)** / B. a config file | S |
| KQ-06 ✅ | **Answered: A (#23).** Spend control: does Scenario give a cost estimate before a job, and do you want a cap? | **A. Show estimate + per-session cap, with confirmation above a threshold** / B. show the cost after the job only | M |
| KQ-07 ✅ | **Answered: no existing tool workflow. Today there's a prompt-helper doc in the owner's campaign vault, some reference images, and ChatGPT image generation (#10).** What does your workflow look like today, and what hurts most? This decides what "replaces" means and which tier-A items rise. | free answer | V |

## B. Files, library and history

| ID | Question | Options (pick in **bold**) | Blocks |
|---|---|---|---|
| KQ-10 ✅ | **Answered: plain folders and files per asset. No single-file packages or databases (#13).** Asset project on disk? | **A. A sidecar folder per asset (`<name>.pforge/`: JSON manifest + image blobs), with the original untouched next to it** / B. a single package file (zip) / C. a central database | S |
| KQ-11 ✅ | **Answered: the original is left alone; the app works only in the library folder; adding images must be frictionless (#16).** Opening a plain PNG: where does its project go? | **A. Next to the image, created on first edit** / B. a separate projects folder that links to the image | S |
| KQ-12 ✅ | **Default kept: A, no objection.** Library index (thumbnails, search): synced or local? SQLite on OneDrive risks corruption. | **A. A local, rebuildable cache outside the synced folder. Tags and collections live in synced files** / B. everything in synced files | S |
| KQ-13 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Multi-machine use through OneDrive: is it in scope? | **A. Yes, one machine at a time. Detect OneDrive conflict copies and warn** / B. single machine only | S |
| KQ-14 ✅ | **Default kept: A, no objection.** OneDrive "files on demand": may the app trigger downloads for thumbnails? | **A. No. Show a cloud placeholder and download on open** / B. hydrate everything | M |
| KQ-15 ✅ | **Answered: A (#18).** What is kept from generation? | **A. Accepted revisions + starred candidates forever. Others are purged after N days (user-visible)** / B. keep everything | S |
| KQ-16 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Is undo history persisted across sessions? | **A. No. The revision history covers it; undo is per session** / B. yes | M |
| KQ-17 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Every generation records model id, seed and all parameters for reproduction? | **A. Yes, always** / B. only on accept | S |
| KQ-18 ✅ | **Answered: no vault integration; a fresh, standalone library (#12).** Campaign subjects (NPCs, locations): defined in this app, or imported from a tool you already use (Obsidian, World Anvil, Foundry…)? | **A. A simple list in the library for now** / B. integrate with your tool | V |

## C. Images, editing and export

| ID | Question | Options (pick in **bold**) | Blocks |
|---|---|---|---|
| KQ-20 ✅ | **Answered: 5–10k px per side typical, up to about 16k; 70–140 px per square (#11).** Typical and maximum battlemap size (pixels, grid size, pixels per square)? | free answer. My guess: 5–10k px per side, up to about 16k | S, M |
| KQ-21 ✅ | **Answered: the owner's own custom VTT (#24); plain image files (#26).** Which VTT(s), and their limits (max dimensions, format, file size)? | **A. Generic PNG/WebP/JPEG at a chosen px-per-square** / B. specific VTT presets now | M |
| KQ-22 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** The milestone says "a real battlemap", but the repo is public. Test fixtures must be generated or CC0. OK to keep your own maps out of the repo and use them for manual playtests only? | **A. Yes** / B. you have shareable maps | M |
| KQ-23 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Patch seams: how should edges blend? | **A. Feathered mask edge + optional colour match, tunable per patch** / B. a hard edge (the model blends) | S |
| KQ-24 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Does the milestone loop need a *real* Scenario call, or is a fake provider enough for acceptance, with a real call as a manual check? | **A. Fake provider in CI; one manual real call at acceptance** / B. real calls in acceptance | M |
| KQ-25 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Who composites? The canvas preview and the Rust export must match. | **A. Rust is the authority for flatten/export; the canvas approximates; golden-image tests** / B. composite in the UI only | S |
| KQ-26 ✅ | **Tier B, via the accepted tiers (#29).** Grid awareness (pixels per square, snapping patches to cells): in the MVP? | **A. Tier B: store and overlay only** / B. MVP | V |
| KQ-27 ✅ | **Tier B, notes for the owner, via the accepted tiers (#29).** Notes pinned to image locations: notes for you, or instructions fed to the edit prompt? | **A. Notes for you (tier B)** / B. instructions into the prompt | V |
| KQ-29 ✅ | **Answered: image files only, no sidecar. The grid size is set in the VTT; its limits are generous (#26).** What does the owner's custom VTT import, and what should export emit? | **A. Image file(s) plus a small JSON sidecar (px per square, grid offset, size in squares, animation info)** / B. image only | M |
| KQ-39 ⏳ | Animation: what gets animated, where does motion come from, and which tier? **Partly answered (#27): everything eventually, after the first milestone, tokens first. Motion source pending `spike/scenario-video`.** | | V, S |
| KQ-28 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Input formats? | **A. PNG, JPEG, WebP (no PSD)** / B. also PSD/layered | M |

## D. Engineering and process

| ID | Question | Options (pick in **bold**) | Blocks |
|---|---|---|---|
| KQ-30 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Frontend framework? | **A. React + TypeScript + Vite (largest ecosystem, react-konva, most agent-familiar)** / B. Svelte / C. Solid | S |
| KQ-31 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Package manager? | **A. pnpm (via corepack)** / B. npm | S |
| KQ-32 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Canvas library? | **A. Decide in `spike/canvas-large-image`: Konva vs Fabric vs PixiJS on a tiled 10k+ map** | S |
| KQ-33 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** E2E tests for a desktop app (there's no browser to drive)? | **A. Playwright on the web UI with mocked Rust IPC for most flows, plus a few `tauri-driver` smoke tests on Windows CI** / B. tauri-driver for everything | S |
| KQ-34 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** CI runners? | **A. Rust core + UI tests on Linux (fast); app build + smoke on `windows-latest`; add macOS later** / B. Windows only | S |
| KQ-35 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Installer signing (unsigned = SmartScreen warning) and auto-update? | **A. Unsigned, no auto-update for now** / B. sign from the start | C |
| KQ-36 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Telemetry or crash reporting? | **A. None** / B. opt-in crash reports | V |
| KQ-37 ✅ | **Answered: local `--no-ff` merges after green branch CI; no branch protection (#28).** Merge flow: PRs with required checks and auto-merge when green, or direct `--no-ff` merges after the CI watch? | **A. PRs + branch protection + `gh pr merge --auto`** / B. direct merges | process |
| KQ-38 ✅ | **Pick accepted at the phase 1a gate (#33–#34).** Worker agent kind and concurrency? | **A. Claude workers, max 4 concurrent** / B. mixed kinds | process |

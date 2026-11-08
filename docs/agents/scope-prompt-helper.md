# Scope note: prompt helper (BL-37)

> Status: **scoping**, to flesh out with the owner. Nothing here is decided.

## Why

In the first real session (2026-11-08), two prompt aids made a large difference to masked edits: the advice to describe the result rather than give an instruction, and the blend instruction (#55). The owner wants this kind of help inside the app.

## Two parts

| Part | What | Needs AI? |
|---|---|---|
| **A. Prompt helpers (presets)** | A few predefined helpers, picked like references: "Describe the result", "Top-down battlemap", "Match surrounding style", "No people/tokens", … Each one adds visible text to the final prompt. Editable, per asset kind. | No. Close to the tier-A **art recipe**; fits #9 ("prompts are assembled from recipes"). |
| **B. Prompt chat** | A pop-out chat window. The assistant sees the map region (and context), the mask, the current prompt, the model and its controls. The user chats about the intent, and the assistant proposes a prompt that the user can insert in one click. | Yes. **Supersedes #9** (no AI/LLM in the app, KQ-03), which needs an owner decision and a tier change. |

## Open questions (for the owner)

| # | Question | Options (tradeoff) | Pick |
|---|---|---|---|
| OQ-PH-01 | Is part A in the MVP? | A. Yes, small and no AI / B. Later with art recipes (tier A) | **A** |
| OQ-PH-02 | Supersede #9 for part B? | A. Yes, an optional assistant, off unless configured / B. No, part A only | owner |
| OQ-PH-03 | Which LLM backend? | A. **Claude Code headless** (`claude -p`, image via file path): no new key, uses the owner's existing install and plan; local-only and needs Claude Code installed / B. **OpenRouter API**: any vision model, one HTTP API, but a second key in the credential store and per-call cost / C. Anthropic API direct: same as B, one vendor | **A first, behind an adapter so B can follow** |
| OQ-PH-04 | What the assistant sees | A. The sent crop (region + margin) with the mask overlaid, plus the prompt, model and controls / B. Also the whole map, downscaled | **A** (smaller and cheaper; B as an option) |
| OQ-PH-05 | Privacy and cost | Map crops leave the machine to the LLM vendor. Show this once, and show the cost per message if the backend reports it. | **Notice + opt-in** |
| OQ-PH-06 | Where it lives | A. A pop-out window (Tauri second window) / B. A drawer in the Generate panel | owner |
| OQ-PH-07 | Glossary | New terms: "prompt helper", "prompt chat" (§2) | add at the gate |

## Notes

- **Architecture:** a core command per capability (AGENTS §9). The LLM backend is an injected seam like the provider, with a fake for CI. No real LLM calls in CI (§3.7).
- **New dependencies:** none for part A. Part B with Claude Code headless is a child process (no crate). OpenRouter would reuse the existing HTTP client.
- **Final prompt rule (FR-RGN-09):** the assistant only proposes. Inserting is a user action, and the job still records exactly what was sent.

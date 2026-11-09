# Seams: phase 4 (new-asset generation + prompt helper)

Decisions: [#52](../decisions-log.md), #58, #59. Specs: [new-asset-generation](../specs/new-asset-generation.md) (GEN), [prompt-helper](../specs/prompt-helper.md) (PRM).

| Seam | Owner | Consumer | What's agreed |
|---|---|---|---|
| Generation job (text/references/structure → candidates) | GEN | PRV | GEN needs a new PRV operation (e.g. `generate`) next to `edit_masked`. GEN writes the needed PRV amendment as an open question naming PRV. The capability-driven-controls rule (#37) applies unchanged. |
| Job pipeline, spend cap, candidates, iteration tracker, keep-all (#56) | RGN / PRJ (existing) | GEN | Reused as is. GEN says only where it differs (no region, no mask, no patch layer). |
| Where a picked result goes | GEN | PRJ | A new asset project from a picked candidate, or the working image of an existing project. GEN decides, with PRJ amendments as an open question. |
| References, packs, structure | REF (existing) | GEN | Same References block and roles (#51). The project's STRUCTURE folder feeds a structure input where the model has one. |
| Prompt block (typed prompt, helpers, final prompt preview, chat button) | PRM | RGN, GEN | PRM defines the helpers and the chat. RGN and GEN host the same prompt block. The blend instruction (#55) stays RGN-only. |
| What the chat sees | PRM | RGN, GEN | RGN: sent crop + mask overlay, prompt, model + controls. GEN: references, structure, kind, prompt, model + controls. Both provide this context through a core command. |
| LLM backend | PRM | core | An adapter seam like the provider: Claude Code headless first, with a fake for CI. No real LLM calls in CI (AGENTS §3.7 applies by analogy). |
| Second window, process spawn | PRM | orchestrator | `tauri.conf.json` and capabilities are orchestrator-only. PRM lists what it needs. |

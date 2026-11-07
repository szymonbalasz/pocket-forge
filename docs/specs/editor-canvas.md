---
feature: editor-canvas
prefix: EDT
owner: spec-editor-canvas
status: claimed   # unclaimed → claimed → draft → in-review → complete (owner only)
depends_on: [asset-project]
---

# Editor canvas

## 1. Summary
Three to five lines: what it is and why it matters.

## 2. Scope
Tier S items covered. Link to [feature-tiers](../feature-tiers.md).

## 3. Goals

## 4. Non-goals

## 5. User stories
"As a DM preparing a session, I…"

## 6. Functional requirements
| ID | Requirement (testable) |
|---|---|
| FR-EDT-01 | |

## 7. UI/UX notes
Layout and controls. States: empty, loading, job running, error, offline, provider unavailable.

## 8. Data and file effects
Entities, fields and relations. What is written to the asset project and where. What is recoverable. Derived and cached data (rebuildable?).

## 9. Commands and events
Rust commands (name, inputs, validation, result, errors) and the events emitted to the UI.

## 10. Provider interaction
Calls to the provider, the model capabilities required, cost display, job lifecycle, retries. "N/A" if none.

## 11. Edge cases
Crash mid-save · cloud sync conflict / file locked · offline · provider error or timeout · very large image · corrupt or unsupported input · undo across the feature boundary.

## 12. Performance budget
Image sizes, latency and memory targets.

## 13. Test plan
- Unit (Rust / TS), integration, thin e2e.
- Fixtures: public-safe images only (generated or CC0). The fake provider and recorded responses.
- **App walkthrough:** steps, expected results.

## 14. Build split
Slices in order, the shared files each touches, and dependencies.

## 15. Open questions
| ID | Question | Options (pick in **bold**) | Blocking? | Affects |
|---|---|---|---|---|
| OQ-EDT-01 | | **A. …** / B. … | | |

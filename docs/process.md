# Process

How the owner and a team of coding agents build Pocket Forge. Specs come before code. One orchestrator coordinates. Short-lived workers each do one task in their own worktree.

## Roles

| Who | Does | Never does |
|---|---|---|
| **Owner** (human) | Product decisions. Accepts ADRs, sets specs to `complete`, says "go" at every gate, playtests | Day-to-day merging or sequencing |
| **Orchestrator** | Plans, writes briefs, spawns agents, merges green branches locally, numbers ADRs, keeps the shared docs, brings decisions to the owner | Feature work |
| **Spec agent** `spec-<feature>` | Writes ONE spec on `spec/<feature>` | Code; editing other specs |
| **Review agent** `review-<topic>` | Read-only checks against the rules | Editing files |
| **Spike agent** `spike-<topic>` | Throwaway proof that answers one technical question; writes the findings and a proposed ADR | Production code |
| **Builder** `build-<feature>` | One feature or slice in thin vertical slices on `build/<feature>` | Shared files without permission |
| **Test agent** `test-<feature>` | Runs one app walkthrough in the builder's worktree and collects evidence | Fixing code |

## Phases and gates

Each gate needs the owner's explicit "go". See [roadmap.md](roadmap.md) for the phases.

Per-phase cycle: **spec → review → owner gate → consolidate → build → demo gate.** Specs are written just in time. Phase N+1 is specced while phase N builds.

**Gate doc format** (`docs/agents/gate-<phase>.md`):
- A summaries table: item, a 3–5 line summary, the reviewer verdict.
- Decisions D1, D2…, each with 2–3 options, tradeoffs and a **bold pick**.
- "Accept the remaining picks as written": a table of `OQ-id | pick`.
- Non-blocking defaults the orchestrator will keep unless the owner objects.
- Items resolved by the integrator.
- What happens next after the answers.

## CI and merging

See [AGENTS.md §5](../AGENTS.md#5-ci-and-merging). In short:
- CI on GitHub Actions runs everything, split evenly across parallel jobs.
- Local runs stay targeted.
- Branches merge locally (`--no-ff`) only when their CI is green (#28).
- A red `main` is fixed before any new work.

## Orchestrator working files

| File | Purpose |
|---|---|
| [agents/orchestrator-prompt.md](agents/orchestrator-prompt.md) | The orchestrator's job description |
| [agents/status.md](agents/status.md) | Scratch board: tasks, agents, worktrees, blockers, pending decisions, locks. Overwritten as things change; not history |
| [agents/backlog.md](agents/backlog.md) | Small `BL-nn` items for idle slots |
| `agents/continue-session.md` | Handover note for a fresh orchestrator session |
| `agents/tools/` | Helper scripts that grow out of repeated chores |

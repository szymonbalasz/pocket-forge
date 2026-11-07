# AGENTS.md

Read this first. It applies to every agent (orchestrator, spec, review, builder, test) and to human contributors.
`CLAUDE.md` only imports this file. Put new rules here.

Project: **Pocket Forge**. It is a desktop asset studio for preparing D&D sessions. It generates, refines, organises and reuses campaign art (battlemaps, tokens, item art).
Start with [docs/vision.md](docs/vision.md), [docs/feature-tiers.md](docs/feature-tiers.md) and [docs/process.md](docs/process.md).

---

## §1 How we talk

- We are two people discussing an idea. Keep replies short. If a reply needs more than about 15 lines, lead with the one decision that matters.
- Ask one or two questions at a time. When there are options, give 2–3, each with a one-line tradeoff, and **state your pick**.
- **Never present something undecided as decided.** Raise it as an open question.
- Batch decisions at gates. Don't drip-feed them.

## §2 How we write docs

- Each doc should be readable in five minutes. Use bullets over prose and tables where they help. Draw diagrams in Mermaid.
- Use only terms from [docs/glossary.md](docs/glossary.md). Don't invent synonyms. A new term starts as an open question.
- Use relative links.
- Make placeholders explicit: `> TODO (<phase>): …`.
- Number decisions in [docs/decisions-log.md](docs/decisions-log.md) and cite them by number (`#7`) instead of re-arguing them.

## §3 Hard rules

1. **This is a public repository.** Never commit anything personal or machine-specific:
   - absolute paths from anyone's machine (home folders, temp folders, OneDrive paths),
   - personal emails, usernames, machine names or campaign content that belongs to someone,
   - API keys, tokens, `.env` files or Scenario credentials.
   Use placeholders such as `<library-root>` or `<home>`. Personal working files belong outside the repo (for example in `%TEMP%/pocket-forge/`). `scripts/check-hygiene.sh` enforces part of this rule in CI.
2. **Accepted ADRs are law.** Change one only through a superseding ADR, and only the owner accepts ADRs.
3. **Scope is tier S** in [docs/feature-tiers.md](docs/feature-tiers.md). Anything else needs the owner's OK.
4. Agents propose and the owner decides. The orchestrator may settle trivial, local, technical questions itself. It logs each one as an *integrator ruling* and reports it at the next gate.
5. **Stop and ask** before starting a new phase, accepting or superseding an ADR, adding a new dependency, or doing anything destructive or outward-facing. If you're unsure, ask.
6. Never approve another agent's permission prompt on the owner's behalf.
7. Generation tests never spend real Scenario credits in CI. Use the fake provider and recorded fixtures.

## §4 Git, worktrees and agents

- **One task = one worktree = one agent = one branch.** Run at most about 4 agents at once, test agents included.
- Branch names: `spec/<feature>`, `build/<feature>`, `fix/<slug>`, `spike/<topic>`, `docs/<slug>`.
- **With herdr** (`HERDR_ENV=1`): create worktrees with
  `herdr worktree create --workspace "$HERDR_WORKSPACE_ID" --branch <branch> --base main --no-focus`.
  Always pass the workspace ID explicitly, and check that the new worktree belongs to this repo before you use it. Start workers with `herdr agent start <name> --kind claude --pane <id>`.
- **Without herdr:** use plain `git worktree add ../pocket-forge-wt/<branch> -b <branch> main` (or a normal branch checkout). Follow the rest of this file as usual.
- The orchestrator stays on `main` at the repo root and doesn't do feature work.
- Briefs and long reports are files outside the repo, sent by path. Reports are ≤ 20 lines.
- Prefer fresh agents. End an agent when its task merges, then remove its worktree.

## §5 CI and merging

- **CI runs everything.** Locally, run only targeted checks (the tests touched by your diff, plus the one e2e test you're working on). Run heavy suites in CI only.
- Spread suites evenly across parallel GitHub Actions jobs and shards. When one job runs longer than about twice the median, rebalance.
- **Push the branch. CI runs on every branch push.** Watch it in the background (`gh run watch <run-id> --exit-status`, or `gh run list --branch <b>`). Never sleep or poll in the foreground.
- **Merge locally, and only when the branch's CI is green.** The orchestrator runs `git merge --no-ff <branch>` on `main`, pushes, and tells active builders to rebase. PRs are optional, for example for outside contributors. No branch protection; use common sense (#28).
- **A red `main` gets fixed first**, before any new work.
- A test that passes only on retry is reported as **flaky** and gets a backlog row.

## §6 Spec work

- Specs are written just in time: phase N+1 is specced while phase N builds.
- Each spec agent writes ONE spec from [docs/specs/_template.md](docs/specs/_template.md) and edits only that spec.
- Front matter is the claim board: `status: unclaimed → claimed → draft → in-review → complete`. **Only the owner sets `complete`.**
- Number requirements `FR-<PREFIX>-NN`, and make each one testable.
- Open questions go in the spec's table, each with a **bold pick** and a one-line tradeoff. Never invent a decision.
- If you need something from another spec, record it as an open question that names that spec. Cross-cutting topics go to `docs/architecture/adr/proposed-<slug>.md`.

## §7 Review work

- Reviewers are read-only. They check specs or code against §8, the accepted ADRs, the glossary, tier S and the related specs.
- They report contradictions, scope creep, untestable FRs and cross-spec conflicts. Reports are ≤ 20 lines, most important first.

## §8 "Spec complete" checklist

- [ ] Every section is filled, or marked N/A with a reason. No TODOs remain.
- [ ] FRs are numbered and testable.
- [ ] The data and file effects are complete: what is written where, and what is recoverable.
- [ ] Every command/event is described, with failure behaviour.
- [ ] Crash mid-save, provider failure, offline and sync-conflict behaviour are stated.
- [ ] The test plan names fixtures (public-safe, no personal art) and an app walkthrough.
- [ ] No blocking open questions remain.
- [ ] It is consistent with the ADRs and the glossary, and readable in five minutes.

## §9 Build conventions

> TODO (phase 1): to be drafted by the orchestrator once the stack ADRs are accepted. It will cover layout and import boundaries, per-worktree isolation (ports and data dirs), fixtures, test commands and naming (`FR-…` in test names), test IDs, lint/typecheck, the shared-files table, and "done" for a build task.

# Prompt chat probe: first run (owner runbook)

PRM slice 0 ([prompt-helper spec](../specs/prompt-helper.md) §10, §14). `pf-chat-probe` checks the Claude Code claims the prompt chat depends on: isolation first (FR-PRM-45), then the other claims. It also records the fixtures that slice 4 replays. It runs on your machine only, never in CI. Steps 1–2 are free. Step 3 uses a little of your Claude plan: six short turns, one of them with a 512×640 image. Run every command from the repo root.

## 1. Check the CLI (free)
```sh
cargo run -p pf-chat --features real-call --bin pf-chat-probe -- status
```
- It prints where `claude` was found, its version and whether the chat's own config folder is logged in. Expect `logged in: no` on the first run.
- The folder is `<data-dir>/claude-config/`. `<data-dir>` is `--data-dir`, else `PF_DATA_DIR`, else `.pf-dev/data` (the dev builds' default, so the login is reused).
- If `claude` isn't found, add `--claude <path-to-claude.exe>` to every command.

## 2. Log in once, into the chat's own folder (free)
```sh
cargo run -p pf-chat --features real-call --bin pf-chat-probe -- login
```
- This runs `claude auth login` with `CLAUDE_CONFIG_DIR=<data-dir>/claude-config/`. Your own `<home>/.claude` login and settings are untouched. Pick the account and plan the chat should use.
- The probe never reads the credentials. It prints only *logged in*, the auth method, the plan, and whether `configDirectory` is the isolated folder.
- If `ANTHROPIC_API_KEY` is set in your shell, Claude Code uses it instead of the plan login and bills the key. Unset it first (PowerShell: `Remove-Item Env:ANTHROPIC_API_KEY`).

## 3. Run the checks (uses plan usage)
```sh
cargo run -p pf-chat --features real-call --bin pf-chat-probe -- run
```
| Session | What it does | Cost |
|---|---|---|
| empty config | `auth status` and one turn in a fresh, empty folder → *not logged in* | free |
| isolation | §10's exact invocation (`--safe-mode`, home = the empty `<data-dir>/chat-home/`), cwd under a stand-in folder with marker files (a `CLAUDE.md` in `.claude/` and in a parent folder, a hook, a skill, an agent, a command, a plugin, an MCP server), with `--debug-file` | 1 short turn |
| safe-mode layer | `--safe-mode` alone: the stand-in is the home folder, and an `AGENTS.md` marker sits above the cwd (in the OS temp folder, away from the repo's `CLAUDE.md`) | 1 short turn |
| control | The same without `--safe-mode`: the markers must show up, or the two sessions above prove nothing | 1 short turn |
| main | §10's exact invocation, your real environment (home replaced by the chat home): turn 1 with the overlay image, turn 2 text only, in the same process | 2 turns |
| cancel | Same list plus `--include-partial-messages`, killed after 5 text deltas | 1 turn, cut short |
| `--max-turn` (optional) | Six noise images, ~4 MiB (FR-PRM-25's worst case) | 1 turn, heavier |

- Each turn has FR-PRM-36's budgets (60 s to the first event, 90 s between events, 300 s in total), so the run can't hang.
- The probe empties `<data-dir>/claude-config/projects/` at the start and after each session, as the app will. It reports anything it found there first.
- Exit code 0 means every gate row is verified and the minimum version is fixed. Exit code 2 means a gate failed, so the chat would ship disabled (`isolation_unavailable`).

## 4. Check the recordings (free)
- Recordings are written to `fixtures/chat/claude-code/`: `not-logged-in`, `success-image`, `two-proposals`, `partial-cancel`, and `max-turn` with `--max-turn`. A failed case is written as `<case>.failed.jsonl`.
- They are scrubbed per §10: ids become `<session-N>` / `<uuid-N>` / `<msg-N>`; paths and project slugs become `<data-dir>/…`, `<home>`, `<project-slug>` or `<path>`; init lists become `[]`; `apiKeySource` becomes `<redacted>`; e-mails become `<email>`. A file that still fails the leak check isn't written to `fixtures/`. It goes to `<data-dir>/chat-probe/leaky-*.jsonl`, and row C23 says so.
- Run `bash scripts/check-hygiene.sh` and `cargo test -p pf-chat`. The test re-checks every committed recording. Then read `git diff fixtures/chat`.
- `usage-limit` can't be forced. It stays a hand-made fixture (`fixtures/chat/handmade/`) unless you hit the limit one day.

## What to paste back
- The block between `===== paste from here =====` and `===== to here =====`, plus the exit code. It's also saved in `<data-dir>/chat-probe/summary.md`.
- The block holds no paths or account data. Row C10's samples are scrubbed debug-log lines; glance at them before pasting.
- Don't paste `<data-dir>/chat-probe/isolation-debug.log` or any stderr. They can hold account details.

## Reading the table
| Rows | Spec claim (§10) | If it fails |
|---|---|---|
| C01–C02 | Executable order; the version ≥ 2.1.268 | Fix the install or `--claude` |
| C03–C07 | `auth status` exit codes, the allowed fields, `configDirectory` | C06 failing = `isolation_unavailable` |
| C08–C11, C24–C25 | No CLAUDE.md, `AGENTS.md`, hook, skill, agent, command, plugin or MCP marker reaches the chat (C08: §10's invocation; C24: `--safe-mode` alone); C25 proves the markers are live; the real home adds nothing to `system/init` | **Gate**: the chat stays disabled (#60). If C25's detail says the `AGENTS.md` marker wasn't seen, a `CLAUDE.md` above your temp folder hid it: say so when you paste |
| C12–C14 | Base64 image blocks; `--system-prompt`; two turns with `--no-session-persistence` | Slice 4 changes the adapter |
| C15 | No transcript in `projects/`, after a clean exit and after a kill | **Gate**: `projects/` emptying is only a backstop |
| C16–C18, C20 | Exit code, usage fields, streaming granularity, timing budgets | Slice 4 adjusts |
| C19 | A kill closes stdout within 5 s (no orphan holds the pipe) | **Gate**: slice 4 needs the job object (§10) |
| C21 | A 4 MiB stdin line works (the unverified 10 MB cap) | FR-PRM-25's limits shrink |
| C22–C23 | API-key source; recordings written | Info |

- **Minimum version** = the installed version when every gate row is verified. Older versions were never checked.
- C10 is information only: lines in the debug log that mention the stand-in `.claude` paths. A path check isn't a read, but read the samples.

## Clean up
- `… pf-chat-probe -- logout` logs the chat's folder out. Deleting `<data-dir>/claude-config/` removes everything Claude Code kept there.
- `<data-dir>/chat-probe/` and `<data-dir>/chat/probe-thread/` are recreated on every run. `<temp>/pf-chat-probe-agents/` is removed after the isolation sessions.
- `<data-dir>/chat-home/` should stay empty; the summary lists what it holds.

# Real provider: first session (owner runbook)

The real Scenario provider runs on your machine only, never in CI (FR-PRV-28, [provider spec](../specs/provider.md) §13 "Manual acceptance"). The steps below are in spending order: steps 1–3 are free, and only step 4 spends credits, under a cap you set. Run commands from the repo root.

## 1. Enter the key (free)
- Start the app with `corepack pnpm dev:real` (run `corepack pnpm env:init` once first). The console prints a **REAL PROVIDER** banner. `PF_DATA_DIR` and `PF_LIBRARY_DIR` still point into `.pf-dev/`.
- Click the status chip (or **Settings**). Enter the API key and secret, then **Save**. They go to the OS credential store (`pocket-forge/scenario`) and nowhere else.
- Optional: enter the team id. Without it the balance shows `unknown`.
- Click **Test connection**. The chip should read `Scenario · ok`.
- Next to the chip, the **Real credits** badge stays visible as long as Scenario is active. It shows `no spend cap yet` until your first submit asks you for a spend cap and a per-job threshold.

## 2. Record the free calls (free)
```sh
cargo run -p pf-provider --features real-call --bin pf-record
```
- This fetches `GET /models/{id}` for every overlay model, then runs one dry run each (`?dryRun=true`, never `ipDetection`). Each dry run uploads the generated 512 px room and disc mask as hidden assets (no credits). It prints one line per model: `offered`, `NOT FOUND` or `NOT OFFERED`, the dry-run cost and any warnings.
- It writes `fixtures/provider/<model>/get-model.json` and `dry-run.json`. A failed call is saved as `<case>.failed.json` instead, so the committed fixtures stay as they are.
- It also runs one `generate` dry run (`dry-run-generate.json`) for every model with a generate row (GEN): the prompt, 1024 × 1024 or ratio `1:1`, one result, no sketch. Free; to run only this, add `-- --case dry-run-generate`.
- To set the team or project id for the recorder, use `PF_SCENARIO_TEAM_ID=<team-id>` and `PF_SCENARIO_PROJECT_ID=<project-id>`.

## 3. Check the recordings (free)
- Run `bash scripts/check-hygiene.sh`, then read `git diff fixtures/provider`.
- The recorder never captures the auth header. It replaces job, asset, upload, team, project and user ids with `<asset-1>`, `<team-id>` and so on, signed URLs with `fake://asset/N`, emails with `<email>`, and image bytes with `{"fixture": "room-512.png"}` (FR-PRV-27, Appendix A).
- Commit the recordings on a branch, or hand them to the orchestrator. A builder then fixes the overlay rows that are `NOT FOUND`/`NOT OFFERED` or whose tests fail on the real bodies.

## 4. One paid masked edit (spends credits)
```sh
cargo run -p pf-provider --features real-call --bin pf-record -- --case masked-edit --model model_ideogram-v3-balanced --allow-spend 20
```
- The dry run comes first. If the estimate is unknown or over `--allow-spend` (in CU), **nothing is submitted**.
- Otherwise it submits, polls until the job finishes (at most 15 minutes), fetches the candidates and writes `masked-edit.json`.
- `sent.png`, `mask-white-is-edit.png` and `candidate-N.*` go to `.pf-dev/real-call/<model>/`, outside tracked files.
- For each candidate it prints a mask check, either *edited inside the mask (OK)* or **EDITED OUTSIDE THE MASK** (inverted convention). Open the files and look either way.
- Optional: repeat with `--model model_openai-gpt-image-2` to check the alpha convention.
- Then try the same edit in the app (`pnpm dev:real`). The spend cap and confirm dialog apply as usual.

## Gated tests (optional, never in CI)
```sh
PF_REAL_CALL=1 cargo test -p pf-provider --features real-call --test real_call -- --nocapture
PF_REAL_CALL=1 PF_REAL_SPEND_CU=20 cargo test -p pf-provider --features real-call --test real_call -- --ignored --nocapture
```
- In PowerShell, set the variable first with `$env:PF_REAL_CALL=1`.
- The first command runs the free tests. The second runs the paid test, which is ignored by default. `PF_REAL_MODEL` picks its model (default V3 Balanced).
- CI fails if the `real-call` feature or `PF_REAL_CALL` is ever set there.

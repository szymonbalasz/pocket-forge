# Chat fixtures (PRM)

- `handmade/`: hand-written stream-json lines in the documented shape (PRM spec §10). They test the
  parser and the scrubber in CI. `{CWD}` is replaced at test time by a path that is then scrubbed.
- `claude-code/`: real recordings from the owner's CLI, written by `pf-chat-probe` and scrubbed per §10
  ([runbook](../../docs/guides/prompt-chat-probe.md)). Each file is the probe's stdout lines for one
  case. A failed case is written as `<case>.failed.jsonl`. Every file must pass `scripts/check-hygiene.sh`.
- Used by code: the fake backend (`crates/pf-chat/src/fake.rs`) and the adapter replay tests
  (`crates/pf-chat/tests/adapter_replay.rs`) embed some of these files, so renaming one breaks the build.

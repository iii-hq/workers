# Changelog

All notable changes to `iii-conductor`. This worker is in 0.x — field
shapes may change between any minor bump until 1.0.

## [0.1.0] — initial release

- `conductor::dispatch` — fan-out N agents in parallel, each in its own
  git worktree under `~/.iii/conductor/worktrees/`.
- `conductor::status` / `conductor::list` — read run state.
- `conductor::merge` — smallest-`finished_at` winner pick over the
  eligible set, loser worktree cleanup, winner branch survives.
- Local agent kinds: `claude`, `codex`, `gemini`, `aider`, `cursor`,
  `amp`, `opencode`, `qwen`. Default arg vector per kind, overridable via
  `bin` / `args`.
- `kind: "remote"` — any iii function id (typically registered by
  `iii-mcp-client` or `iii-a2a-client`) participates on equal footing,
  with worktree path passed as `cwd` in the trigger payload.
- Verifier gates: arbitrary iii functions of shape
  `(input: { cwd }) -> { ok, reason? }`. Results stored as ordered
  `Vec<GateRunResult>` so duplicate `function_id`s with different
  descriptions are preserved.
- Run state persisted under `state::set` scope `conductor`, key
  `runs::<run_id>`. Written after every agent transitions, not only at
  start and end.
- All four functions registered with `metadata.public = true` for MCP/A2A
  exposure via `iii-worker-manager`.

### Known gaps tracked for 0.2

- Stable fingerprint-based `run_id` so dispatch can be retried safely.
  Today it is fire-and-forget; the caller must dedupe via
  `conductor::list`.
- Streaming agent stdout to an `iii-stream` channel for live UI
  consumers. Today the only progress signal is polling
  `conductor::status`.
- A sibling `verify` worker (or `examples/`) shipping reusable verifier
  registrations for `cargo test`, `npm test`, `tsc --noEmit`, etc.

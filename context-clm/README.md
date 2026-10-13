# context-clm

`context-clm` lets a [`harness`](https://github.com/iii-hq/workers/tree/main/harness)
session manage its own context. Everything after the task is mirrored to a
file inside the session's working directory, `.clm/<session>.ctx`; the model
condenses stale turns by editing that file with ordinary shell commands, and
the edited file is its context on the next request. A controller keeps the
session under a token budget: the model sees its current size after each
command, it nudges as the context grows, warns when the next tool output
would not fit, rolls back the newest steps when the context crosses the
limit, and grants one final turn before ending the run. Every request still
passes through
[`context-manager`](https://github.com/iii-hq/workers/tree/main/context-manager)
for the model's limits and the final fit. Sessions that do not opt in are
unaffected, and sub-agents always use the default strategy.

## Install

```bash
iii trigger compose::add worker=context-clm
```

Compose starts the worker with its `configuration` and `context-manager`
dependencies. Run it next to `harness` and
[`session-manager`](https://github.com/iii-hq/workers/tree/main/session-manager).
A CLM turn needs an absolute filesystem root (`fs_scope.root`) and
permission to call `shell::exec`, because the model edits the mirror with
shell commands. Without either, the harness keeps the default strategy for
that turn and says why under `context_strategy` in
`harness::status {"verbose": true}`.

## Quickstart

Opt a session in by adding `context_strategy: "clm"` to its metadata
(operators and the console can; agents cannot). `session::set-meta`
replaces the whole metadata object, so read the current one, merge the keys
in and write it back:

```bash
META=$(iii trigger session::get --json '{"session_id":"s_123"}' | jq -c '(.meta.metadata // {}) + {context_strategy:"clm",context_budget_tokens:32768}')
iii trigger session::set-meta --json "{\"session_id\":\"s_123\",\"metadata\":$META}"
```

The change applies from the next `harness::send` to `s_123`, which runs
through `context-clm::assemble`; a turn woken by an inject carries the
previous choice. The model sees its context in
`.clm/s_123.ctx` under the session's filesystem root and is told how to edit
it.

Or start a new session with it:

```bash
iii trigger harness::send --json '{
  "message": "Find why the parser drops quoted commas and fix it",
  "model": "claude-opus-5-5",
  "provider": "anthropic",
  "session": {"metadata": {"context_strategy": "clm"}},
  "options": {"metadata": {"fs_scope": {"root": "/home/me/project"}}}
}'
```

`session.metadata` only applies when this send creates the session. For a
session that already exists the stored metadata wins and `session.metadata`
is ignored. That includes a console draft: the console creates the session
before its first send, so turn CLM on there with the `set-meta` steps above.

See what the strategy did for the session:

```bash
iii trigger context-clm::status --json '{"session_id": "s_123"}'
```

```json
{"session_id": "s_123", "found": true, "projection_messages": 42,
 "projection_tokens": 21877, "configured_budget_tokens": 65536,
 "budget_tokens": 32768, "calibration_ratio": 1.12, "stopped": false,
 "stats": {"steps": 57, "edits_applied": 6, "edits_rejected": 1,
           "rollbacks": 0, "stops": 0, "peak_tc": 30210, "stripped": 9}}
```

`projection_tokens` is the calibrated size of the task, the rollback ledger
and the projection; it leaves out the system prompt, which `peak_tc` counts.
`budget_tokens` is the budget the session's last step ran
under (its `context_budget_tokens`, else the worker default, lowered to fit
the model's usable room), and
`configured_budget_tokens` is the worker default. `stats` holds the
session's cumulative counters: steps, edits applied and rejected, rollbacks,
stops, the peak calibrated context (`peak_tc`) and stripped thinking blocks
(`stripped`). `calibration_ratio` is provider prompt tokens per counted
token. A session the worker has not seen answers `"found": false`.

Opt out by removing `context_strategy` from the session metadata; the next
`harness::send` uses context-manager again:

```bash
META=$(iii trigger session::get --json '{"session_id":"s_123"}' | jq -c '(.meta.metadata // {}) | del(.context_strategy, .context_budget_tokens)')
iii trigger session::set-meta --json "{\"session_id\":\"s_123\",\"metadata\":$META}"
```

`context-clm` is tuned for Claude Opus 5.5 and Sonnet 5.5; weaker models
tend to delete too much of their context.

## Configuration

The worker owns the `context-clm` configuration entry (`III_CONFIG_NAME`
when Compose assigns one), edited in **Settings → Workers → context-clm**.
Every field hot-reloads on the next assemble call except `data_dir`, which
is read at boot; restart the worker after changing it.

| field | default | meaning |
|---|---|---|
| `budget_tokens` | `65536` | context budget; a session's `context_budget_tokens` overrides it, unless the override is at most reserve + rollback margin (ignored with a warning) |
| `reserve_tokens` | `2048` | kept free; the limit is budget − reserve |
| `nudge_ratios` | `[0.25, 0.5, 0.75]` | budget fractions that trigger a one-time nudge |
| `persistent_nudge` | `"adaptive"` | urgent nudge: `"adaptive"` (next output would not fit), a fraction of the limit in (0, 1], or `"off"` |
| `max_rollbacks` | `50` | rollbacks before the run gets its final turn |
| `rollback_margin_tokens` | `2048` | how far below the limit a rollback lands; must be less than budget − reserve |
| `edit_gate` | `"fit"` | `"fit"` rejects edits that leave the context over the limit and bigger; `"shrink"` rejects any growth |
| `observation_max_chars` | `10000` | head/tail cap on each tool output |
| `strip_thinking` | `true` | drop thinking blocks from the first changed message on (signatures bind the exact prefix) |
| `step_log` | `true` | append one JSON line per step to the step log |
| `data_dir` | `data/context-clm` | per-session state and step logs, relative to the project; read at boot |

The entry's schema checks each field alone; the worker also checks the
cross-field rules (`budget_tokens` above `reserve_tokens`, and
`rollback_margin_tokens` below their difference), which the schema cannot
express. A stored value that breaks them is refused on hot reload (the
previous settings stay live, with a warning in the worker log), and on
purpose it stops the worker's next boot with that error instead of running
on settings nobody chose. Fix the entry, then restart the worker.

Files per session: the mirror `<root>/.clm/<session>.ctx` (mode 0600, in a
0700 `.clm/` that ignores itself through `.gitignore`), the state
`<data_dir>/<session>.json` and the step log
`<data_dir>/log/<session>.jsonl`. All three hold transcript text and are
removed when the session is deleted. The mirror also leaves the workspace when each turn ends (and `.clm/` with it once no
other session's mirror is in it); the next step writes it again from the saved state.

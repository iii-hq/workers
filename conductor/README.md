# conductor

Multi-agent fan-out + verifier-gated merge for the iii engine. Runs a task
across N agent CLIs in parallel, each in its own git worktree, runs a
configurable list of verifier gates against each result, and picks the agent
that finished first among the eligible set.

Designed to be transport-agnostic: every function the worker registers shows
up over MCP (via `iii-mcp`) and A2A (via `iii-a2a`) without any extra wiring,
subject to the RBAC policy on `iii-worker-manager`.

## Install

```bash
iii worker add conductor
```

This adds the binary to your iii install and registers it with the engine on
next worker startup. Conductor depends on three runtime peers being present:
the iii engine itself, `iii-worker-manager` (for RBAC), and at least one
transport worker (`iii-mcp` and/or `iii-a2a`) if you want to call conductor
from outside the engine. See [RBAC](#rbac) below for the matching
`config.yaml` block.

## Quick start

The dispatch below uses two stub agents and one no-op verifier so you can
see the fan-out + merge mechanics without installing `claude`, `codex`, or
any verifier worker. Replace the agents and gates with real ones once the
shape is familiar.

```bash
# 1. Register a no-op verifier function any way you like — example
#    `iii-conductor`-adjacent worker that always passes:
#
#    iii.register_function_with(
#        RegisterFunctionMessage {
#            id: "verify::noop".into(),
#            description: Some("Always pass — for conductor demos".into()),
#            ..Default::default()
#        },
#        |_payload: serde_json::Value| async move {
#            Ok(serde_json::json!({ "ok": true }))
#        },
#    );

# 2. Fan out: two `echo`-based stub agents that "succeed" by writing a file.
iii trigger conductor::dispatch --payload '{
  "task": "demo",
  "cwd": "'"$(pwd)"'",
  "agents": [
    { "kind": "claude", "bin": "bash", "args": ["-c", "echo claude > AGENT.txt"] },
    { "kind": "codex",  "bin": "bash", "args": ["-c", "echo codex  > AGENT.txt"] }
  ],
  "gates": [{ "function_id": "verify::noop" }]
}'
# => { "ok": true, "run_id": "<uuid>", "agents": 2, "gates": 1 }

# 3. Merge.
iii trigger conductor::merge --payload '{ "run_id": "<uuid>" }'
# => { "ok": true, "winner": { "index": 0, ... }, "losers": [1] }
```

The first agent to finish with a non-empty diff and passing gate wins. Loser
worktrees are pruned. The winner's branch survives at
`conductor/<run_id>/<i>-<kind>` for review.

## Functions

| Function | Input | Output |
|---|---|---|
| `conductor::dispatch` | `{ task, agents[], gates?[], cwd, timeout_ms? }` | `{ ok, run_id, agents, gates }` |
| `conductor::status` | `{ run_id }` | `RunState \| null` |
| `conductor::list` | `{}` | `RunState[]` |
| `conductor::merge` | `{ run_id }` | `MergeResult` |

### `AgentSpec`

```jsonc
{
  "kind": "claude",            // claude | codex | gemini | aider | cursor | amp | opencode | qwen | remote
  "bin": "claude",             // optional, override the default CLI binary
  "args": ["--print", "..."],  // optional, override the default arg vector
  "function_id": "a2a.foo::write_code",  // required when kind=remote
  "prompt": "Add /healthz",    // optional, defaults to the dispatch task
  "worktree": false            // pass --worktree to the CLI when supported
}
```

For `kind: "remote"`, the conductor still creates a worktree and passes
that worktree path as `cwd` in the trigger payload. Remote handlers that
write to `cwd` produce a real diff and can win the merge. This is how
external A2A agents (registered via `iii-a2a-client`) and remote MCP tool
servers (via `iii-mcp-client`) participate in a fan-out on equal footing
with local CLI agents.

### `GateSpec`

```jsonc
{ "function_id": "verify::tests", "description": "unit tests pass" }
```

A gate is **any iii function you register** with the shape
`(input: { cwd: string }) -> { ok: boolean, reason?: string }`. The
conductor passes the agent's worktree as `cwd` and treats `ok: false` as a
stop. There is no built-in `verify::*` worker in this repo — you register
gates that fit your stack. A typical gate runs `cargo test`, `npm test`,
`tsc --noEmit`, or a custom CI script and reports the exit code through
`ok`. See `examples/` (TODO) for a sample verifier worker.

Gate results are stored as an ordered `Vec<GateRunResult>`, preserving
order and duplicates (the same `function_id` can appear twice with
different descriptions, e.g. `verify::tests` for unit then again for
integration).

### `timeout_ms`

Optional. Default **600 000 ms (10 min)**. Applies to each agent
separately, not to the whole dispatch. Local agents that don't exit by
the deadline are SIGKILLed; remote agents bubble up an
`IIIError::Handler("trigger timeout: ...")`.

## How a run flows

1. `dispatch` records a seed `RunState` under `state::set` scope
   `conductor`, key `runs::<run_id>`.
2. For each agent (local or remote), conductor creates a git worktree
   (`conductor/<run_id>/<i>-<kind>`) off the current branch under
   `~/.iii/conductor/worktrees/`.
3. Local agents are spawned via `tokio::process::Command` inside their
   worktree. Remote agents are reached via
   `iii.trigger(spec.function_id, { task, cwd: <worktree path> })`.
4. As each agent completes, gates run in series against its worktree and
   the run is written back to `state::set`. Mid-run crashes preserve the
   transitions of every agent that already finished.
5. `merge` picks the eligible agent with the **smallest `finished_at`**
   (true "first finished agent wins" semantics). An agent is eligible
   when `status == Finished`, `diff` is non-empty, and every gate passed.
   Losers' worktrees are removed; the winner's worktree and branch survive
   for review.

## Idempotency — fire-and-forget

`conductor::dispatch` is **not idempotent**. Every call creates a fresh
run with a new UUID `run_id`. Calling dispatch twice with the same payload
fans out twice. If your caller might retry, dedupe at the caller (e.g.
filter `conductor::list` for an in-flight run with the same `task` /
`cwd`) before issuing a fresh dispatch. Stable fingerprint-based run ids
are tracked for v0.2.

## Errors

All errors surface as `IIIError::Handler(String)`. The string contains the
problem; resolve at the caller. Three common cases:

| Error | Cause | Fix |
|---|---|---|
| `dispatch failed: task required` | Empty or whitespace-only `task` field. | Pass a non-empty task string. |
| `dispatch failed: state::set seed: timeout` | iii engine has not registered `state::set`, or the engine is unreachable. | Confirm `iii-worker-manager` is running and `--engine-url` matches. |
| Agent state has `error: "no binary configured for agent kind X"` | The agent CLI binary (e.g. `claude`, `codex`) is not on `PATH` and `bin` was not overridden in the `AgentSpec`. | Install the CLI, set `bin` explicitly, or switch to `kind: "remote"`. |

Run with `--debug` for verbose `tracing` output if you need to dig deeper.

## RBAC

This worker registers its functions with `metadata.public = true`. To expose
them over MCP or A2A, list them in `iii-worker-manager`'s `expose_functions`:

```yaml
workers:
  - name: iii-worker-manager
    config:
      rbac:
        auth_function_id: myproject::auth
        expose_functions:
          - match("conductor::*")
          - metadata:
              public: true
  - name: iii-mcp
  - name: iii-a2a
  - name: conductor
```

## CLI flags

```text
--engine-url <URL>   WebSocket URL of the iii engine (default ws://localhost:49134)
--debug              Verbose logging (iii_conductor=debug, iii_sdk=debug)
```

## Versioning policy (0.x)

Conductor is in 0.x. Field shapes (`AgentSpec`, `GateSpec`,
`DispatchInput`, `RunState`, `MergeResult`) may change between any minor
bump. Pin `iii-conductor = "=0.1.x"` in your worker config and read
`CHANGELOG.md` before upgrading. A 1.0 release will commit to a stable
field surface.

## Dependencies

- `git` on PATH (worktree creation, diffs).
- The agent CLIs you list in `agents[]` must be installed on PATH for local
  kinds, or registered with the engine for `kind: "remote"`.
- `state::set` / `state::get` / `state::list` / `state::delete` must be
  registered (the engine ships these by default).

## Layout

Worktrees land under `~/.iii/conductor/worktrees/`.

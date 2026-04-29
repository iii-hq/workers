# conductor

Multi-agent fan-out + verifier-gated merge for the iii engine. Runs a task
across N agent CLIs in parallel, each in its own git worktree, runs a
configurable list of verifier gates against each result, and picks a winning
diff.

Designed to be transport-agnostic: every function the worker registers shows
up over MCP (via `iii-mcp`) and A2A (via `iii-a2a`) without any extra wiring,
subject to the RBAC policy on `iii-worker-manager`.

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
  "worktree": false             // pass --worktree to the CLI when supported
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

Each gate is just an iii function the conductor invokes against the agent's
worktree. The gate returns `{ ok: boolean, reason?: string }`. Recommended
gates: `verify::tests`, `verify::lint`, `verify::types`, `verify::build`,
`verify::diff_clean`. The `eval`, `guardrails`, and `proof` workers in this
repo register suitable gate functions.

Gate results are stored as an ordered `Vec<GateRunResult>`, not a map. The
same `function_id` can appear more than once with different descriptions
(e.g. `verify::tests` for unit and again for integration), and the original
order is preserved for `conductor::status`.

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

## Example

```bash
iii trigger conductor::dispatch \
  --payload '{
    "task": "Add a /healthz endpoint to the public API",
    "cwd": "/abs/path/to/repo",
    "agents": [
      { "kind": "claude" },
      { "kind": "codex" },
      { "kind": "remote", "function_id": "a2a.codex_web::write_code" }
    ],
    "gates": [
      { "function_id": "verify::tests" },
      { "function_id": "verify::types" },
      { "function_id": "verify::build" }
    ],
    "timeout_ms": 600000
  }'

iii trigger conductor::merge --payload '{ "run_id": "<id from dispatch>" }'
```

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
--debug              Verbose logging
```

## Dependencies

- `git` on PATH (worktree creation, diffs).
- The agent CLIs you list in `agents[]` must be installed on PATH for local
  kinds, or registered with the engine for `kind: "remote"`.
- `state::set` / `state::get` / `state::list` / `state::delete` must be
  registered (the engine ships these by default).

## Layout

Worktrees land under `~/.iii/conductor/worktrees/`.

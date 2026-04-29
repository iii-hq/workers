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
| `conductor::dispatch` | `{ task, agents[], gates?[], cwd, timeoutMs? }` | `{ ok, run_id, agents, gates }` |
| `conductor::status` | `{ run_id }` | `RunState \| null` |
| `conductor::list` | `{}` | `RunState[]` |
| `conductor::merge` | `{ run_id }` | `{ ok, winner?, losers[] }` |

### `AgentSpec`

```ts
{
  kind: 'claude' | 'codex' | 'gemini' | 'aider' | 'cursor' | 'amp' | 'opencode' | 'qwen' | 'remote'
  bin?: string           // override the default CLI binary
  args?: string[]        // override the default CLI arg vector
  function_id?: string   // required when kind='remote'; the iii function to trigger
  prompt?: string        // defaults to `task`
  worktree?: boolean     // pass `--worktree` to the CLI when supported
}
```

For `kind: 'remote'`, the conductor calls `iii.trigger(function_id, { task, cwd })`.
This is how external A2A agents (registered via `iii-a2a-client`) and remote MCP tool
servers (via `iii-mcp-client`) participate in a fan-out.

### `GateSpec`

```ts
{ function_id: string; description?: string }
```

Each gate is just an iii function the conductor invokes against the agent's
worktree. The gate returns `{ ok: boolean; reason?: string }`. Recommended
gates: `verify::tests`, `verify::lint`, `verify::types`, `verify::build`,
`verify::diff_clean`. The `eval`, `guardrails`, and `proof` workers in this
repo register suitable gate functions.

## How a run flows

1. `dispatch` records a `RunState` under `state::set` scope `conductor`,
   key `runs::<run_id>`.
2. For each agent, conductor creates a git worktree (`conductor/<run_id>/<i>-<kind>`)
   off the current branch.
3. Local agents are spawned via `child_process.spawn(<bin>, <args>, { cwd: worktree })`.
   Remote agents are reached via `iii.trigger(spec.function_id, …)`.
4. After each agent completes, the configured gates run in series against
   the agent's worktree.
5. `merge` picks the first agent whose status is `finished`, whose diff is
   non-empty, and whose every gate passed. Losers' worktrees are removed;
   the winner's worktree and branch survive for review.

## Example

```ts
import { TriggerAction } from 'iii-sdk'

const { run_id } = await iii.trigger({
  function_id: 'conductor::dispatch',
  payload: {
    task: 'Add a /healthz endpoint to the public API',
    cwd: '/abs/path/to/repo',
    agents: [
      { kind: 'claude' },
      { kind: 'codex' },
      { kind: 'remote', function_id: 'a2a.codex_web::write_code' },
    ],
    gates: [
      { function_id: 'verify::tests' },
      { function_id: 'verify::types' },
      { function_id: 'verify::build' },
    ],
    timeoutMs: 600_000,
  },
})

const merged = await iii.trigger({
  function_id: 'conductor::merge',
  payload: { run_id },
})
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

## Dependencies

- `git` on PATH (worktree creation, diffs).
- The agent CLIs you list in `agents[]` must be installed on PATH for local
  kinds, or registered with the engine for `kind: 'remote'`.
- `state::set` / `state::get` / `state::list` / `state::delete` must be
  registered (the `iii-cloud` engine ships these by default).

## Configuration

| Env | Default | Purpose |
|---|---|---|
| `III_URL` | `ws://localhost:49134` | iii engine websocket URL |

Worktrees land under `~/.iii/conductor/worktrees/`.

# claude-code

Claude Code as an iii worker: the Claude Code API exposed as functions and streams on the iii bus, nothing else. The worker spawns the same `claude` binary the user runs in their terminal, with the same login, the same filesystem, and the same tools (file edits, shell, web). `claude::run` executes one headless turn and returns the result; the raw Claude Code messages mirror verbatim onto the `claude::events` stream, and a translated AgentEvent view lands on `agent::events`, so the iii console, the acp worker, and any sibling worker observe a Claude Code run exactly like a native harness turn. The worker also registers `run::start_and_wait`, the same entrypoint the console and the acp worker drive, so both run Claude Code with no changes.

## Install

```bash
iii worker add claude-code
```

Requires the `claude` CLI on the host (the Agent SDK shells out to it) and either `ANTHROPIC_API_KEY` in the worker environment or an existing `claude` login.

## Skills

Install the `claude-code` agent skill for Claude Code, Cursor, and 30+ other agents:

```bash
npx skills add iii-hq/workers --skill claude-code
```

## Quickstart

From zero to a Claude Code turn over the bus:

```bash
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
iii worker add claude-code
iii   # starts the engine + worker
```

Then talk to it like any other function: from the console chat, from `iii trigger claude::run`, or from any SDK:

```ts
import { registerWorker } from 'iii-sdk';

const iii = registerWorker('ws://127.0.0.1:49134', { workerName: 'demo' });

const res = await iii.trigger({
  function_id: 'claude::run',
  payload: {
    prompt: 'Add a /health endpoint to server.ts and run the tests',
    cwd: '/path/to/repo',
    permission_mode: 'acceptEdits',
  },
  timeout_ms: 600_000,
});
// { session_id, claude_session_id, result, stop_reason, usage, total_cost_usd }
```

Or straight from the terminal with the `iii trigger` CLI:

```bash
# one full turn (raise the timeout; the default 30s is too short for agent turns)
iii trigger claude::run --timeout-ms 600000 \
  --json '{"prompt":"add a /health endpoint and run the tests","cwd":"/path/to/repo"}'

# quick reads use key=value syntax
iii trigger claude::sessions::list
iii trigger claude::status session_id=<session_id>

# background turn + control
iii trigger claude::start --json '{"prompt":"...","cwd":"/path/to/repo"}'
iii trigger claude::stop session_id=<session_id>

# ask the running engine for a function's description
iii trigger claude::run --help
```

A turn from the CLI and the session record it leaves behind:

![iii trigger claude::run returning the result with usage and cost](https://raw.githubusercontent.com/iii-hq/workers/main/claude-code/assets/cli-run.png)

![iii trigger claude::status showing the stored session record](https://raw.githubusercontent.com/iii-hq/workers/main/claude-code/assets/cli-status.png)

![iii trigger claude::run --help printing the published request schema as a parameter table](https://raw.githubusercontent.com/iii-hq/workers/main/claude-code/assets/cli-help.png)

Call `claude::run` again with the returned `session_id` to continue the same conversation: the worker maps iii session ids to Claude Code session ids in engine state and resumes automatically.

Two ids come back from every run. `session_id` is the iii session id: the key for `claude::status`, `claude::stop`, resume, and the stream group. `claude_session_id` is Claude Code's internal session id (what the worker passes to the CLI's resume under the hood) — returned for reference, not a lookup key.

Long turns: use `claude::start` to return immediately, then watch `agent::events` (group_id = your session_id) for `message_complete`, `function_execution_start/end`, and `turn_end` frames. `claude::stop` interrupts a live run, `claude::status` reads a point-in-time view, `claude::sessions::list` enumerates past sessions.

## Functions

| Function | Purpose |
| --- | --- |
| `claude::run` | Run one turn, wait, return the final result |
| `claude::start` | Fire-and-forget turn; progress arrives on `agent::events` |
| `claude::stop` | Interrupt a live run |
| `claude::status` | Session state, live flag, usage, cost |
| `claude::sessions::list` | All sessions this worker has run |
| `run::start_and_wait` | Alias for `claude::run` under the entrypoint the console and acp worker drive |

`claude::run` accepts either a bare `prompt` string or a `messages` array (`[{ role: 'user', content: [{ type: 'text', text }] }]`), plus `model`, `cwd`, `system_prompt`, `append_system_prompt`, `permission_mode`, `allowed_tools`, `disallowed_tools`, and `max_turns` overrides.

### Raw API pass-through

The named fields above cover the common path; everything else the Agent SDK accepts goes through the `options` field untouched (camelCase, exactly as in the SDK):

```jsonc
{
  "prompt": "...",
  "options": {
    "forkSession": true,
    "includePartialMessages": true,
    "fallbackModel": "claude-sonnet-4-6",
    "addDirs": ["/another/repo"]
  }
}
```

And the full output side is available raw: every message Claude Code emits (`system/init`, `assistant`, `user`, `result`, and `stream_event` token deltas when `includePartialMessages` is set) is mirrored verbatim onto the `claude::events` stream, group_id = session_id. Consumers that want the exact Claude Code wire format read `claude::events`; consumers that want harness-shaped frames read `agent::events`. Same turn, two views.

## The agent on the bus

By default every turn's system prompt carries the iii runtime context: the same engine-grounded rules as the harness identity prompts, retargeted to the `iii` CLI the agent reaches through its shell. The agent discovers capabilities from the live engine instead of memory — `iii trigger engine::functions::list` to find function ids, `iii trigger <fn> --help` as the contract before every first call, the registry flow (`directory::registry::workers::list/info`, `worker::add`) when nothing registered fits — plus the calling rules and error-handling discipline that go with them. The matching `Bash(iii *)` allow rule is added automatically so those calls run headless. Local file edits stay on Claude Code's native tools; backend actions go through registered functions.

```bash
# the agent answers this by querying the live engine itself
iii trigger claude::run --timeout-ms 300000 \
  --json '{"prompt":"List every worker connected to this engine and what each one does.","cwd":"/tmp"}'
```

Turn it off per call with `"iii_context": false` or globally in `config.yaml`; a caller-supplied `system_prompt` always wins verbatim and gets nothing appended.

## Plan mode and permission modes

`permission_mode` maps straight onto Claude Code's native modes, per turn:

| Mode | Behavior |
| --- | --- |
| `default` | Claude Code's standard permission prompts (headless: unapproved calls fail) |
| `acceptEdits` | File edits auto-approved; the worker default |
| `plan` | Native plan mode: read-only exploration, produces a plan, refuses edits |
| `bypassPermissions` | Skip all permission checks |

Plan mode headless behaves like plan mode in the terminal: the turn ends when Claude finishes the plan, and the plan text is the `result` — nothing executes. Because the worker resumes sessions, plan-then-execute is two calls against the same `session_id`:

```bash
# 1. plan (read-only)
iii trigger claude::run --timeout-ms 600000 \
  --json '{"prompt":"Plan how to add rate limiting to the REST API. Do not implement.","cwd":"/path/to/repo","permission_mode":"plan"}'

# 2. execute the plan with full context, same conversation
iii trigger claude::run --timeout-ms 600000 \
  --json '{"session_id":"<from-step-1>","prompt":"Implement the plan.","permission_mode":"acceptEdits","cwd":"/path/to/repo"}'
```

The approval step is whatever sits between the two calls — a human reading the plan, another worker, or a trigger.

## Configuration

```yaml
engine_url: ws://127.0.0.1:49134

defaults:
  model: ""                    # empty = Claude Code default
  permission_mode: acceptEdits # default | acceptEdits | plan | bypassPermissions
  max_turns: 50
  cwd: ""                      # default working directory for runs

approval_gate: false           # route tool permissions through policy::check_permissions
events_stream: agent::events   # translated AgentEvent frames
raw_events_stream: claude::events  # verbatim Claude Code messages
claude_executable: ""          # path to the claude CLI; empty = SDK default resolution
```

With `approval_gate: true` and the harness worker installed, every Claude Code tool call is checked against `policy::check_permissions` before it executes, fail-closed when the gate is unreachable, so the same YAML permission rules and console approval flow that govern native harness turns govern Claude Code.

## Observability

Every `claude::run` is an ordinary traced invocation on the engine: the trace carries the full input payload (prompt, cwd, caller worker id) and the output (result, stop reason, token usage, cost) as span events, with per-function p50/p95/p99 in the console's trace explorer — no extra instrumentation in the worker.

![claude::run invocations in the iii console trace explorer, with input and output payloads](https://raw.githubusercontent.com/iii-hq/workers/main/claude-code/assets/console-traces.png)

## How it maps

| Claude Code | iii |
| --- | --- |
| SDK `query()` turn | `claude::run` invocation |
| every SDK message, verbatim | `claude::events` stream frame |
| assistant message | `message_complete` frame on `agent::events` |
| tool_use / tool_result | `function_execution_start` / `function_execution_end` frames |
| final result | `turn_end` + `agent_end` frames, function return value |
| session resume | engine state scope `claude_sessions`, keyed by iii session_id |
| permission prompt | `canUseTool` -> `policy::check_permissions` (optional) |
| extra capability | another iii worker on the bus (`shell`, `database`, `storage`, ...) |

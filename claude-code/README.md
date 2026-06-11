# claude-code

Claude Code as an iii worker: the Claude Code API exposed as functions and streams on the iii bus, nothing else. The worker spawns the same `claude` binary the user runs in their terminal, with the same login, the same filesystem, and the same tools (file edits, shell, web). `claude::run` executes one headless turn and returns the result; the raw Claude Code messages mirror verbatim onto the `claude::events` stream, and a translated AgentEvent view lands on `agent::events`, so the iii console, the acp worker, and any sibling worker observe a Claude Code run exactly like a native harness turn. The worker also registers `run::start_and_wait`, the canonical brain entrypoint, so anything built to drive an iii brain can drive Claude Code with no changes.

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

Call `claude::run` again with the returned `session_id` to continue the same conversation: the worker maps iii session ids to Claude Code session ids in engine state and resumes automatically.

Long turns: use `claude::start` to return immediately, then watch `agent::events` (group_id = your session_id) for `message_complete`, `function_execution_start/end`, and `turn_end` frames. `claude::stop` interrupts a live run, `claude::status` reads a point-in-time view, `claude::sessions::list` enumerates past sessions.

## Functions

| Function | Purpose |
| --- | --- |
| `claude::run` | Run one turn, wait, return the final result |
| `claude::start` | Fire-and-forget turn; progress arrives on `agent::events` |
| `claude::stop` | Interrupt a live run |
| `claude::status` | Session state, live flag, usage, cost |
| `claude::sessions::list` | All sessions this worker has run |
| `run::start_and_wait` | Canonical brain contract alias for `claude::run` |

`claude::run` accepts either a bare `prompt` string or the brain-contract `messages` array (`[{ role: 'user', content: [{ type: 'text', text }] }]`), plus `model`, `cwd`, `system_prompt`, `append_system_prompt`, `permission_mode`, `allowed_tools`, `disallowed_tools`, and `max_turns` overrides.

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

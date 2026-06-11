# claude-code

Claude Code as an iii worker. `claude::run` executes a headless Claude Code turn (file edits, shell, web, MCP tools) against any directory on the host and returns the result over the iii bus. Every turn streams AgentEvent frames onto `agent::events`, so the iii console, the acp worker, and any sibling worker observe a Claude Code run exactly like a native harness turn. The worker also registers `run::start_and_wait`, the canonical brain entrypoint, so anything built to drive an iii brain can drive Claude Code with no changes.

The integration is bidirectional. While iii drives Claude Code through `claude::*`, an in-process MCP bridge hands Claude three tools (`mcp__iii__functions_list`, `mcp__iii__functions_info`, `mcp__iii__trigger`) that expose the live engine catalog, so a running Claude Code turn can call any function registered by any worker on the bus.

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

Start the engine, then run a turn from any SDK:

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

## Configuration

```yaml
engine_url: ws://127.0.0.1:49134

defaults:
  model: ""                    # empty = Claude Code default
  permission_mode: acceptEdits # default | acceptEdits | plan | bypassPermissions
  max_turns: 50
  cwd: ""                      # default working directory for runs

expose_iii_bridge: true        # give Claude the mcp__iii__* tools
approval_gate: false           # route tool permissions through policy::check_permissions
events_stream: agent::events
claude_executable: ""          # path to the claude CLI; empty = SDK default resolution
```

With `approval_gate: true` and the harness worker installed, every Claude Code tool call is checked against `policy::check_permissions` before it executes, fail-closed when the gate is unreachable, so the same YAML permission rules and console approval flow that govern native harness turns govern Claude Code.

## How it maps

| Claude Code | iii |
| --- | --- |
| SDK `query()` turn | `claude::run` invocation |
| assistant message | `message_complete` frame on `agent::events` |
| tool_use / tool_result | `function_execution_start` / `function_execution_end` frames |
| final result | `turn_end` + `agent_end` frames, function return value |
| session resume | engine state scope `claude_sessions`, keyed by iii session_id |
| permission prompt | `canUseTool` -> `policy::check_permissions` (optional) |
| MCP tools | `mcp__iii__trigger` -> any registered iii function |

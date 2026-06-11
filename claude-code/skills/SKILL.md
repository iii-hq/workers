---
name: claude-code
description: >-
  Run headless Claude Code turns over the iii bus — file edits, shell, web,
  and MCP tools against any host directory — with raw message streaming,
  AgentEvent translation, and session resume.
---

# claude-code

The claude-code worker turns Claude Code into an iii brain. `claude::run`
executes one headless Claude Code turn (the full agent: file edits, shell
commands, web fetch, MCP tools) in a configured working directory and returns
the final result, token usage, and cost over the bus. Every turn streams
AgentEvent frames onto `agent::events` keyed by session id, so the iii console
and the acp worker render Claude Code turns like any native harness turn. The
worker also registers `run::start_and_wait`, so anything built to drive the
canonical brain contract can drive Claude Code unchanged.

The worker is a pure pass-through: named payload fields cover the common
path, the `options` field forwards any Agent SDK option verbatim (including
`mcpServers`), and the raw Claude Code messages mirror onto `claude::events`
untouched. Requires the `claude` CLI on the host with an existing login or
`ANTHROPIC_API_KEY` in the worker environment.

## When to Use

- Delegate a whole coding task ("add an endpoint and run the tests") to a
  full agent from any iii worker or trigger, instead of orchestrating
  individual `coder::*` / `shell::*` calls yourself.
- Continue a conversation across calls: pass the same `session_id` and the
  worker resumes the underlying Claude Code session with full context.
- Run long agentic jobs in the background with `claude::start` and watch
  `agent::events` for `message_complete`, `function_execution_start/end`,
  and `turn_end` frames; interrupt with `claude::stop`.

## Boundaries

- Spawns the host `claude` CLI per turn — needs Claude Code installed and
  authenticated; not available inside a bare container without it.
- Tool execution happens inside Claude Code's own sandbox and permission
  model (`permission_mode`, `allowed_tools`, `disallowed_tools`), not the
  engine's; set `approval_gate: true` to route every tool call through
  `policy::check_permissions` (fail-closed, needs the harness worker).
- One live run per session id; a second `claude::run` for a busy session
  waits on the engine queue rather than merging into the live turn.
- Emits the AgentEvent subset (`message_complete`,
  `function_execution_start/end`, `turn_end`, `agent_end`) — no
  token-by-token `message_update` deltas.

## Functions

- `claude::run` — run one Claude Code turn and wait; accepts `prompt` or
  brain-contract `messages`, plus `model`, `cwd`, `permission_mode`,
  `allowed_tools`, `max_turns` overrides; returns
  `{session_id, result, usage, total_cost_usd}`.
- `claude::start` — same payload, returns `{session_id, started}`
  immediately; progress arrives on `agent::events`.
- `claude::stop` — interrupt the live run for a session.
- `claude::status` — point-in-time session view: live flag, status, turns,
  usage, cost.
- `claude::sessions::list` — every session this worker has run.
- `run::start_and_wait` — canonical brain entrypoint backed by Claude Code.

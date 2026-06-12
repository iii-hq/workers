---
name: claude-code
description: >-
  Run headless Claude Code turns over the iii bus — file edits, shell, and
  web against any host directory — with verbatim message streaming, session
  resume, and full Agent SDK option pass-through.
---

# claude-code

The claude-code worker exposes the Claude Code API as iii functions. One
`claude::run` call executes one headless Claude Code turn — the same agent
the user runs in their terminal, with the same login, filesystem, and
permission model — in a chosen working directory, and returns the final
result, token usage, and cost. The worker is a pure pass-through: named
payload fields cover the common path, the `options` field forwards any Agent
SDK option verbatim, and every message Claude Code emits mirrors untouched
onto the `claude::events` stream. A translated AgentEvent view lands on
`agent::events`, which is what the iii console and the acp worker render.

Requires the `claude` CLI on the host with an existing login or
`ANTHROPIC_API_KEY` in the worker environment. When a turn needs a
capability beyond Claude Code itself, add another iii worker to the bus
instead of bolting anything onto this one.

## When to Use

- Delegate a whole coding task ("add an endpoint and run the tests") in one
  call, instead of orchestrating individual `coder::*` / `shell::*` calls
  yourself: `claude::run` with `prompt` and `cwd`.
- Continue a conversation across calls: pass the same `session_id` again and
  the worker resumes the underlying Claude Code session with full context.
- Run long jobs without holding the call open: `claude::start` returns
  `{session_id, started}` immediately; follow `claude::events` (group_id =
  session_id) for raw progress or `agent::events` for the rendered view;
  interrupt with `claude::stop`.
- Plan before touching anything: `permission_mode: "plan"` runs Claude
  Code's native plan mode (read-only, returns the plan as the result);
  then send "implement the plan" on the same `session_id` with
  `permission_mode: "acceptEdits"`.
- Reach past the named payload fields: anything the Agent SDK accepts goes
  through `options` unchanged — `{"options": {"forkSession": true,
  "includePartialMessages": true}}` — and `includePartialMessages` puts
  token-level `stream_event` frames on `claude::events`.

## Boundaries

- Spawns the host `claude` CLI per turn — needs Claude Code installed and
  authenticated; not available inside a bare container without it.
- Function execution happens inside Claude Code's own permission model
  (`permission_mode`, `allowed_tools`, `disallowed_tools`), not the
  engine's; set `approval_gate: true` to route every call through
  `policy::check_permissions` (fail-closed, needs the harness worker).
- One turn per session at a time: check `claude::status` (`live: true`)
  before sending another `claude::run` for the same `session_id`; parallel
  runs against one session race on the underlying Claude Code resume.
- `agent::events` carries whole-message frames (`message_complete`,
  `function_execution_start/end`, `turn_end`, `agent_end`); token deltas
  exist only on `claude::events` and only when `includePartialMessages` is
  set.

## Functions

- `claude::run` — run one Claude Code turn and wait; accepts `prompt` (or a
  `messages` array whose last user entry becomes the prompt), plus `model`,
  `cwd`, `permission_mode`, `allowed_tools`, `disallowed_tools`,
  `max_turns`, `system_prompt`, `append_system_prompt`, and raw `options`;
  returns `{session_id, claude_session_id, result, stop_reason, usage,
  total_cost_usd}`.
- `claude::start` — same payload, returns `{session_id, started}`
  immediately; progress arrives on the streams.
- `claude::stop` — interrupt the live run for a session.
- `claude::status` — point-in-time session view: live flag, status, turns,
  usage, cost.
- `claude::sessions::list` — every session this worker has run.
- `run::start_and_wait` — alias for `claude::run` under the entrypoint the
  console and acp worker drive, so both run Claude Code with no changes.

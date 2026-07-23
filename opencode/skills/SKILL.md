---
type: index
name: opencode
description: >-
  Run headless OpenCode turns over the iii bus — file edits, shell, and search
  against any host directory — with verbatim JSON event streaming, session
  resume, and inline token usage and cost.
---

# opencode

The opencode worker exposes the OpenCode API as iii functions. One
`opencode::run` call executes one headless OpenCode turn — the same agent the
user runs in their terminal, with the same tools — in a chosen working
directory, and returns the final result, token usage, and cost. Every JSON
event OpenCode emits (`step_start`, `text`, `tool_use`, `step_finish`) mirrors
verbatim onto `opencode::events`; a translated AgentEvent view lands on
`agent::events`, which the iii console and the acp worker render.

Requires the `opencode` CLI on the host and an API key for the LLM provider you
use (e.g. `ANTHROPIC_API_KEY`, or `opencode auth`). When a turn needs a
capability beyond OpenCode itself, add another iii worker to the bus instead of
bolting anything onto this one.

## When to Use

- Delegate a whole coding task ("add an endpoint and run the tests") in one
  call: `opencode::run` with `prompt` and `cwd`.
- Continue a conversation across calls: pass the same `session_id` again and
  the worker resumes the underlying OpenCode session (`--session`).
- Run long jobs without holding the call open: `opencode::start` returns
  `{session_id, started}` immediately; follow `agent::events` (group_id =
  session_id) for the rendered view or `opencode::events` for raw JSON;
  interrupt with `opencode::stop`.
- Act on the whole backend: turns carry the iii runtime context by default, so
  the agent discovers and calls any registered function through the iii CLI
  (engine::functions::list, `iii trigger <fn> --help`); disable per turn with
  `iii_context: false`.
- Pick a model or agent per turn: `model` as `provider/model`, `agent` as an
  OpenCode agent name.

## Boundaries

- Spawns the host `opencode` CLI per turn — needs OpenCode installed and a
  provider key; not available in a bare container without it.
- `opencode::run` / `opencode::start` are not agent-callable without human
  approval (they spawn a full agent with the host's shell + filesystem);
  read-only introspection is allowed (see iii-permissions.yaml).
- One turn per session at a time: check `opencode::status` (`live: true`) before
  another `opencode::run` for the same `session_id`.

## Functions

- `opencode::run` — run one turn, wait; accepts `prompt` (or a `messages`
  array), plus `model`, `cwd`, `agent`, `iii_context`; returns `{session_id,
  opencode_session_id, result, stop_reason, usage, total_cost_usd}`.
- `opencode::start` — same payload, returns `{session_id, started}` immediately.
- `opencode::stop` — interrupt the live run for a session.
- `opencode::status` — point-in-time session view: live flag, status, turns,
  usage, cost.
- `opencode::sessions::list` — every session this worker has run.
- `run::start_and_wait` — alias for `opencode::run` under the entrypoint the
  console and acp worker drive.

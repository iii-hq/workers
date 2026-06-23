---
name: pi
description: >-
  Run headless Pi coding-agent turns over the iii bus — file edits, shell, and
  search against any host directory — with verbatim event streaming, session
  resume, and live steering.
---

# pi

The pi worker exposes the Pi coding-agent API as iii functions. One `pi::run`
call executes one headless Pi turn — the same in-process agent loop Pi runs in
the terminal, with the same tools (read, bash, edit, write) — in a chosen
working directory, and returns the final result, token usage, and cost. Every
event Pi emits mirrors untouched onto the `pi::events` stream; a translated
AgentEvent view lands on `agent::events`, which is what the iii console and the
acp worker render.

Pi runs the loop in-process (no CLI subprocess), so it needs model credentials
in the worker environment (e.g. `ANTHROPIC_API_KEY`) or an existing Pi login.
When a turn needs a capability beyond Pi itself, add another iii worker to the
bus instead of bolting anything onto this one.

## When to Use

- Delegate a whole coding task ("add an endpoint and run the tests") in one
  call, instead of orchestrating individual `coder::*` / `shell::*` calls
  yourself: `pi::run` with `prompt` and `cwd`.
- Continue a conversation across calls: pass the same `session_id` again and
  the worker resumes the underlying Pi session file with full context.
- Run long jobs without holding the call open: `pi::start` returns
  `{session_id, started}` immediately; follow `pi::events` (group_id =
  session_id) for raw progress or `agent::events` for the rendered view;
  interrupt with `pi::stop`.
- Steer a run while it works: `pi::steer` injects an instruction applied after
  the current tool calls finish; `pi::follow_up` queues a message processed
  after the agent would otherwise stop.
- Act on the whole backend: turns carry the iii runtime context by default, so
  the agent discovers and calls any registered function through the iii CLI
  (engine::functions::list, `iii trigger <fn> --help`); disable per turn with
  `iii_context: false`.
- Tune depth and scope per turn: `thinking_level` (off → xhigh) sets reasoning
  effort, and `tools` narrows the active tool set to an allowlist.

## Boundaries

- Runs the Pi loop in-process — needs model credentials in the worker
  environment; without them a turn fails at the first model request.
- Tool execution happens inside Pi's own tool set (`tools` allowlist), not the
  engine's permission model; `pi::run` / `pi::start` are not agent-callable
  without human approval (see iii-permissions.yaml).
- One turn per session at a time: check `pi::status` (`live: true`) before
  sending another `pi::run` for the same `session_id`; parallel runs against
  one session race on the underlying session file.
- `agent::events` carries whole-message frames (`message_complete`,
  `function_execution_start/end`, `turn_end`, `agent_end`); the raw Pi event
  shapes (including token deltas via `message_update`) live on `pi::events`.

## Functions

- `pi::run` — run one Pi turn and wait; accepts `prompt` (or a `messages`
  array whose last user entry becomes the prompt), plus `model`, `cwd`,
  `thinking_level`, `tools`, and `iii_context`; returns `{session_id,
  pi_session_id, result, stop_reason, usage, total_cost_usd}`.
- `pi::start` — same payload, returns `{session_id, started}` immediately;
  progress arrives on the streams.
- `pi::steer` — inject a steering instruction into a live run.
- `pi::follow_up` — queue a follow-up message for a live run.
- `pi::stop` — interrupt the live run for a session.
- `pi::status` — point-in-time session view: live flag, status, turns, usage,
  cost.
- `pi::sessions::list` — every session this worker has run.
- `run::start_and_wait` — alias for `pi::run` under the entrypoint the console
  and acp worker drive, so both run Pi with no changes.

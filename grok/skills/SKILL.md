---
name: grok
description: >-
  Run headless xAI Grok CLI turns over the iii bus — shell, file edits, and
  tools against any host directory — with verbatim event streaming and session
  resume.
---

# grok

The grok worker exposes the xAI Grok CLI as iii functions. One `grok::run`
call executes one headless Grok turn (`grok --single <prompt> --output-format
streaming-json`) — the same agent the user runs in their terminal, with the
same `XAI_API_KEY`, filesystem, and working directory — in a chosen directory,
and returns the final result. Every Grok event mirrors
untouched onto the `grok::events` stream; a translated AgentEvent view lands on
`agent::events`, which is what the iii console renders.

Requires the `grok` CLI on the host with `XAI_API_KEY` in the worker
environment. When a turn needs a capability beyond Grok itself, add another iii
worker to the bus instead of bolting anything onto this one.

## When to Use

- Delegate a whole coding task ("add an endpoint and run the tests") in one
  call, instead of orchestrating individual `coder::*` / `shell::*` calls
  yourself: `grok::run` with `prompt` and `cwd`.
- Continue a conversation across calls: pass the same `session_id` again and
  the worker resumes the underlying Grok session with full context.
- Run long jobs without holding the call open: `grok::start` returns
  `{session_id, started}` immediately; follow `grok::events` (group_id =
  session_id) for raw progress or `agent::events` for the rendered view;
  interrupt with `grok::stop`.
- Act on the whole backend: turns carry the iii runtime context by default
  (prepended to the first prompt of a session), so the agent discovers and
  calls any registered function through the iii CLI (engine::functions::list,
  `iii trigger <fn> --help`); disable per turn with `iii_context: false`.

## Boundaries

- Spawns the host `grok` CLI per turn — needs Grok installed and
  authenticated (`XAI_API_KEY`); not available inside a bare container without
  it.
- Tool/command execution is the Grok CLI's own; headless turns run with
  `always_approve: true` so they don't block on an interactive approval
  prompt. Set `always_approve: false` to let the CLI's approval policy gate
  execution.
- One turn per session at a time: check `grok::status` (`live: true`)
  before sending another `grok::run` for the same `session_id`; parallel
  runs against one session race on the underlying session resume.
- `agent::events` carries whole-message frames; per-item progress detail
  exists only on `grok::events`.

## Functions

- `grok::run` — run one Grok turn and wait; accepts `prompt` (or a
  `messages` array whose last user entry becomes the prompt), plus `model`,
  `cwd`, `always_approve`, and `iii_context`; returns
  `{session_id, grok_thread_id, result, stop_reason, num_turns}`.
- `grok::start` — same payload, returns `{session_id, started}`
  immediately; progress arrives on the streams.
- `grok::stop` — interrupt the live run for a session.
- `grok::status` — point-in-time session view: live flag, status, turn
  count.
- `grok::sessions::list` — every session this worker has run.

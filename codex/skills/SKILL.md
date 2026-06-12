---
name: codex
description: >-
  Run headless OpenAI Codex turns over the iii bus — sandboxed shell, file
  edits, and web search against any host directory — with verbatim event
  streaming, thread resume, and full SDK option pass-through.
---

# codex

The codex worker exposes the OpenAI Codex API as iii functions. One
`codex::run` call executes one headless Codex turn — the same agent the user
runs in their terminal, with the same login, filesystem, and sandbox — in a
chosen working directory, and returns the final result and token usage. The
worker is a pure pass-through: named payload fields cover the common path,
the `options` field forwards any Codex SDK ThreadOption verbatim, and every
thread event mirrors untouched onto the `codex::events` stream. A translated
AgentEvent view lands on `agent::events`, which is what the iii console
renders.

Requires the `codex` CLI on the host with an existing `codex login` or
`OPENAI_API_KEY` in the worker environment. When a turn needs a capability
beyond Codex itself, add another iii worker to the bus instead of bolting
anything onto this one.

## When to Use

- Delegate a whole coding task ("add an endpoint and run the tests") in one
  call, instead of orchestrating individual `coder::*` / `shell::*` calls
  yourself: `codex::run` with `prompt` and `cwd`.
- Continue a conversation across calls: pass the same `session_id` again and
  the worker resumes the underlying Codex thread with full context.
- Run long jobs without holding the call open: `codex::start` returns
  `{session_id, started}` immediately; follow `codex::events` (group_id =
  session_id) for raw progress or `agent::events` for the rendered view;
  interrupt with `codex::stop`.
- Act on the whole backend: new threads carry the iii runtime context by
  default, so the agent discovers and calls any registered function through
  the iii CLI (engine::functions::list, `iii trigger <fn> --help`); disable
  per turn with `iii_context: false`.
- Plan before touching anything: run the planning prompt with
  `sandbox_mode: read-only` (writes physically fail), read the plan, then
  send "implement the plan" on the same `session_id` with
  `sandbox_mode: workspace-write`.
- Get structured final output: pass `output_schema` (JSON schema) and the
  final agent message is JSON matching it.
- Attach screenshots or diagrams: `images: ["/path/a.png"]` adds local
  images to the prompt.
- Wire MCP servers or model providers into one turn: `codex_config`
  forwards any `config.toml` override, e.g.
  `{"codex_config": {"mcp_servers": {"github": {"command": "gh-mcp"}}}}`.
- Reach past the named payload fields: anything the SDK ThreadOptions
  accept goes through `options` unchanged, e.g.
  `{"options": {"networkAccessEnabled": true, "webSearchMode": "live"}}`.

## Boundaries

- Spawns the host `codex` CLI per turn — needs Codex installed and
  authenticated; not available inside a bare container without it.
- Execution safety is Codex's own sandbox (`sandbox_mode`), not the
  engine's: `read-only` blocks writes, `workspace-write` allows edits in
  `cwd`, `danger-full-access` disables the sandbox. Headless turns run
  `approval_policy: never`, so blocked commands fail instead of prompting.
- One turn per session at a time: check `codex::status` (`live: true`)
  before sending another `codex::run` for the same `session_id`; parallel
  runs against one session race on the underlying thread resume.
- `agent::events` carries whole-message frames; per-item progress detail
  (command output as it accumulates, todo lists) exists only on
  `codex::events`.

## Functions

- `codex::run` — run one Codex turn and wait; accepts `prompt` (or a
  `messages` array whose last user entry becomes the prompt), plus `model`,
  `cwd`, `sandbox_mode`, `approval_policy`, `reasoning_effort`,
  `skip_git_repo_check`, `output_schema`, and raw `options`; returns
  `{session_id, codex_thread_id, result, stop_reason, usage}`.
- `codex::start` — same payload, returns `{session_id, started}`
  immediately; progress arrives on the streams.
- `codex::stop` — interrupt the live run for a session.
- `codex::status` — point-in-time session view: live flag, status, turns,
  usage.
- `codex::sessions::list` — every session this worker has run.

---
name: devin
description: >-
  Drive Devin over the iii bus. Run the local Devin CLI agent (SWE-1.6) with the
  iii runtime context so it discovers and operates your mesh, or start and steer
  autonomous Devin cloud sessions through the REST API, with a passthrough to any
  Devin v1/v3 endpoint.
---

# devin

The devin worker exposes [Devin](https://docs.devin.ai) as iii functions across
two distinct surfaces.

The **CLI surface** drives the local [Devin CLI](https://cli.devin.ai/docs) (the
SWE-1.6 coding agent that runs in your terminal on your files). `devin::run`
executes one headless turn (`devin --permission-mode dangerous --print -- <prompt>`)
and streams stdout onto `devin::events` with a terminal AgentEvent frame on
`agent::events`. Because the agent runs locally with the iii runtime context
prepended, a plain-language prompt makes Devin discover and operate your engine
on its own: it runs `iii trigger engine::functions::list`, calls any registered
function, and reports back, reaching the engine at `localhost` with no exposure.

The **cloud surface** calls the Devin REST API: `devin::session::create` starts
an autonomous cloud session, `devin::session::get` reads it, `devin::session::message`
steers it, `devin::pr-review::*` runs code reviews, and `devin::api` reaches any
endpoint the typed wrappers do not cover.

The CLI surface needs the official Devin CLI installed (`brew install --cask devin-cli`)
and authenticated (`devin auth login`). The cloud surface needs `DEVIN_API_KEY`
(and `DEVIN_ORG_ID` for a service key). When a run needs a capability beyond
Devin itself, add another iii worker to the bus instead of bolting it on.

## When to Use

- Delegate a coding task to the local agent in a chosen directory: `devin::run`
  with `prompt` and `cwd`; follow `devin::events` / `agent::events`; interrupt
  with `devin::stop`.
- Have Devin operate your iii mesh: with `iii_context` on (default), ask a plain
  question ("what workers are connected and what does each do?") and Devin
  discovers and calls engine functions itself through the iii CLI.
- Delegate a whole task to Devin's cloud agent ("open a PR that adds X"):
  `devin::session::create`, then poll `devin::session::get` and steer with
  `devin::session::message`.
- Review a pull request: `devin::pr-review::trigger` with a `pr_url`, then
  `devin::pr-review::status`; bind a GitHub PR-opened trigger for auto-review.
- Reach any other Devin capability with no typed wrapper (knowledge, playbooks,
  secrets, repos, code scan, org admin): `devin::api` with
  `{ method, path, query?, body? }`.
- Schedule or fan out: bind a `cron` trigger to `devin::session::create`, or
  spawn multiple runs with `harness::spawn`. The worker does not re-implement
  scheduling or sub-agents.

## Boundaries

- `devin::run` / `devin::start` spawn the local Devin CLI headless with
  `--permission-mode dangerous` by default, so the agent auto-approves all local
  tools (needed to run `iii trigger` without blocking). Drop to `accept-edits`
  or `auto` via `cli_extra_args` to restrict it.
- `devin::run` needs the official Devin CLI installed and `devin auth login`; it
  is not available inside a bare container without it.
- The cloud surface spends ACUs and mutates real Devin state; cloud functions
  return JSON directly and do not stream, so poll `devin::session::get` (or
  schedule the poll with `cron`) rather than spinning a tight loop.
- Mutating functions (`run`, `start`, `session::create` / `message`,
  `pr-review::trigger`, `api`) stay at the `needs_approval` default; read-only
  reads and `stop` are allow-listed.
- An empty `api_key` disables the cloud surface; the CLI surface still works if
  the local binary is authenticated.

## Functions

- `devin::run` — run one local CLI turn and wait; accepts `prompt` (or a
  `messages` array), `cwd`, and `iii_context`; returns
  `{session_id, devin_session_id, url, result, stop_reason, is_error}`.
- `devin::start` — same payload, returns `{session_id, started}` immediately;
  progress arrives on the streams.
- `devin::stop` — interrupt the live CLI run for a session.
- `devin::status` — point-in-time view of a recorded run: live flag, status,
  linked Devin session id.
- `devin::sessions::list` — every run this worker has recorded (each linked to
  its Devin session). For all cloud sessions org-wide, use `devin::api`.
- `devin::session::create` — start a Devin cloud session; accepts `prompt` (or a
  `messages` array) plus `title`, `tags`, `playbook_id`, `knowledge_ids`,
  `secret_ids`, `max_acu_limit`, and the v3 (`devin_mode`, `repos`, ...) or v1
  (`snapshot_id`, `unlisted`, `idempotent`) fields your token accepts.
- `devin::session::get` — fetch one session by id (status, output).
- `devin::session::message` — send a follow-up `message` to a running session.
- `devin::pr-review::trigger` — start a Devin review for a `pr_url`.
- `devin::pr-review::status` — latest review for a `pr_url` (optional `commit_sha`).
- `devin::api` — raw authenticated call to any v1/v3 endpoint:
  `{ method, path, query?, body? }`.

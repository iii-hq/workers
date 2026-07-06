---
name: devin
description: >-
  Drive Devin over the iii bus. Start and steer autonomous Devin cloud sessions
  through the REST API, run the local devin CLI as one streamed turn, or reach
  any Devin v3 endpoint with a passthrough.
---

# devin

The devin worker exposes [Devin](https://docs.devin.ai) as iii functions across
two surfaces. The cloud surface calls the Devin REST API: `devin::session::create`
starts an autonomous session, `devin::session::get` / `::list` read it back,
`devin::session::message` steers it, and `devin::api` reaches any v3 endpoint the
typed wrappers do not cover. The CLI surface drives the local `devin` binary:
`devin::run` executes one headless turn and streams its stdout onto
`devin::events`, with a terminal AgentEvent frame on `agent::events` for the
console.

The API surface needs `DEVIN_API_KEY` in the worker environment. The CLI surface
needs the `devin` CLI installed and authenticated on the host. When a run needs a
capability beyond Devin itself, add another iii worker to the bus instead of
bolting it onto this one.

## When to Use

- Delegate a whole task to Devin's cloud agent ("open a PR that adds X"):
  `devin::session::create` with a `prompt`, then poll `devin::session::get` and
  steer with `devin::session::message`.
- Run the local CLI as one streamed turn in a chosen directory: `devin::run`
  with `prompt` and `cwd`; follow `devin::events` (group_id = session_id) for
  raw output or `agent::events` for the rendered view; interrupt with
  `devin::stop`.
- Review a pull request with Devin: `devin::pr-review::trigger` with a `pr_url`,
  then `devin::pr-review::status` to read the latest verdict.
- Work enterprise code scan findings (enterprise-gated): list with
  `devin::code-scan::findings`, measure with `devin::code-scan::metrics`, and
  open a fix PR with `devin::code-scan::remediate`.
- Reach any other Devin capability with no typed wrapper (knowledge, playbooks,
  secrets, repos, org admin): `devin::api` with `{ method, path, query?, body? }`.
- Schedule or fan out: bind a `cron` trigger to `devin::session::create` instead
  of a polling loop, or spawn multiple runs with `harness::spawn`. The worker
  does not re-implement scheduling or sub-agents.

## Boundaries

- The cloud surface spends ACUs and mutates real Devin state. `devin::session::create`,
  `devin::session::message`, `devin::api`, `devin::run`, and `devin::start` stay
  at the `needs_approval` default; only read-only reads and `devin::stop` are
  allow-listed.
- Cloud functions return JSON directly and do not stream. Poll
  `devin::session::get` for progress, or schedule the poll with `cron`; do not
  spin a tight loop.
- `devin::run` spawns the host `devin` CLI, so it needs the CLI installed and
  authenticated; it is not available inside a bare container without it. If your
  CLI needs a headless flag, add it via `cli_extra_args` in config.
- An empty `api_key` disables the whole cloud surface; the CLI surface still
  works if the local binary is authenticated.

## Functions

- `devin::session::create` — start a Devin cloud session; accepts `prompt` (or a
  `messages` array) plus `title`, `tags`, `playbook_id`, `snapshot_id`,
  `idempotent`, `max_acu_limit`, `secret_ids`, `knowledge_ids`, `unlisted`.
- `devin::session::get` — fetch one session by id (status, messages, output).
- `devin::session::message` — send a follow-up `message` to a running session.
- `devin::pr-review::trigger` — start a Devin review for a `pr_url`.
- `devin::pr-review::status` — latest review for a `pr_url` (optional `commit_sha`).
- `devin::code-scan::findings` — list enterprise findings; filter by severity,
  status, scan_id, repo_name, org_ids; paginate with after/first.
- `devin::code-scan::metrics` — enterprise code scan metrics.
- `devin::code-scan::remediate` — launch a fix session for a `{scan_id, finding_id}`.
- `devin::api` — raw authenticated call to any v3 endpoint:
  `{ method, path, query?, body? }`.
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

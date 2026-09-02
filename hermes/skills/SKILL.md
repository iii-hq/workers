---
type: index
name: hermes
description: >-
  Run headless Hermes agent turns over the iii bus, deliver messages to 27+
  chat platforms, and react to inbound platform/webhook events — with the agent
  carrying the live iii runtime context.
---

# hermes

The hermes worker puts the Hermes agent on the iii bus. `hermes::run` runs one
headless Hermes turn carrying the iii runtime context, so the agent discovers
and drives the whole engine live. `hermes::send` delivers to any of Hermes's
27+ messaging platforms. Inbound platform messages and webhook events arrive on
the worker's HTTP sink (`hermes::inbound`) and republish so any iii worker can
react. Hermes becomes iii's omnichannel front door: chat-platform reach wired to
the entire function registry.

Requires pre-provisioned Hermes credentials (`~/.hermes/.env`). When a turn
needs a capability beyond Hermes, add another iii worker instead of bolting it
on here.

## When to Use

- Run a Hermes turn that can act on the whole backend: `hermes::run` with
  `prompt` and `cwd` — the agent discovers and calls any registered function
  through the iii CLI (engine::functions::list, `iii trigger <fn> --help`).
- Reach a user on any platform: `hermes::send` with `{platform, message}` for
  Telegram, Discord, Slack, WhatsApp, Teams, and 20+ more.
- React to inbound platform/webhook events: point a Hermes gateway route at the
  worker's `inbound_api_path`; deliveries land on `hermes::inbound` and
  republish for iii workers to handle.
- Continue context across turns: pass the same `session_id` again.

## Boundaries

- Runs the Hermes CLI in a container — needs credentials provisioned; the
  interactive OAuth-via-portal flow is not used inside the worker.
- `hermes::run` / `hermes::send` are not agent-callable without human approval
  (privilege + outbound messaging); read-only introspection is allowed (see
  iii-permissions.yaml).
- Hermes one-shot returns only final text, so `agent::events` carries whole-turn
  frames (`turn_end`, `agent_end`), not per-tool frames.
- Hermes's own shell/code/web tools and MCP are intentionally not re-exposed —
  iii already has `shell`, `coder`, `web`.

## Functions

- `hermes::run` — one turn, wait, return `{session_id, result, is_error}`;
  accepts `prompt` or `messages`, plus `model`, `cwd`, `iii_context`.
- `hermes::send` — deliver `{platform, message}` to a gateway platform.
- `hermes::sessions::list` — sessions this worker has run.
- `hermes::status` — session state + live flag.
- `hermes::stop` — interrupt a live run.
- `run::start_and_wait` — alias for `hermes::run`.
- `hermes::inbound` (HTTP) — inbound gateway delivery sink; republishes events.

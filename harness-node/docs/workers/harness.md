# harness

Meta-worker that composes the modular Node workers backing the iii chat
surface.

## Purpose

The harness worker is the glue layer of the bundle. It exposes the global
status and policy surfaces every other worker relies on, terminates the
operator-facing `ui::*` plane, and pumps `agent::events` out to subscribed
browsers. On boot it reads [config.yaml](harness-node/config.yaml) for the
engine URL and the permissions file path, loads
[iii-permissions.yaml](iii-permissions.yaml), and starts watching it with
`chokidar` so policy changes apply without a restart.

It does NOT participate in the durable run loop and registers no triggers
that drive transitions; its fan-out trigger is a passive stream subscriber.

## Registered functions

- `harness::status` — Returns the harness bundle name, version, and the list of expected runtime workers.
- `ui::subscribe` — Register a browser's interest in a session (or all sessions if session_id is null).
- `ui::unsubscribe` — Remove a browser's subscription to a session (or its all-sessions sub if session_id is null).
- `harness::fs::read_inline` — Read a host file via shell::fs::read, drain its channel, and return a `{content:[{text}], details:{size, truncated, bytes_read}}` envelope (max 256 KiB inline by default).
- `policy::check_permissions` — Evaluate a function call against the current `iii-permissions.yaml`. Returns `{ decision: "allow" | "deny" | "needs_approval", rule_id?, matched_constraint? }`.
- `harness::fanout::agent_event_handler` — Internal: `agent::events` fanout handler.
- `harness::session::is_create_event` — Internal condition function bound to the sessions state trigger; matches `state:created` writes to `session/<id>/turn_state`.
- `harness::fanout::session_created` — Internal handler invoked by the sessions state trigger; fans the new session id out to every all-sessions subscriber via `ui::sessions::changed::<browser_id>`.

## Triggers

- **Stream subscriber** on `agent::events` → `harness::fanout::agent_event_handler`. Registered by [src/harness/fanout/agent-events.ts](harness-node/src/harness/fanout/agent-events.ts).
- **State trigger** on `scope: agent` gated by `condition_function_id: harness::session::is_create_event` → `harness::fanout::session_created`. Lives in [src/harness/fanout/sessions-poll.ts](harness-node/src/harness/fanout/sessions-poll.ts). This replaced the previous 1 Hz `state::list` diff loop: new sessions now reach all-sessions subscribers reactively, on the same `turn_state` write that creates them.

The fanout handler forwards every `agent::events` frame to the per-browser
endpoint `ui::session::event::<browser_id>` for each browser whose
`ui::subscribe` set matches the event's `session_id` (or who is subscribed
to all sessions). Browsers that respond with `function_not_found` are
evicted from the in-process subscription set.

## State keys

The harness reads state but doesn't own any keys. The sessions state
trigger observes `session/<id>/turn_state` writes — those entries are
owned by the orchestrator (see
[workers/turn-orchestrator.md](harness-node/docs/workers/turn-orchestrator.md)).

## Configuration

From the top-level [config.yaml](harness-node/config.yaml):

- `engine_url` (default `ws://127.0.0.1:49134`) — used to construct
  `ChannelReader` URIs in `harness::fs::read_inline`.
- `permissions_path` (default `./iii-permissions.yaml`) — file watched by
  `chokidar`. Changes hot-reload the in-memory policy used by
  `policy::check_permissions`.

## Dependencies

From [src/harness/iii.worker.yaml](harness-node/src/harness/iii.worker.yaml):

- iii engine surfaces: `iii-state ^0.11.0`, `iii-queue ^0.11.0`,
  `iii-stream ^0.11.0`, `iii-bridge ^0.11.0`, `iii-http ^0.11.0`,
  `iii-sandbox ^0.11.0`, `iii-directory ^0.5.1`.
- harness-node siblings: `turn-orchestrator`, `models-catalog`,
  `provider-anthropic`, `provider-openai`, `approval-gate`, `session`,
  `hook-fanout`, `auth-credentials`, `llm-budget` (all `^0.2.0`).
- Rust workers: `shell ^0.3.0` (for `harness::fs::read_inline`).

The full list of expected runtime peers is hard-coded in
[src/harness/expected-workers.ts](harness-node/src/harness/expected-workers.ts)
and surfaced by `harness::status` so an operator can detect a missing
worker.

## Source layout

| File | Purpose |
|---|---|
| [src/harness/main.ts](harness-node/src/harness/main.ts) | Binary entry point (`iii-harness`). |
| [src/harness/register.ts](harness-node/src/harness/register.ts) | Composes the worker's bus surface; called by both `main.ts` and the composite [src/index.ts](harness-node/src/index.ts). |
| [src/harness/config.ts](harness-node/src/harness/config.ts) | Loads `engine_url` + `permissions_path` from `config.yaml`. |
| [src/harness/status.ts](harness-node/src/harness/status.ts) | `harness::status` handler. |
| [src/harness/expected-workers.ts](harness-node/src/harness/expected-workers.ts) | List of workers `harness::status` reports as expected. |
| [src/harness/ui-subscribe.ts](harness-node/src/harness/ui-subscribe.ts) | In-memory `FanoutState` plus `ui::subscribe` / `ui::unsubscribe`. |
| [src/harness/fs.ts](harness-node/src/harness/fs.ts) | `harness::fs::read_inline` — wraps `shell::fs::read` and inlines the channel into the legacy `{content, details}` envelope. |
| [src/harness/policy.ts](harness-node/src/harness/policy.ts) | YAML loader + `chokidar` watcher; produces a `PermissionsHandle`. |
| [src/harness/policy-fn.ts](harness-node/src/harness/policy-fn.ts) | `policy::check_permissions` handler. |
| [src/harness/fanout/index.ts](harness-node/src/harness/fanout/index.ts) | Spawns the two fan-out pumps. |
| [src/harness/fanout/agent-events.ts](harness-node/src/harness/fanout/agent-events.ts) | `agent::events` stream subscriber → per-browser fan-out. |
| [src/harness/fanout/sessions-poll.ts](harness-node/src/harness/fanout/sessions-poll.ts) | State-trigger handler that detects `session/<id>/turn_state` creates and fans the new session id out to every all-sessions subscriber via `ui::sessions::changed::<browser_id>`. (Filename kept for history; the implementation is no longer a poll loop.) |
| [src/harness/iii.worker.yaml](harness-node/src/harness/iii.worker.yaml) | iii worker manifest (dependencies, install/start scripts). |

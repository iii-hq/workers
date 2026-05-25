# harness

Meta-worker that composes the modular Node workers backing the iii chat
surface.

## Purpose

The harness worker is the glue layer of the bundle. It exposes the policy
surface every other worker relies on, terminates the
operator-facing `ui::*` plane, and pumps `agent::events` out to subscribed
browsers. On boot it reads [config.yaml](harness/config.yaml) for the
engine URL and the permissions file path, loads
[iii-permissions.yaml](iii-permissions.yaml), and starts watching it with
`chokidar` so policy changes apply without a restart.

It does NOT participate in the durable run loop and registers no triggers
that drive transitions; its fan-out trigger is a passive stream subscriber.

## Registered functions

- `harness::trigger` — Browser kickoff for a chat turn: take `{session_id?, message_id?, payload}` (where `payload` is a flat `run::start` payload), forward `payload` to `run::start`, and return the result wrapped in an HTTP-style `{status_code, headers, body}` envelope. The target function id is always `run::start` — clients don't choose it. Routing through this hop (instead of calling `run::start` directly) lets the harness span wrapper seed `iii.session.id` / `iii.message.id` baggage from the outer body (see [architecture.md § Telemetry & trace correlation](harness/docs/architecture.md#telemetry--trace-correlation)).
- `ui::subscribe` — Register a browser's interest in a session (or all sessions if session_id is null).
- `ui::unsubscribe` — Remove a browser's subscription to a session (or its all-sessions sub if session_id is null).
- `harness::fs::read_inline` — Read a host file via shell::fs::read, drain its channel, and return a `{content:[{text}], details:{size, truncated, bytes_read}}` envelope (max 256 KiB inline by default).
- `policy::check_permissions` — Evaluate a function call against the current `iii-permissions.yaml`. Returns `{ decision: "allow" | "deny" | "needs_approval", rule_id?, matched_constraint? }`.
- `harness::fanout::agent_event_handler` — Internal: `agent::events` fanout handler.
- `harness::fanout::session_created` — Internal handler invoked by the sessions state trigger; fans the new session id out to every all-sessions subscriber via `ui::sessions::changed::<browser_id>`. Gates in-handler on the `state:created` marker.

## Triggers

- **Stream subscriber** on `agent::events` → `harness::fanout::agent_event_handler`. Registered by [src/harness/fanout/agent-events.ts](harness/src/harness/fanout/agent-events.ts).
- **State trigger** on `scope: session_index` (no `condition_function_id`) → `harness::fanout::session_created`. Lives in [src/harness/fanout/sessions-poll.ts](harness/src/harness/fanout/sessions-poll.ts). The turn-orchestrator writes a one-time `session_index/<sid>` marker when a session's `turn_state` is first persisted, so the trigger matches in-engine by scope alone — no per-write condition predicate. (This itself replaced an earlier 1 Hz `state::list` diff loop.)

The fanout handler forwards every `agent::events` frame to the per-browser
endpoint `ui::session::event::<browser_id>` for each browser whose
`ui::subscribe` set matches the event's `session_id` (or who is subscribed
to all sessions). Browsers that respond with `function_not_found` are
evicted from the in-process subscription set.

## State keys

The harness reads state but doesn't own any keys. The sessions state
trigger observes `session/<id>/turn_state` writes — those entries are
owned by the orchestrator (see
[workers/turn-orchestrator.md](harness/docs/workers/turn-orchestrator.md)).

## Configuration

From the top-level [config.yaml](harness/config.yaml):

- `engine_url` (default `ws://127.0.0.1:49134`) — used to construct
  `ChannelReader` URIs in `harness::fs::read_inline`.
- `permissions_path` (default `./iii-permissions.yaml`) — file watched by
  `chokidar`. Changes hot-reload the in-memory policy used by
  `policy::check_permissions`.

## Dependencies

From [src/harness/iii.worker.yaml](harness/src/harness/iii.worker.yaml):

- iii engine surfaces: `iii-state ^0.11.0`, `iii-queue ^0.11.0`,
  `iii-stream ^0.11.0`, `iii-bridge ^0.11.0`, `iii-http ^0.11.0`,
  `iii-sandbox ^0.11.0`, `iii-directory ^0.5.1`.
- harness siblings: `turn-orchestrator`, `models-catalog`,
  `provider-anthropic`, `provider-openai`, `approval-gate`, `session`,
  `hook-fanout`, `auth-credentials`, `llm-budget` (all `^0.2.0`).
- Rust workers: `shell ^0.3.0` (for `harness::fs::read_inline`).

## Source layout

| File | Purpose |
|---|---|
| [src/harness/main.ts](harness/src/harness/main.ts) | Binary entry point (`iii-harness`). |
| [src/harness/register.ts](harness/src/harness/register.ts) | Composes the worker's bus surface; called by both `main.ts` and the composite [src/index.ts](harness/src/index.ts). |
| [src/harness/config.ts](harness/src/harness/config.ts) | Loads `engine_url` + `permissions_path` from `config.yaml`. |
| [src/harness/trigger.ts](harness/src/harness/trigger.ts) | `harness::trigger` handler — WS ingestion bridge for browser-originated chat turns. Forwards the flat `payload` to `run::start` (target function id hard-coded, not client-supplied); the wrapping `instrumentHandler` (see `runtime/otel.ts`) reads `session_id`/`message_id` from the outer body and seeds baggage. |
| [src/harness/ui-subscribe.ts](harness/src/harness/ui-subscribe.ts) | In-memory `FanoutState` plus `ui::subscribe` / `ui::unsubscribe`. |
| [src/harness/fs.ts](harness/src/harness/fs.ts) | `harness::fs::read_inline` — wraps `shell::fs::read` and inlines the channel into the legacy `{content, details}` envelope. |
| [src/harness/policy/check-permissions.ts](harness/src/harness/policy/check-permissions.ts) | `registerPolicy` — registers `policy::check_permissions` and maps a `Decision` to the wire reply (`allow` / `deny` / `needs_approval`). |
| [src/harness/policy/handle.ts](harness/src/harness/policy/handle.ts) | `PermissionsHandle` + `loadAndWatch` — loads `iii-permissions.yaml`, holds the current `Permissions`, and hot-reloads it via a debounced `chokidar` watcher. |
| [src/harness/policy/permissions.ts](harness/src/harness/policy/permissions.ts) | `Permissions` — parses the YAML into compiled rules and evaluates a call via `check(function_id, args)` (first match wins → `Decision`). |
| [src/harness/policy/compile.ts](harness/src/harness/policy/compile.ts) | `compileRule` / `matchFunctionId` / `matchConstraints` — compiles a `RuleSpec` into a `CompiledRule`, matches a `function_id` by exact equality or `*` glob, and evaluates `equals` / `matches` (regex) arg constraints. |
| [src/harness/policy/types.ts](harness/src/harness/policy/types.ts) | `RuleSpec`, `ConstraintSpec`, `Decision`, `MatchedConstraint` types for `iii-permissions.yaml` rules and evaluation results. |
| [src/harness/fanout/index.ts](harness/src/harness/fanout/index.ts) | Spawns the two fan-out pumps. |
| [src/harness/fanout/agent-events.ts](harness/src/harness/fanout/agent-events.ts) | `agent::events` stream subscriber → per-browser fan-out. |
| [src/harness/fanout/sessions-poll.ts](harness/src/harness/fanout/sessions-poll.ts) | State-trigger handler that detects `session/<id>/turn_state` creates and fans the new session id out to every all-sessions subscriber via `ui::sessions::changed::<browser_id>`. (Filename kept for history; the implementation is no longer a poll loop.) |
| [src/harness/iii.worker.yaml](harness/src/harness/iii.worker.yaml) | iii worker manifest (dependencies, install/start scripts). |

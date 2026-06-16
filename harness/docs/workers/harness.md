# harness

Meta-worker that composes the modular Node workers backing the iii chat
surface.

## Purpose

The harness worker is the glue layer of the bundle. It exposes the policy
surface every other worker relies on and terminates the
operator-facing `ui::*` plane. On boot it reads [config.yaml](harness/config.yaml) for the
engine URL and the permissions file path, loads
[iii-permissions.yaml](iii-permissions.yaml), and starts watching it with
`chokidar` so policy changes apply without a restart.

It does NOT participate in the durable run loop and registers no triggers
that drive transitions; its only fan-out trigger is the passive models-catalog
state trigger.

## Registered functions

- `harness::trigger` — Browser kickoff for a chat turn: take `{session_id?, message_id?, payload}` (where `payload` is a flat `run::start` payload), forward `payload` to `run::start`, and return the result wrapped in an HTTP-style `{status_code, headers, body}` envelope. The target function id is always `run::start` — clients don't choose it. Routing through this hop (instead of calling `run::start` directly) lets the harness span wrapper seed `iii.session.id` / `iii.message.id` baggage from the outer body (see [architecture.md § Telemetry & trace correlation](harness/docs/architecture.md#telemetry--trace-correlation)).
- `ui::models::subscribe` — Register a browser's interest in model-catalog changes (`ui::models::changed::<browser_id>` pushes).
- `ui::models::unsubscribe` — Remove a browser's model-catalog change subscription.
- `harness::fs::read_inline` — Read a host file via shell::fs::read, drain its channel, and return a `{content:[{text}], details:{size, truncated, bytes_read}}` envelope (max 256 KiB inline by default).
- `policy::check_permissions` — Evaluate a function call against the current `iii-permissions.yaml`. Returns `{ decision: "allow" | "deny" | "needs_approval", rule_id?, matched_constraint? }`.
- `harness::fanout::models_changed` — Internal handler invoked by the models-catalog state trigger; debounces writes and fans out `ui::models::changed::<browser_id>` to every subscribed browser.

## Triggers

- **State trigger** on `scope: models` (no `condition_function_id`) → `harness::fanout::models_changed`. Lives in [src/harness/fanout/models-changed.ts](harness/src/harness/fanout/models-changed.ts). The handler debounces models-scope state writes and pushes `ui::models::changed::<browser_id>` to every browser that called `ui::models::subscribe`.

The harness no longer fans `agent::events` out to browsers: each browser
subscribes directly to the engine `agent::events` stream with a
`group_id`-scoped stream trigger (see `console/web` `session-events-live.ts`).
The turn-orchestrator writes the stream (`turn-orchestrator/events.ts`); the
harness meta-worker no longer re-pushes it.

## State keys

The harness reads state but doesn't own any keys. The models-catalog state
trigger observes `models` scope writes — those entries are owned by the
models-catalog worker (see
[workers/models-catalog.md](harness/docs/workers/models-catalog.md)).

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
  `provider-anthropic`, `provider-openai`, `session`,
  `hook-fanout`, `llm-budget` (all `^0.2.0`).
- iii engine built-in: `configuration` (the harness `configuration` entry
  holds provider credentials/settings — see
  [storage.md](../storage.md)).
- Rust workers: `shell ^0.3.0` (for `harness::fs::read_inline`).

## Source layout

| File | Purpose |
|---|---|
| [src/harness/main.ts](harness/src/harness/main.ts) | Binary entry point (`iii-harness`). |
| [src/harness/register.ts](harness/src/harness/register.ts) | Composes the worker's bus surface; called by both `main.ts` and the composite [src/index.ts](harness/src/index.ts). |
| [src/harness/config.ts](harness/src/harness/config.ts) | Loads `engine_url` + `permissions_path` from `config.yaml`. |
| [src/harness/trigger.ts](harness/src/harness/trigger.ts) | `harness::trigger` handler — WS ingestion bridge for browser-originated chat turns. Forwards the flat `payload` to `run::start` (target function id hard-coded, not client-supplied); the wrapping `instrumentHandler` (see `runtime/otel.ts`) reads `session_id`/`message_id` from the outer body and seeds baggage. |
| [src/harness/ui-subscribe.ts](harness/src/harness/ui-subscribe.ts) | In-memory `FanoutState` plus `ui::models::subscribe` / `ui::models::unsubscribe`. |
| [src/harness/fs.ts](harness/src/harness/fs.ts) | `harness::fs::read_inline` — wraps `shell::fs::read` and inlines the channel into the legacy `{content, details}` envelope. |
| [src/harness/policy/check-permissions.ts](harness/src/harness/policy/check-permissions.ts) | `registerPolicy` — registers `policy::check_permissions` and maps a `Decision` to the wire reply (`allow` / `deny` / `needs_approval`). |
| [src/harness/policy/handle.ts](harness/src/harness/policy/handle.ts) | `PermissionsHandle` + `loadAndWatch` — loads `iii-permissions.yaml`, holds the current `Permissions`, and hot-reloads it via a debounced `chokidar` watcher. |
| [src/harness/policy/permissions.ts](harness/src/harness/policy/permissions.ts) | `Permissions` — parses the YAML into compiled rules and evaluates a call via `check(function_id, args)` (first match wins → `Decision`). |
| [src/harness/policy/compile.ts](harness/src/harness/policy/compile.ts) | `compileRule` / `matchFunctionId` / `matchConstraints` — compiles a `RuleSpec` into a `CompiledRule`, matches a `function_id` by exact equality or `*` glob, and evaluates `equals` / `matches` (regex) arg constraints. |
| [src/harness/policy/types.ts](harness/src/harness/policy/types.ts) | `RuleSpec`, `ConstraintSpec`, `Decision`, `MatchedConstraint` types for `iii-permissions.yaml` rules and evaluation results. |
| [src/harness/fanout/index.ts](harness/src/harness/fanout/index.ts) | Spawns the models-catalog fan-out pump. |
| [src/harness/fanout/models-changed.ts](harness/src/harness/fanout/models-changed.ts) | State-trigger handler on scope `models` that debounces catalog writes and fans `ui::models::changed::<browser_id>` to subscribed browsers. |
| [src/harness/iii.worker.yaml](harness/src/harness/iii.worker.yaml) | iii worker manifest (dependencies, install/start scripts). |

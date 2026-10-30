# observe v2 in iii-hq/workers: the function contract

> **Amendment (spec commit a93643b29).** `onObserverMatch` replaces `keyed`
> and `next`: an observer is chosen for a call by a predicate
> `(obs, call) => boolean` or by keys `{ observer, call }` that iii indexes, and
> iii delivers to the matched observers. `onObserverDeliver` takes no `next`;
> it receives the matched observers, delivers itself, and returns what the
> caller gets back. Wherever this file says `keyed`, read match by keys.


Binding naming and shape contract for porting every worker in this repo to
iii observe v2. It applies the engine spec
([`iii/tech-specs/2026-10-29-observe-v2/README.md`](../../../iii/tech-specs/2026-10-29-observe-v2/README.md))
and its decisions
([`decisions.md`](../../../iii/tech-specs/2026-10-29-observe-v2/decisions.md),
in particular 16-24 for the SDK API and 26, 28-34 for workers) to the workers.
Where a worker port and this file differ, this file wins; where this file and
`decisions.md` differ, `decisions.md` wins and this file is corrected.

Every worker pins the v2 SDK: Rust `iii-sdk = "=0.26.0"` (every iii crate at
its `0.26.0`), npm `iii-sdk` / `iii-browser-sdk` `0.26.0`, Python
`iii-sdk>=0.26.0`, Go `github.com/iii-hq/iii/sdk/packages/go/iii v0.26.0`.
Shared crates in `crates/` pin the same `=0.26.0` (one iii-sdk per build
graph), including the ones that declare a range today (`config-client`,
`console-ui`, `native-ui`).

Contents:

1. [Rules](#1-rules)
2. [Every provided event source](#2-every-provided-event-source)
3. [Engine and Compose sources workers observe](#3-engine-and-compose-sources-workers-observe)
4. [Shared library APIs](#4-shared-library-apis)
5. [Harness, ADE and rbac-proxy surfaces](#5-harness-ade-and-rbac-proxy-surfaces)

## 1. Rules

### 1.1 Function ids

A trigger type is an observable function. Its id is derived mechanically:

| Old type | Function id | Rule |
|---|---|---|
| `W::X`, `W::P::X` | `W::on-X`, `W::P::on-X` | the last segment `X` becomes `on-X` (`browser::navigated` → `browser::on-navigated`, `router::models::changed` → `router::models::on-changed`, `github::pr::event` → `github::pr::on-event`) |
| `a:b` | `a::on-b` | a single-colon type gets `::` (`kanban:change` → `kanban::on-change`, `stories:build` → `stories::on-build`) |
| `W::on-X` | `W::on-X` | ids whose last segment already starts with `on-` stay (`directory::skills::on-change`) |

Fixed names, from the spec and the decisions:

| Old type | Function id | Source |
|---|---|---|
| `cron` | `cron::on-tick` | spec §11 |
| `http` | `http::on-request` | spec §11 |
| `durable:subscriber` | `queue::on-message` | spec §11 |
| `database::row-changed` | `database::on-row-changed` | spec §11 |
| `state` | `state::on-changed` | spec §11 |
| `subscribe` | `publish` (the pubsub worker's callable function; spec §11 writes `pubsub::publish`, the worker and the engine's stream bridge use the bare id) | spec §11 |
| `compose-operation` | `compose::on-operation-progress` | decision 26 |
| `console:script`, `console:style`, `console:module` | `console::ui::asset` with args `{ kind: 'script' \| 'style' \| 'module', path }` | decision 29 |
| `console:assets` | `console::ui::on-assets-changed` | decision 29 |
| `timer` (harness) | `harness::on-timer` | decision 30, `W::on-<event>` |
| `harness::triggers-changed` | `harness::on-observers-changed` | decision 30 (`harness::triggers::*` becomes `harness::observers::*`) |
| `harness::hook::pre-trigger`, `harness::hook::post-trigger` | `harness::hook::on-pre-call`, `harness::hook::on-post-call` | the hook points run around a function call; `call` is the verb (decision 10) |

The constant that holds an id is named for the function (`ON_TICK_FN`,
`ON_CHANGED_FN`), never `TRIGGER_TYPE`.

### 1.2 Provider functions

Every function in §2 follows these rules. The only exceptions are named in
its row (`publish` is public; the guest rows describe an API).

- **Private, and the handler returns the event.** Register it with
  `private: true`, a handler that returns its input, and `request_format` and
  `response_format` set to the event type. Rust:
  `RegisterFunction::new(|e: Event| Ok::<_, Error>(e)).private()`; Node:
  `(e: Event) => e` with `{ private: true }`; Python: `lambda e: e` with
  `private=True`; Go: `iii.Identity` with `iii.FunctionOptions{Private: true}`.
- **The worker emits by calling its function.** A function delivered by iii
  (`default` or `match` in §2) is called with `CallAction::Void`
  (`action: CallAction.Void()`, Go `iii.VoidAction()`): the call runs
  in-process and delivery does not wait for targets. A function whose deliver
  hook returns a result to the caller (`http::on-request`,
  `queue::on-message`, `storage::on-object-*`, `harness::hook::on-*`,
  `github::pr::on-event`) or holds the caller for back-pressure
  (`browser::on-crawl-item`) is called without an action and awaited.
- **Gate on observation.** Before building an event, the emitter checks
  `is_observed(id)` (one atomic load). A function matched by keys checks
  `is_observed_for(id, &call)` with the call's input, which looks up the
  call's keys. Counters that must advance whatever the observers (agent feed
  `seq`, kanban revisions) advance before the check.
- **`args_format` only when §1.3 says so.** A function without it takes no
  args: an observer that passes args gets `INVALID_ARGS` from the engine.
- **Hooks.** Rust and Go set hooks by id before registering
  (`iii.on_observer_added_for(ID, h)`, `client.OnObserverAddedFor(ID, h)`),
  Node and Python chain them on the registration or set them by id. A hook
  that needs worker state implements the hook trait on the worker's type
  (Rust) or closes over it.
- **Rejections.** `onObserverAdded` rejects with
  `ObserveRejection(code, message)`. Codes are the worker's own and listed in
  its README: `INVALID_CRON_EXPRESSION`, `HTTP_ROUTE_CONFLICT`, `UNKNOWN_DB`,
  `CONFIG_ERROR`, `OBSERVER_ID_CONFLICT`, and `OBSERVER_LIMIT` for a function
  that caps its observers (§2 names every cap). A function whose observers
  can become invalid later (a database removed from config, an asset
  superseded) calls `obs.reject(code, message)`.
- **Initial state.** A function whose observers receive the current state when
  they are added (`tmux::on-changed`, `harness::on-ready`, `router::on-ready`,
  `console::workspace::on-changed`, `console::ui::on-assets-changed`,
  `shell::on-job-finished`, the `console::ui::asset` content fetch) does it
  from `onObserverAdded` by spawning a task that awaits `obs.accepted()` and
  then calls through the observer:
  `iii.call(CallRequest::observer(&obs, state).action(CallAction::Void))`.
  The hook itself returns at once: a call through an observer that is not
  active fails with `OBSERVER_NOT_ACTIVE`, and the observer becomes active only
  after its hooks resolve (decision 19).
- **Custom delivery.** A deliver hook that calls targets itself applies each
  observer's filters with `obs.matches(call)` before calling it, and documents
  in the function's description and README what targets receive and what the
  caller gets back.
- **Observers stay with the engine.** A provider never reads `obs.filters` to
  decide anything, never stores an observer's target, namespace or metadata
  (the engine holds them and applies them to every call through the observer),
  and keeps per-observer state keyed by `obs.id`.
- **Removed from providers:** `TriggerHandler` impls, `register_trigger_type`
  / `registerTriggerType`, subscriber sets and binding tables, per-binding
  metadata and namespace passthrough, `trigger_request_format` /
  `call_request_format`, manifest `trigger_types`, and event fields that name
  the trigger (`CronTick.trigger`, `HttpRequest.trigger`,
  `EventEnvelope.trigger_type`, `crawl.trigger_type`).
- **Credentials.** A function whose input or output carries a secret is
  registered with `observable: false`: `secrets::set`,
  `secrets::compare-and-set` and `secrets::resolve`, and any function a
  worker documents as carrying a credential.
- **One host.** A function id has one host per namespace. A worker that runs
  more than one instance registers its sources with `standby: true`; fire
  locks that existed to stop instances from firing twice go away (cron's
  per-observer redis lock).

### 1.3 Args or filters (decision 28)

Args belong to the provider; filters belong to iii. A function takes args
only when the provider needs a per-observer value:

1. **Setup per observer:** something the provider starts, installs or stores
   for that observer (a cron job, a route, a queue consumer, a database
   trigger, a filesystem watch, a persisted subscriber, a timer, a hook's
   priority and timeout, an asset path).
2. **Per-observer state or delivery shape:** a cap enforced in
   `onObserverAdded`, a per-observer payload (kanban `ticket: summary`,
   stories narrowing, github notification policy), a per-observer delivery
   slot or timeout.
3. **High-volume per-key streams that must skip unwatched keys:** frame
   streams and agent feeds, matched by keys (`onObserverMatch`) and gated with
   `is_observed_for(call)`, so a call for a key nobody watches costs one hash
   lookup per key.

Every other event happens whatever is observing it and takes **no args**:
consumers select calls with filters on its `input` fields. Session-scoped
events of session-manager, harness and approval-gate take no args; consumers
filter on `input.session_id`.

The functions with args, all of them: `cron::on-tick`, `http::on-request`,
`queue::on-message`, `database::on-row-changed`, `harness::hook::on-*` (six),
`harness::on-timer`, `console::ui::asset`, `shell::on-changed`,
`shell::on-git-changed`, `shell::on-job-finished`, `github::pr::on-event`,
`storage::on-object-created`, `storage::on-object-deleted`,
`kanban::on-change`, `kanban::on-comment`, `stories::on-changed`,
`computer::on-frame-changed`, `browser::on-frame-event`,
`browser::on-crawl-item`, `ios-simulator::on-frame-event`, the agent feeds
`<agent>::on-agent-event` and `<agent>::on-raw-event` for `claude`, `codex`,
`cursor`, `devin`, `grok`, `hermes`, `opencode` and `pi`, and the a2ui example
`demo::on-counter-changed`.

### 1.4 Consumers

A binding is an observer:

```ts
// before: iii.registerTrigger({ type: T, function_id: target, config, metadata })
iii.observe(
  { function_id: F, args: config, deliver: 'output' },   // F takes args
  target,
  { metadata },
)
iii.observe(
  { function_id: F, filters: { 'input.session_id': { $eq: id } }, deliver: 'output' },  // F takes none
  target,
)
```

- **`config` → args or filters.** When `F` takes args, the old config is its
  args (minus `metadata` and `condition_function_id`). When it takes none,
  each config field that selected events becomes a filter on `input.<field>`
  with an explicit operator (`$eq`, `$in`, `$gte`; filters have no literal
  shorthand), and the rest is dropped.
- **`deliver`.** A target whose handler takes the event observes with
  `deliver: 'output'` (`Deliver::Output`, Go `iii.DeliverOutput`): for an
  event source the output is the event. Observers of `publish` use
  `deliver: 'input'`. The harness's own observers take the `Observed`
  envelope (decision 30). Functions with custom delivery (`http`, `queue`,
  `storage`, `github::pr`, `kanban`, `stories`, `computer::on-frame-changed`,
  `browser::on-crawl-item`, `harness::hook::*`, `console::ui::asset`) call
  targets themselves; `deliver` does not apply and their rows say what the
  target receives.
- **Metadata.** Binding metadata and `config.metadata` move to the observe
  options (`ObserverOptions::metadata`). The target receives them as the
  metadata sidecar; the provider never sees them. A value the provider needs
  is an arg (the harness hooks' `inject_prompt`).
- **Namespaces.** A binding's `trigger_namespace` / `in_trigger_namespace`
  is the source's namespace (`Source::namespace`, `ObserveSource.namespace`);
  its target namespace is the target's (`Target::function(id).namespace(ns)`,
  Go `ObserverOptions.TargetNamespace`). An unset source namespace resolves
  like a call: the worker's own namespace, and `default` for `engine::*` and
  `configuration::*`. A worker in a namespace that observes another
  engine-hosted source (`observability::*`, `stream::*`) sets
  `namespace: 'default'`; one that observes Compose sets the Compose
  namespace.
- **`condition_function_id`** has no equivalent. A condition that selected by
  event fields is written as filters. The in-repo uses are dropped:
  `http/tests/e2e_condition.rs`, the condition cases of
  `state/tests/e2e_state.rs`, and the unused condition path of cron and
  state. No production consumer in this repo sets one.
- **No retry loops.** `observe` returns a handle at once; the observer is
  `pending` until the function registers and becomes `active` after the
  provider's hooks accept it. Loops that re-bound until a provider existed
  (`approval-gate` `retry_hook_bindings`, `memory-consolidate` bind
  retries, `bind_best_effort` helpers, `trigger_instance_count` probes) are
  removed. A worker that reports binding health reads `handle.state()` or
  awaits `handle.ready()` with a bounded timeout.
- **Handles.** `Trigger` handles become `ObserverHandle`s; `unregister()`
  becomes `dispose()`. Dropping a handle does not dispose the observer. To
  change an observer (a new cron schedule, a reloaded asset), observe the new
  one, then dispose the old.
- **Browser tabs** register `on(handlerId, cb)`, observe with target
  `${handlerId}::${browserId}`, and dispose on unmount (§4.5).
- **Agents** observe through `harness::observe` (§5.1).

### 1.5 Calling

`trigger` is `call` everywhere (decision 16): `iii.trigger(TriggerRequest)` →
`iii.call(CallRequest)`, `TriggerAction` → `CallAction`,
`TriggerRequestWithMetadata` → `CallRequestWithMetadata`, Python
`trigger` / `trigger_async` → `call` / `call_async`, Go `client.Trigger` →
`client.Call(ctx, iii.CallRequest{...})`, browser `host.iii.trigger` →
`host.iii.call`, CLI `iii trigger <fn>` → `iii call <fn>` (scripts, CI,
READMEs, `InvokePanel` command strings). Workers never use the aliases.

### 1.6 Docs, manifests and tests

- Each worker README lists every function it provides with its args (or
  "none" and the input fields to filter on), its delivery and its event shape,
  and shows one `iii.observe` per consumer pattern. SKILL.md files, agent
  prompts and `iii.worker.yaml` descriptions name functions and observers.
- Docs state behavior plainly. They do not say "no longer", "now", "instead
  of triggers" or "unlike"; trigger wording stays only for database triggers
  and unrelated meanings.
- Golden schemas for trigger types become the function's
  `request_format`/`response_format`/`args_format` goldens. Tests that
  scripted `registertrigger` frames script `observe` frames and assert
  observer state.
- CI reads `engine::functions::list|info` (with `args_format`, `private`,
  `observable`, `observer_count`) and `engine::observers::list`; the
  `engine::triggers::*` and `engine::registered-triggers::*` reads and the
  trigger-types baseline go away (`.github/scripts/collect_worker_interface.py`
  and its tests).

### 1.7 The API per language

| | Rust | Node / browser | Python | Go |
|---|---|---|---|---|
| Register a source | `iii.register_function(ID, RegisterFunction::new(\|e: E\| Ok::<_, Error>(e)).args_format::<A>().private())` | `iii.registerFunction(ID, (e: E) => e, { args_format, private: true })` | `iii.register_function(ID, lambda e: e, args_format=A, private=True)` | `client.RegisterFunction(ID, iii.Identity, iii.FunctionOptions{ArgsFormat: a, Private: true})` |
| Hooks for one function | `iii.on_observer_added_for(ID, h)`, `on_observer_removed_for`, `on_observer_match_for`, `on_observer_deliver_for` | `iii.onObserverAdded(ID, h)` or `ref.onObserverAdded(h)`, `onObserverRemoved`, `onObserverMatch`, `onObserverDeliver` | `@ref.on_observer_added`, `iii.on_observer_added(ID)(h)`, `on_observer_match`, `on_observer_deliver` | `client.OnObserverAddedFor(ID, h)`, `ref.OnObserverMatch(m)`, `ref.OnObserverDeliver(h)` |
| Hooks for every function | `iii.on_observer_added(h)`, `iii.on_observer_removed(h)` | `iii.onObserverAdded(h)` | `@iii.on_observer_added` | `client.OnObserverAdded(h)` |
| Match by predicate | `iii.on_observer_match_for(ID, Match::predicate(\|o, c\| …))` | `ref.onObserverMatch((obs, call) => …)` | `ref.on_observer_match(lambda obs, call: …)` | `ref.OnObserverMatch(iii.MatchFunc(func(o *iii.Observer, c *iii.Call) bool { … }))` |
| Match by keys (indexed) | `Match::keys(\|o\| keys, \|c\| keys)` | `ref.onObserverMatch({ observer, call })` | `ref.on_observer_match(observer=..., call=...)` | `iii.MatchKeys(obsKeys, callKeys)` |
| Is anyone watching | `iii.is_observed(ID)`, `iii.is_observed_for(ID, &call)` | `iii.isObserved(ID, call?)` | `iii.is_observed(ID, call)` | `client.IsObserved(ID)`, `client.IsObservedFor(ID, call)` |
| Emit | `iii.call(CallRequest::new(ID, json!(e)).action(CallAction::Void))` | `iii.call({ function_id: ID, payload: e, action: CallAction.Void() })` | `await iii.call_async({"function_id": ID, "payload": e, "action": CallAction.Void()})` | `client.Call(ctx, iii.CallRequest{FunctionID: ID, Payload: e, Action: iii.VoidAction()})` |
| Call through an observer | `iii.call(CallRequest::observer(&obs, payload))` | `iii.call({ observer: obs, payload })` | `await iii.call_async({"observer": obs, "payload": payload})` | `client.Call(ctx, iii.CallRequest{Observer: obs, Payload: p})` |
| Reject | `Err(ObserveRejection::new(code, msg))` | `throw new ObserveRejection(code, msg)` | `raise ObserveRejection(code, msg)` | `return iii.ObserveRejection(code, msg)` |
| Observe | `iii.observe_with(Source::function(F).args(a).filters(f).deliver(Deliver::Output), target, ObserverOptions::default().metadata(m))` | `iii.observe({ function_id: F, args, filters, deliver: 'output' }, target, { metadata })` | `iii.observe({"function_id": F, "args": a, "filters": f, "deliver": "output"}, target, {"metadata": m})` | `client.Observe(iii.Source{FunctionID: F, Args: a, Filters: f, Deliver: iii.DeliverOutput}, target, iii.ObserverOptions{Metadata: m})` |

The names follow decisions 16-24 and the in-progress SDK branches
(`ov2/*`). The SDKs are not finished: a spelling that differs in the released
0.26.0 is fixed in the port, and the shapes (arguments, return values,
where hooks live) stay as written here.

## 2. Every provided event source

Columns: the hosting worker, the old trigger type, the function id, its args
(field list, or "none" with the input fields consumers filter on), its
delivery, the event its handler returns (the type name in the worker, with its
fields where the name does not say them), and the consumers in this repo
(`worker:path`). Delivery is one of:

- **default**: iii delivers every call to each observer whose filters match.
- **match(o: …, c: …)**: `onObserverMatch` by keys, with the observer key and
  the call key; `*` lists a wildcard key. iii delivers to the matched
  observers. (Rows written `match(o: …, c: …)` mean exactly this.)
- **custom**: an `onObserverDeliver` hook described in the cell. It receives
  the observers that matched (all of them, unless the row also names match
  keys) and has no `next`: it calls through observers itself and returns what
  the caller gets back.

`added` and `removed` name the `onObserverAdded` / `onObserverRemoved` hooks.

### 2.1 The spec §10 and §11 workers

| # | Worker | Old type | Function | Args | Delivery | Event | Consumers |
|---|---|---|---|---|---|---|---|
| 1 | cron | `cron` | `cron::on-tick` | `{ expression }` | match(o: `args.expression`, c: `input.expression`); added: retain the expression's job (`INVALID_CRON_EXPRESSION`); removed: release it; one job per distinct expression calls the function once per fire | `CronTick { expression, scheduled_time, actual_time }` | memory-consolidate:src/main.rs, sentinel:src/triggers.rs, eval:src/main.rs, github:src/webhooks/wiring.rs, openwiki:src/index.mjs (args `{ expression }`), security-scan:src/schedule.rs (`{ repository }` as observe metadata), workflow:src/configuration.rs, worktree:src/configuration.rs, harness:src/configuration.rs, cron:tests/e2e_schedule.rs, cron:tests/e2e_on_tick.rs |
| 2 | http | `http` | `http::on-request` | `{ method, path, middleware_function_ids?, public_webhook? }` | custom (spec §10.3): added: `routes.add` (`HTTP_ROUTE_CONFLICT`); removed: `routes.remove`; deliver: match method and path, `{ status_code: 404 }` on a miss, `422` when `!obs.matches(call)`, run middleware, then `call({ observer, payload: { ...request, params } })`; its result is the response | `HttpRequest` (target receives `HttpRequest` with `params`, returns `HttpResponse`) | hermes:src/hooks.py, memory-consolidate:src/main.rs (paths gain the leading `/`), memory:src/main.rs, slack:src/ingress.rs, telegram-bot:src/functions/mod.rs, whatsapp:cloud_webhook.go, whatsapp:cloud_flow_endpoint.go, github:src/webhooks/wiring.rs, image-resize:example/src/hooks.ts, image-resize:example/src/index.ts, mcp:src/main.rs, mcp:tests/common/engine.rs, openwiki:src/index.mjs, worktree:src/main.rs, github:tests/webhooks_integration.rs, http:tests/common/backend.rs, http:tests/webhook_listener.rs, http:tests/on_request.rs, http:tests/e2e_request_body.rs, http:tests/e2e_streaming.rs |
| 3 | queue | `durable:subscriber` | `queue::on-message` | `{ topic, max_retries?, backoff_ms?, queue_config?, group? }` | custom (spec §10.4): added: `consumers.start(obs.id, args)` (`OBSERVER_ID_CONFLICT`); removed: `consumers.stop(obs.id)`; consumer loops call the function with `metadata: { subscriber: obs.id }`; deliver: that observer, or `OBSERVER_NOT_ACTIVE` (retried, then dead-lettered); a filter miss acks with `null`; else `call({ observer, payload: input })`, whose result acks, retries or dead-letters | `QueueMessage` (the published data) | memory:src/main.rs, github:src/webhooks/wiring.rs, queue:tests/e2e_durability.rs, queue:tests/e2e_scaled_out.rs, queue:tests/e2e_observe.rs |
| 4 | database | `database::row-changed` | `database::on-row-changed` | `{ db, table?, ops? }` (table canonical) | as row 5 | `RowChangedEvent` | database:ui/src/page/useRowChanges.ts, database:tests/observe.rs, database:tests/e2e/workers/harness/src/cases-native-capture.ts, database:tests/e2e/workers/harness/src/cases-row-changed.ts, database:tests/e2e/workers/harness/src/cases-pg-routing.ts |
| 5 | database | `database::on-row-changed` (0.25 owner-delivered) | `database::on-row-changed` | `{ db, table?, ops? }` | match(o: `db\|table or *\|op or *` for each op, c: `db\|{table,*}\|{op,*}`, four keys); added: `NativeCapture` validates against live config and installs capture (`UNKNOWN_DB`, `NATIVE_CAPTURE_REQUIRES_TABLE`, `NON_CANONICAL_TABLE`, `CAPTURE_INSTALL_FAILED`); removed: release; config revalidation calls `obs.reject` | `RowChangedEvent` | as row 4 |
| 6 | state | `state` | `state::on-changed` | none; filters `input.scope`, `input.key`, `input.event_type` | default; emitted in the writing handler's context (`caller.parent` names the writer) | `StateEvent { type, event_type, scope, key, old_value, new_value }` | canvas:ui/src/lib/live.ts, a2ui:ui/src/live.ts, state:ui/src/lib/events.ts, onboarding:ui/src/conditions.ts, harness:ui/src/context-chip/index.tsx, crates/provider-integration-testkit:src/codex_login.rs, kits:deepwork/agents/deepwork.md, kits:deepwork/agents/deepwork-lead.md, state:tests/e2e_state.rs |
| 7 | state | `state::on-changed` (0.25 auto) | `state::on-changed` | none | default | `StateEvent` | as row 6 |
| 8 | pubsub | `subscribe` | `publish` (public, callable by anyone) | none; filters `input.topic` | default; observers use `deliver: 'input'` | `PubSubInput { topic, data }` | pubsub:tests/e2e_pubsub.rs, pubsub:README.md |

### 2.2 Every other worker

| # | Worker | Old type | Function | Args | Delivery | Event | Consumers |
|---|---|---|---|---|---|---|---|
| 9 | a2ui (example) | `demo::counter-changed` | `demo::on-counter-changed` | `{ counter }` | match(o: `args.counter`, c: `input.counter`); added: `OBSERVER_LIMIT` past 64 | `CounterChanged { counter, revision }` | a2ui:tests/live_owned_binding.rs |
| 10 | approval-gate | `approval::pending-created` | `approval::on-pending-created` | none; filters `input.session_id`, `input.session_metadata.<key>` | default | `PendingApprovalRecord` | slack:src/functions/bindings.rs, telegram-bot:src/functions/mod.rs, ade:web/src/lib/backend/approval-events-live.ts, approval-gate:src/testkit/engine.rs |
| 11 | approval-gate | `approval::pending-resolved` | `approval::on-pending-resolved` | none; filters as row 10 | default | `PendingResolvedEvent` | as row 10 |
| 12 | browser | `browser::session-started` | `browser::on-session-started` | none; filters `input.session_id` | default | `SessionStartedEvent` | browser:ui/src/lib/events.ts, browser:ui/src/page/useBrowserSessionsLive.ts, browser:native-ui/src/api.ts |
| 13 | browser | `browser::session-stopped` | `browser::on-session-stopped` | none; filters `input.session_id` | default | `SessionStoppedEvent` | browser:ui/src/lib/events.ts, browser:ui/src/page/useBrowserSessionsLive.ts, browser:native-ui/src/api.ts, browser:tests/integration.rs |
| 14 | browser | `browser::session-updated` | `browser::on-session-updated` | none; filters `input.session_id` | default | `SessionUpdatedEvent` | browser:ui/src/page/useLiveFrames.ts, browser:ui/src/page/useBrowserSessionsLive.ts, browser:native-ui/src/api.ts, browser:tests/integration.rs |
| 15 | browser | `browser::navigated` | `browser::on-navigated` | none; filters `input.session_id` | default | `NavigatedEvent` | browser:ui/src/page/useLiveFrames.ts, browser:ui/src/page/useBrowserSessionsLive.ts, browser:native-ui/src/api.ts |
| 16 | browser | `browser::console-event` | `browser::on-console-event` | none; filters `input.session_id` | default; emitter gates on `is_observed()` before serializing | `ConsoleEventPayload` | browser:ui/src/page/ConsolePanel.tsx, browser:native-ui/src/api.ts |
| 17 | browser | `browser::network-event` | `browser::on-network-event` | none; filters `input.session_id` | default; gated as row 16 | `NetworkEventPayload` | browser:ui/src/page/NetworkPanel.tsx, browser:native-ui/src/api.ts |
| 18 | browser | `browser::picked` | `browser::on-picked` | none; filters `input.session_id` | default | `PickedEvent` | browser:ui/src/lib/events.ts |
| 19 | browser | `browser::handoff-requested` | `browser::on-handoff-requested` | none; filters `input.session_id` | default | `HandoffRequestedEvent` | browser:ui/src/lib/events.ts, browser:native-ui/src/api.ts |
| 20 | browser | `browser::handoff-resolved` | `browser::on-handoff-resolved` | none; filters `input.session_id` | default | `HandoffResolvedEvent` | browser:ui/src/lib/events.ts, browser:native-ui/src/api.ts |
| 21 | browser | `browser::download-changed` | `browser::on-download-changed` | none; filters `input.session_id` | default | `DownloadChangedEvent` | browser:ui/src/page/DownloadsPanel.tsx |
| 22 | browser | `browser::frame-event` | `browser::on-frame-event` | `{ session_id? }` | match(o: `args.session_id` or `*`, c: `[input.session_id, *]`); the frame pump gates on `is_observed_for(call)` before encoding; viewers observe with `max_in_flight: 1` so a slow viewer drops frames (`OVERFLOW`) | `FrameEventPayload` | browser:ui/src/page/useLiveFrames.ts, browser:native-ui/src/api.ts (`WIRE.frames`), browser:tests/integration.rs, browser:tests/native_viewer.rs |
| 23 | browser | `browser::chromium-install-progress` | `browser::on-chromium-install-progress` | none | default | `InstallProgress` | browser:native-ui/src/api.ts, ade:web/src/lib/engine-trigger.ts, ade:web/src/lib/onboarding/chromium.ts |
| 24 | browser | `browser::crawl-item` | `browser::on-crawl-item` | `{ crawl_id? }` (at most 256 bytes) | custom, with match(o: `args.crawl_id` or `*`, c: `[input.crawl_id, *]`) picks observers; the hook awaits `call({ observer, payload, timeout_ms })` for each matching observer in `seq` order, so a slow consumer slows the crawl; added: validates `crawl_id`, `OBSERVER_LIMIT` past 64 | `CrawlItemEvent` (the crawl result's `trigger_type` field becomes `function_id`) | browser:tests/crawl_feed_live.rs |
| 25 | claude-code | `claude::agent-event` | `claude::on-agent-event` | `{ session_id }` (required, 1..512 chars) | match(o: `args.session_id`, c: `input.session_id`); added: `OBSERVER_LIMIT` past 256; emitter gates on `isObserved(ID, { input: { session_id } })` | `FeedPayload { session_id, event_id, seq, epoch, source, event }` | acp:src/handler.rs, acp:src/agent_feed.rs |
| 26 | claude-code | `claude::raw-event` | `claude::on-raw-event` | `{ session_id }` | as row 25, its own `seq` | `FeedPayload` (raw CLI message) | none in repo |
| 27 | code-runner | guest-defined (`iii.registerTriggerType` in guest code) | guest-defined, through the guest API of §4.4 | guest-defined `args_format` | iii's delivery, or the guest's calls through observers (§4.4) | guest-defined | guest code (code-runner:src/node_bus.rs) |
| 28 | codex | `codex::agent-event` | `codex::on-agent-event` | `{ session_id }` | as row 25 | `FeedPayload` | acp:src/handler.rs, acp:src/agent_feed.rs |
| 29 | codex | `codex::raw-event` | `codex::on-raw-event` | `{ session_id }` | as row 25 | `FeedPayload` (raw thread event) | none in repo |
| 30 | computer | `computer::session-started` | `computer::on-session-started` | none; filters `input.session_id` | default | `SessionStartedEvent` | computer:ui/src/page/index.tsx |
| 31 | computer | `computer::session-stopped` | `computer::on-session-stopped` | none; filters `input.session_id` | default | `SessionStoppedEvent` | computer:ui/src/overlay/ComputerOverlay.tsx, computer:ui/src/page/index.tsx |
| 32 | computer | `computer::activity` | `computer::on-activity` | none; filters `input.session_id` | default | `ActivityEvent` | computer:ui/src/overlay/ComputerOverlay.tsx |
| 33 | computer | `computer::frame-changed` | `computer::on-frame-changed` | `{ session_id }` (required, non-empty) | custom, with match(o: `args.session_id`, c: `input.session_id`) picks observers; the hook offers the frame to each matching observer's delivery slot (latest frame wins) and the slot's drain calls through the observer (`timeout_ms: 5000`); added: `OBSERVER_LIMIT` past 64, opens the slot; removed: closes it | `FrameChange` | computer:ui/src/page/useLiveFrames.ts, computer:tests/live_frames.rs |
| 34 | crates/node-core | guest-defined (`registerTriggerType`) | guest-defined (§4.4) | guest-defined | as row 27 | guest-defined | guest code run by code-runner |
| 35 | cursor | `cursor::agent-event` | `cursor::on-agent-event` | `{ session_id }` | as row 25 | `FeedPayload` | acp:src/handler.rs, acp:src/agent_feed.rs |
| 36 | cursor | `cursor::raw-event` | `cursor::on-raw-event` | `{ session_id }` | as row 25 | `FeedPayload` (raw frame) | none in repo |
| 37 | devin | `devin::agent-event` | `devin::on-agent-event` | `{ session_id }` | as row 25 | `FeedPayload` | acp:src/handler.rs, acp:src/agent_feed.rs |
| 38 | devin | `devin::raw-event` | `devin::on-raw-event` | `{ session_id }` | as row 25 | `FeedPayload` | none in repo |
| 39 | email | `email::new-mail` | `email::on-new-mail` | none; filters `input.account`, `input.folder` | default (IMAP IDLE runs per configured account) | `NewMailEvent { account, folder, uid, message_id, from, subject, snippet, ts }` | none in repo |
| 40 | eval | `eval::completed` | `eval::on-completed` | none; filters `input.evaluation_id` | default | `CompletedEventV1 { evaluation_id, status, timestamp }` | eval:ui/src/events.ts |
| 41 | github | `github::called` | `github::on-called` | none | default; `run_and_emit` gates on `is_observed()` | `CalledEvent` | github:ui/src/page/events.ts |
| 42 | github | `github::pr::event` | `github::pr::on-event` | `EventFilter { notifications?, watch_id?, repo?, number?, categories? }` | custom (the queue pattern): added: validate repo and policy, persist the subscriber under `obs.id`; removed: drop it (orphan and grace logic stay); the job processor calls the function once per subscriber notification with `metadata: { subscriber: obs.id }` and that subscriber's payload (digest, compact or full); deliver: that observer if `obs.matches(call)`, `call({ observer, payload: input, timeout_ms: 8000 })`; reconciliation reads `iii.observers(ID)` | `NotificationEvent` | github:tests/webhooks_integration.rs |
| 43 | grok | `grok::agent-event` | `grok::on-agent-event` | `{ session_id }` | as row 25 | `FeedPayload` | acp:src/handler.rs, acp:src/agent_feed.rs |
| 44 | grok | `grok::raw-event` | `grok::on-raw-event` | `{ session_id }` | as row 25 | `FeedPayload` (streaming-json event) | none in repo |
| 45 | hermes | `hermes::agent-event` | `hermes::on-agent-event` | `{ session_id }` | as row 25 (Python: `match(observer=..., call=...)`) | `FeedPayload` | acp:src/handler.rs, acp:src/agent_feed.rs |
| 46 | hermes | `hermes::raw-event` | `hermes::on-raw-event` | `{ session_id }` | as row 25 | `FeedPayload` (inbound platform delivery) | none in repo |
| 47 | ide | `shell::changed` | `shell::on-changed` | `{ path, include_ignored? }` | match(o: `root\|include_ignored` of the root resolved at add, c: the producing watch's key); added: resolve and jail-check `path`, retain the `(root, include_ignored)` watch and await its start (`ObserveRejection` on failure); removed: release it; the pump calls the function once per event | `ChangedEvent { path, kind, root, dir, ignored }` | ide:ui/src/page/live.ts, ide:ui/src/page/git-watch.ts, a2ui:ui/src/live.ts |
| 48 | ide | `shell::git-changed` | `shell::on-git-changed` | `{ path }` | match(o and c: canonical path); added: retain the path's `GitWatch`; removed: release it | `GitChangedEvent` | ide:ui/src/page/git-watch.ts |
| 49 | ide | `shell::job-finished` | `shell::on-job-finished` | `{ job_id? }` | match(o: `args.job_id` or `*`, c: `[input.job_id, *]`); added: when the job already finished, after `obs.accepted()` call through the observer with its retained terminal event | `JobFinishedEvent` | kits:deepwork/agents/deepwork.md (through `harness::observe`) |
| 50 | ide | `shell::turns::changed` | `shell::turns::on-changed` | none; filters `input.session_id` | default; the 200 ms coalescing stays in the producer | `TurnsChangedEvent { session_id }` | ide:ui/src/page/turn.ts |
| 51 | iii-directory | `directory::skills::on-change` | `directory::skills::on-change` | none; filters `input.op`, `input.name` | default | `SkillsChange { op, name? }` | harness:src/skills.rs, iii-directory:ui/src/page/browser.tsx |
| 52 | iii-directory | `directory::system-prompts::on-change` | `directory::system-prompts::on-change` | none | default | `SystemPromptsChange { op, name? }` | none in repo |
| 53 | iii-directory | `directory::agents::on-change` | `directory::agents::on-change` | none | default | `AgentsChange { op, name? }` | iii-directory:ui/src/page/browser.tsx |
| 54 | iii-directory | `directory::kits::on-change` | `directory::kits::on-change` | none; filters `input.op`, `input.plan_id` | default | `KitsChange { op: plan \| discard \| progress \| apply \| updates, kit?, plan_id?, steps? }` | iii-directory:ui/src/kits/api.ts |
| 55 | ios-simulator | `ios-simulator::device-changed` | `ios-simulator::on-device-changed` | none; filters `input.tenant`, `input.udid` | default | `DeviceChangedEvent` | ios-simulator:ui/src/overlay/SimulatorOverlay.tsx, ios-simulator:ui/src/page/index.tsx |
| 56 | ios-simulator | `ios-simulator::media-changed` | `ios-simulator::on-media-changed` | none; filters `input.tenant`, `input.udid` | default | `MediaChangedEvent` | ios-simulator:ui/src/page/Media.tsx |
| 57 | ios-simulator | `ios-simulator::frame-event` | `ios-simulator::on-frame-event` | `{ tenant?, udid? }` | match(o: `tenant or *\|udid or *`, c: `{tenant,*}\|{udid,*}`, four keys); the bridge gates on `is_observed_for(call)` before encoding; viewers observe with `max_in_flight: 1` | `FrameEvent` | ios-simulator:ui/src/page/live.ts |
| 58 | ios-simulator | `ios-simulator::activity` | `ios-simulator::on-activity` | none; filters `input.tenant`, `input.udid` | default | `ActivityEvent` | ios-simulator:ui/src/overlay/SimulatorOverlay.tsx |
| 59 | kanban | `kanban:change` | `kanban::on-change` | `{ ticket_id?, parent_id?, release_id?, ticket?: 'full' \| 'summary', events? }` | custom: selects observers with `matches_change(args, input)` (`matches_config` for `config.updated`) and `obs.matches(call)`; passes `ticket: full` observers to `next`; calls each `ticket: summary` observer through itself with the summarized payload (`Void`) | `ChangeEvent` or `ConfigEvent` | kanban:ui/src/shared.tsx, kanban:native-ui/src/api.ts |
| 60 | kanban | `kanban:comment` | `kanban::on-comment` | `{ ticket_id?, parent_id?, author?, exclude_author?, root_only?, ticket? }` (one of `ticket_id`, `parent_id` required) | custom: `matches_comment(args, input)` and the summary split of row 59 | `CommentEvent` | kanban:skills/SKILL.md (through `harness::observe`) |
| 61 | llm-router | `router::ready` | `router::on-ready` | none | default; added: when the router is ready, after `obs.accepted()` call through the observer with the ready event | `RouterReadyEvent {}` | provider-anthropic, provider-claude-code, provider-command-code, provider-deepseek, provider-elevenlabs, provider-github-copilot, provider-kimi, provider-llamacpp, provider-openai, provider-openai-codex, provider-opencode-go, provider-openrouter, provider-sarvam, provider-xai, provider-zai (each src/register.rs, through `router_client::observe_router_ready`), cursor:src/provider.ts, llm-router:tests/integration.rs |
| 62 | llm-router | `router::models::changed` | `router::models::on-changed` | none; filters `input.provider` | default | `ModelsChangedEvent { provider, count }` | sentinel:ui/src/settings/useModelCatalog.ts, context-manager:src/adapters/cache.rs, security-scan:ui/src/page/security-scan-data.ts, ade:ui/src/configuration-forms/model-catalog.ts, ade:web/src/lib/models-catalog.ts, provider-xai:tests/integration.rs, llm-router:tests/integration.rs |
| 63 | llm-router | `router::provider::changed` | `router::provider::on-changed` | none; filters `input.provider`, `input.op` | default | `ProviderChangedEvent { provider, op: register \| available \| unavailable }` | ade:web/src/lib/models-catalog.ts |
| 64 | memory | `memory::item-changed` | `memory::on-item-changed` | none; filters `input.bank` | default | `ItemChangedEvent { event_type, bank, memory, reason? }` | memory:ui/src/page/useMemoryLive.ts |
| 65 | memory | `memory::bank-changed` | `memory::on-bank-changed` | none; filters `input.bank` | default | `BankChangedEvent { event_type, bank }` | memory:ui/src/page/useMemoryLive.ts |
| 66 | opencode | `opencode::agent-event` | `opencode::on-agent-event` | `{ session_id }` | as row 25 | `FeedPayload` | acp:src/handler.rs, acp:src/agent_feed.rs |
| 67 | opencode | `opencode::raw-event` | `opencode::on-raw-event` | `{ session_id }` | as row 25 | `FeedPayload` (raw JSON event) | none in repo |
| 68 | opengantry | `gantry::verdict` | `gantry::on-verdict` | none | default | `VerdictEvent { status, error_code, repo_root, msn_id, mission_rel_path }` | none in repo |
| 69 | pi | `pi::agent-event` | `pi::on-agent-event` | `{ session_id }` | as row 25 | `FeedPayload` | acp:src/handler.rs, acp:src/agent_feed.rs |
| 70 | pi | `pi::raw-event` | `pi::on-raw-event` | `{ session_id }` | as row 25 | `FeedPayload` (raw event) | none in repo |
| 71 | quick-tunnel | `quick-tunnel::changed` | `quick-tunnel::on-changed` | none; filters `input.tunnel_id` | default | `Snapshot { tunnel_id, generation, status, public_url, error }` | whatsapp:cloud_tunnel.go, github:src/webhooks/wiring.rs |
| 72 | sandbox-code-runner | `sandbox-code-runner::event` | `sandbox-code-runner::on-event` | none; filters `input.kind` | default; added/removed on the function retain and release the fleet watcher, so `sandbox::list` is polled only while observed | `Event { kind, runtime_id?, lang?, sandbox_id?, function_id?, daemon_absent?, sandboxes? }` | sandbox-code-runner:ui/src/page/useFleet.ts, sandbox-code-runner:ui/src/page/CreateDialog.tsx, sandbox-code-runner:ui/src/page/chip.tsx |
| 73 | secrets | `secrets::changed` | `secrets::on-changed` | none; filters `input.name` (a bare name, both stores) or `input.ref` (`secret://NAME`, `env://NAME`) | default | `ChangedEvent { name, ref, action, fingerprint?, updated_at }` | crates/judge-provider:src/secrets.rs, database:src/configuration.rs, llm-router:src/register.rs, storage:src/configuration.rs, judge:ui/src/session/index.tsx, secrets:tests/engine.rs, secrets:tests/boot.rs |
| 74 | security-scan | `security-scan::run-changed` | `security-scan::on-run-changed` | none; filters `input.repository`, `input.run_id` | default | `RunChangedEventV1` | security-scan:ui/src/page/useSecurityRunsLive.ts, security-scan:tests/live_change_feed.rs |
| 75 | security-scan | `security-scan::reconciliation-changed` | `security-scan::on-reconciliation-changed` | none; filters as row 74 | default | `ReconciliationChangedEventV1` | security-scan:ui/src/page/useSecurityRunsLive.ts, security-scan:tests/live_change_feed.rs |
| 76 | security-scan | `security-scan::action-changed` | `security-scan::on-action-changed` | none; filters as row 74 | default | `ActionChangedEventV1` | security-scan:ui/src/page/security-actions.js |
| 77 | sentinel | `sentinel::group-changed` | `sentinel::on-group-changed` | none; filters `input.group_id`, `input.op` | default; the emitter keeps its 1 s coalescing | `GroupChangedEventV1 { op, group_id, status, previous_status?, occurrence_count, reason? }` | sentinel:ui/src/page/index.tsx |
| 78 | sentinel | `sentinel::investigation-changed` | `sentinel::on-investigation-changed` | none; filters `input.group_id` | default | `InvestigationChangedEventV1` (typed `response_format`) | sentinel:ui/src/page/index.tsx |
| 79 | session-manager | `session::created` | `session::on-created` | none; filters `input.session_id`, `input.session_metadata.<key>` | default | `SessionCreatedEvent` plus `session_metadata?` | cron:ui/src/page/index.tsx, iii-directory:ui/src/session-chip/index.tsx, security-scan:ui/src/page/useFollowAnalysisChat.ts, security-scan:ui/src/page/ScanRequestForm.tsx, ade:web/src/lib/sessions/events.ts, session-manager:tests/steps/engine_steps.rs |
| 80 | session-manager | `session::message-added` | `session::on-message-added` | none; filters `input.session_id`, `input.message.role`, `input.session_metadata.<key>` | default | `MessageAddedEvent` plus `session_metadata?` | slack:src/functions/bindings.rs, telegram-bot:src/functions/mod.rs, voice:ui/src/lib/voice-chat.ts, ade:web/src/lib/sessions/events.ts, session-manager:tests/steps/engine_steps.rs |
| 81 | session-manager | `session::message-updated` | `session::on-message-updated` | none; filters as row 80 | default; the hottest source, gated on `is_observed()` | `MessageUpdatedEvent` plus `session_metadata?` | as row 80 |
| 82 | session-manager | `session::status-changed` | `session::on-status-changed` | none; filters `input.session_id`, `input.session_metadata.<key>` | default | `StatusChangedEvent` plus `session_metadata?` | telegram-bot:src/functions/mod.rs, ade:web/src/lib/sessions/events.ts, session-manager:tests/steps/engine_steps.rs |
| 83 | session-manager | `session::meta-updated` | `session::on-meta-updated` | none; filters as row 82 | default | `MetaUpdatedEvent` plus `session_metadata?` | iii-directory:ui/src/session-chip/index.tsx, security-scan:ui/src/page/ScanRequestForm.tsx, ade:web/src/lib/sessions/events.ts, session-manager:tests/steps/engine_steps.rs |
| 84 | session-manager | `session::deleted` | `session::on-deleted` | none; filters as row 82 | default | `SessionDeletedEvent` plus `session_metadata?` | memory:src/main.rs, approval-gate:src/main.rs, harness:src/configuration.rs, ade:web/src/lib/sessions/events.ts, session-manager:tests/steps/engine_steps.rs |
| 85 | session-manager | `session::store::events` | `session::store::on-events` (main fs-mode instance only) | none | default | `EventEnvelope { function_id, payload, session_metadata? }` (`function_id` names the session event function; the wire field of `session::store::publish-events` follows) | session-manager:src/events.rs (each bridged instance observes the main instance's function) |
| 86 | stories | `stories:changed` | `stories::on-changed` | `{ workspace?, project?, kinds? }` | custom: for each observer that `obs.matches(call)`, `narrow_changed(args, input)`; an empty result skips it, any other is called through the observer (`Void`) | `ChangedEvent` (narrowed per observer) | stories:ui/src/explorer.tsx |
| 87 | stories | `stories:build` | `stories::on-build` | none; filters `input.workspace` | default | `BuildEvent` | stories:ui/src/explorer.tsx |
| 88 | storage | `storage::object-created` | `storage::on-object-created` | `{ bucket, handler_timeout_ms? }` | custom, with match(o: `args.bucket`, c: `input.bucket`) picks observers; the hook calls each matching observer through itself with the event (`timeout_ms: handler_timeout_ms`, default 60000) and returns `{ acked }`, true when every target returned `null` or `{ ack: true }`; pollers and the local backend ack upstream on `acked`; added: `CONFIG_ERROR` when the bucket has no wired notification source (under the reconfigure read lock) | `ObjectCreatedEvent` | storage:tests/e2e/workers/harness/src/runner.ts, storage:config.yaml.example, storage:README.md |
| 89 | storage | `storage::object-deleted` | `storage::on-object-deleted` | `{ bucket, handler_timeout_ms? }` | as row 88 | `ObjectDeletedEvent` | as row 88 |
| 90 | tmux | `tmux::changed` | `tmux::on-changed` | none | default; added: after `obs.accepted()`, call through the observer with the current overview (`Void`); the poll loop gates on `is_observed()` | `Overview` | tmux:ui/src/lib/live.ts |
| 91 | voice | `voice::transcript` | `voice::on-transcript` | none; filters `input.session_id` | default; the session's `output_function_id` keeps its plain `call` | `TranscriptEvent` | none in repo |
| 92 | voice | `voice::session-started` | `voice::on-session-started` | none; filters `input.session_id` | default | `SessionStartedEvent` | none in repo |
| 93 | voice | `voice::session-stopped` | `voice::on-session-stopped` | none; filters `input.session_id` | default | `SessionStoppedEvent` | none in repo |
| 94 | voice | `voice::model-progress` | `voice::on-model-progress` | none | default | `ModelProgressEvent` | voice:ui/src/lib/progress.ts |
| 95 | voice | `voice::speech-ended` | removed: it has no emitter and no consumer | | | | |
| 96 | whatsapp | `whatsapp::events` | `whatsapp::on-events` | none; filters `input.session_id`, `input.kind` | default; the event loop calls only when `IsObserved` and the event is current | `Event { session_id, sequence, kind, timestamp_ms, data }` | whatsapp:ui/src/page.tsx |
| 97 | whatsapp | `whatsapp::cloud::events` | `whatsapp::cloud::on-events` | none; filters `input.kind`, `input.chat_id`, `input.sender_id`, `input.message_type`, `input.status`, `input.business_account_id`, `input.phone_number_id`, `input.group_id`, `input.participant_id`, `input.field`, `input.event` | default | `CloudEvent` | none in repo |
| 124 | workflow | `workflow::run-completed` | `workflow::on-run-completed` | none; filters `input.run_id`, `input.status` | default; finalize calls it once per terminal run, before the terminal status is stored (a crash in between calls it again) | `RunCompletedEvent { run_id, status: completed \| failed \| cancelled, result, result_error }` | none in repo |
| 98 | worktree | `worktree::created` | `worktree::on-created` | none; filters `input.repo_path`, `input.worktree_id`, `input.session_id` | default | `CreatedEvent` | worktree:ui/src/page/index.tsx |
| 99 | worktree | `worktree::claimed` | `worktree::on-claimed` | none; filters as row 98 | default | `ClaimedEvent` | worktree:ui/src/page/index.tsx |
| 100 | worktree | `worktree::released` | `worktree::on-released` | none; filters as row 98 | default | `ReleasedEvent` | worktree:ui/src/page/index.tsx |
| 101 | worktree | `worktree::removed` | `worktree::on-removed` | none; filters as row 98 | default | `RemovedEvent` | worktree:ui/src/page/index.tsx |
| 102 | worktree | `worktree::landed` | `worktree::on-landed` | none; filters as row 98 | default | `LandedEvent` | worktree:ui/src/page/index.tsx, ade:web/src/hooks/use-worktree-events.ts |
| 103 | worktree | `worktree::land-blocked` | `worktree::on-land-blocked` | none; filters as row 98 | default | `LandBlockedEvent` | worktree:ui/src/page/index.tsx, ade:web/src/hooks/use-worktree-events.ts |

Session events carry `session_metadata` (the session's metadata when it has
any), so a binding's `metadata` subset match becomes filters on
`input.session_metadata.<key>`. Message events carry the role at
`input.message.role`; a `roles` binding becomes
`{ 'input.message.role': { $in: roles } }`.

The `acp` worker observes `<ns>::on-agent-event` with args `{ session_id }`
per owned session; its `--events-trigger-type` flag and
`IIIACP_EVENTS_TRIGGER_TYPE` variable become `--events-function` and
`IIIACP_EVENTS_FUNCTION`, defaulting to `<brain ns>::on-agent-event`.

### 2.3 Harness

Decision 30. Every function is private; the harness's own consumers and the
agent observers it creates are in §5.1.

| # | Worker | Old type | Function | Args | Delivery | Event | Consumers |
|---|---|---|---|---|---|---|---|
| 104 | harness | `harness::ready` | `harness::on-ready` | none | default; added: when the harness is ready, after `obs.accepted()` call through the observer with the ready event | `ReadyEvent { status: 'ready', timestamp }` | none in repo |
| 105 | harness | `harness::turn-started` | `harness::on-turn-started` | none; filters `input.session_id`, `input.parent_session_id`, `input.parent.session_id` | default; added: `FORBIDDEN` when `obs.target` is `harness::spawn` | `TurnStartedEvent { session_id, turn_id, timestamp, depth, message_preview?, parent?, parent_session_id? }` | ide:src/turns.rs, ide:ui/src/page/turn.ts, openwiki:src/index.mjs, voice:ui/src/lib/voice-chat.ts, ade:web/src/lib/backend/turn-events-live.ts |
| 106 | harness | `harness::turn-completed` | `harness::on-turn-completed` | none; filters as row 105 | default; added as row 105 | `TurnCompletedEvent { session_id, turn_id, status, terminal, timestamp, result?, result_error?, reason?, parent?, parent_session_id?, context? }` | memory:src/main.rs, sentinel:src/triggers.rs, eval:src/main.rs, slack:src/functions/bindings.rs, telegram-bot:src/functions/mod.rs, approval-gate:src/main.rs, ide:src/turns.rs, ide:ui/src/page/turn.ts, openwiki:src/index.mjs, security-scan:src/main.rs, workflow:src/configuration.rs, onboarding:ui/src/conditions.ts, voice:ui/src/lib/voice-chat.ts, ade:web/src/lib/backend/turn-events-live.ts |
| 107 | harness | `harness::message-queued` | `harness::on-message-queued` | none; filters `input.session_id` | default | `MessageQueuedEvent { session_id, entry_id, queued_at, timestamp }` | ade:web/src/lib/backend/turn-events-live.ts |
| 108 | harness | `harness::triggers-changed` | `harness::on-observers-changed` | none; filters `input.session_id` | default | `ObserversChangedEvent { session_id, timestamp }` (a doorbell: consumers read `harness::observers::list`) | ade:web/src/lib/backend/turn-events-live.ts, cron:ui/src/page/index.tsx |
| 109 | harness | `harness::compaction` | `harness::on-compaction` | none; filters `input.session_id`, `input.compaction_id` | default | `CompactionEvent { session_id, compaction_id, phase, timestamp, ... }` | ade:web/src/lib/backend/real.ts |
| 110 | harness | `harness::session-tree-deletion` | `harness::on-session-tree-deletion` | none; filters `input.session_id`, `input.operation_id` | default | `Snapshot` (deletion progress) | ade:web/src/lib/sessions/delete-tree.ts |
| 111 | harness | `harness::hook::pre-turn` | `harness::hook::on-pre-turn` | `HookArgs` (§5.1) | custom: the hook chain (§5.1) | the hook input; targets return the hook decision | ide:src/turns.rs |
| 112 | harness | `harness::hook::pre-generate` | `harness::hook::on-pre-generate` | `HookArgs` | custom: the hook chain | the hook input | judge:src/mentions/mod.rs, memory:src/main.rs, web:src/configuration.rs, code-runner:src/functions/mod.rs, iii-directory:src/hook.rs, sandbox-code-runner:src/configuration.rs, fp:src/guidance.rs, workflow:src/configuration.rs, browser:src/scrapling/mod.rs, ade:src/context_tracking.rs, ade:src/functions/working_directory.rs |
| 113 | harness | `harness::hook::post-generate` | `harness::hook::on-post-generate` | `HookArgs` | custom: the hook chain | the hook input | none in repo |
| 114 | harness | `harness::hook::pre-trigger` | `harness::hook::on-pre-call` | `HookArgs` | custom: the hook chain | the hook input | a2ui:src/hook.rs, ide:src/turns.rs, approval-gate:src/configuration.rs, workflow:src/configuration.rs, ade:src/context_tracking.rs, ade:src/functions/working_directory.rs |
| 115 | harness | `harness::hook::post-trigger` | `harness::hook::on-post-call` | `HookArgs` | custom: the hook chain | the hook input | ide:src/turns.rs, approval-gate:src/configuration.rs |
| 116 | harness | `harness::hook::post-turn` | `harness::hook::on-post-turn` | `HookArgs` | custom: the hook chain | the hook input | ide:src/turns.rs, harness:src/functions/subscribe.rs (agent self-validators) |
| 117 | harness | `timer` | `harness::on-timer` | `{ at }` (epoch ms) | match(o: `args.at`, c: `input.scheduled_at`); added: validate `at` (`CONFIG_ERROR` past `MAX_TIMER_MS` or in the past), arm a one-shot per distinct `at`; removed: cancel it with its last observer; at the deadline the harness calls the function once | `TimerEvent { scheduled_at, actual_at }` | kits:deepwork/agents/deepwork.md (through `harness::observe`, which turns `in_ms` into `at`), harness:tests/integration/src/scenarios/timer_wake.rs |

### 2.4 ADE

Decisions 29 and 32. Every function is private.

| # | Worker | Old type | Function | Args | Delivery | Event | Consumers |
|---|---|---|---|---|---|---|---|
| 118 | ade | `console:script` | `console::ui::asset` | `{ kind: 'script', path }` | none (§5.2): added: validate `path` and `kind`; after `obs.accepted()`, fetch the content through the observer with `{ path }`; a failed fetch calls `obs.reject('UI_ASSET_FETCH_FAILED', …)`; a later observer of the same path supersedes it (`obs.reject('UI_ASSET_SUPERSEDED', …)`); removed: drop the asset | none (the function is never called; its handler returns its input) | crates/console-ui:src/lib.rs (every worker with a console UI), vscode:src/index.ts, claude-code:src/terminal/ui.ts, pi:src/terminal/ui.ts, onboarding:src/index.mjs, whatsapp:ui.go |
| 119 | ade | `console:style` | `console::ui::asset` | `{ kind: 'style', path }` | as row 118 | none | as row 118 |
| 120 | ade | `console:module` | `console::ui::asset` | `{ kind: 'module', path }` | as row 118 | none | crates/console-ui:src/lib.rs (ide, kanban, tmux) |
| 121 | ade | `console:assets` | `console::ui::on-assets-changed` | none | default; added: after `obs.accepted()`, call through the observer with `{ op: 'sync', assets }` | `UiAssetsEvent { op: 'sync' \| 'set' \| 'delete', assets?, asset?, path? }` | ade:web/src/lib/ui-loader.tsx |
| 122 | ade | `console::workspace::changed` | `console::workspace::on-changed` | none | default; added: after `obs.accepted()`, call through the observer with `{}`; store writes call the function, throttled by `RING_GAP` | `WorkspaceChangedEvent {}` | ade:web/src/hooks/use-workspace-tabs.ts, onboarding:ui/src/use-tour.ts, onboarding:ui/src/spotlight.ts |
| 123 | ade | `console::compose::changed` | `console::compose::on-changed` | none; filters `input.kind`, `input.file`, `input.namespace` | default; added: start the state watcher with the first observer; removed: close it with the last | `ComposeChangedEvent { kind, file, namespace, state_dir, path, captured_at }` | ade:web/src/pages/Workers/index.tsx |

Rows: 104 from the worker map (§2.1, §2.2; row 124 follows the worker order,
its number follows the ADE rows), 14 harness (§2.3) and 6 ADE (§2.4): **124**.

## 3. Engine and Compose sources workers observe

Spec §11, decisions 6, 7 and 26. All are private functions hosted in
`default` (Compose's in its namespace) whose handler returns the event; none
takes args.

| Old type | Function | Filters | Event | Consumers |
|---|---|---|---|---|
| `configuration` `{ configuration_id, event_types }` | `configuration::on-updated` | `{ 'input.id': { $eq: id }, 'input.event_type': { $eq: 'configuration:updated' } }` | `ConfigurationEvent { type, event_type: configuration:registered \| configuration:updated \| configuration:deleted, id, name, description, schema, old_value, new_value, metadata }` | every `<worker>::on-config-change`, through crates/config-client (§4.1) or directly (a2ui, bridge, browser, canvas, claude-code, code-runner, codex, computer, context-manager, cron, cursor, database, devin, document, email, github, grok, harness, http, ide, iii-directory, ios-simulator, kanban, llm-router, memory, memory-consolidate, opencode, openwiki, pdf, pi, provider-xai, pubsub, queue, session-manager, slack, state, storage, tailscale, telegram-bot, voice, vscode, whatsapp, worktree, ade, rbac-proxy); browser tabs judge:ui/src/session/index.tsx, judge-openai:ui/src/configuration/index.tsx, judge-typesafe:ui/src/configuration/index.tsx (`input.id` only) |
| `configuration` waiting for an entry to exist | `configuration::on-updated` | `{ 'input.id': { $eq: id }, 'input.event_type': { $eq: 'configuration:registered' } }` | as above | ade onboarding waits (web/src/lib/onboarding/event-wait.ts) |
| `engine::functions-available` | `engine::on-functions-changed` | none | `{ event: 'functions_changed', functions }` | queue:src/trigger.rs, iii-directory:src/functions/search.rs, llm-router:src/register.rs, harness:src/discovery.rs, rbac-proxy:src/configuration.rs, judge:ui/src/session/index.tsx, ade:ui/src/catalog/engine.ts |
| `engine::workers-available` | `engine::on-workers-changed` | none, or `input.event` | `{ event: worker_connected \| worker_disconnected \| worker_metadata_updated, worker_id }` | queue:src/trigger.rs, harness:src/engine_events.rs, ade (useWorkerPresence) |
| `worker` `{ operations, stages }` (no provider exists in the engine, iii-compose or this repo) | `engine::on-workers-changed` | `{ 'input.event': { $in: ['worker_connected', 'worker_disconnected'] } }` (`add` → connected, `remove` → disconnected); the target re-reads what it needs. A target that looks a worker up by name also selects `worker_metadata_updated`: the engine emits `worker_connected` before the worker announces its name with `engine::workers::register` (telegram-bot) | as above | telegram-bot:src/approval_gate.rs, iii-directory:src/main.rs, ade (useWorkerLifecycle) |
| `log` `{ level: 'error' }` | `observability::on-log` | `{ 'input.severity_number': { $gte: 17 } }` | `LogEvent { timestamp_unix_nano, severity_number, severity_text, body, attributes, trace_id, span_id, resource, service_name, ... }` | sentinel:src/triggers.rs |
| `trace` `{ status: 'error' }` | `observability::on-span` | none: the event carries only trace ids, so the target reads the spans and checks their status | `SpanEvent { trace_ids }` | sentinel:src/triggers.rs, ade:ui/src/catalog/span-feed.ts, ade:web/src/lib/traces-activity.ts |
| `stream` | `stream::on-changed` | `input.stream_name`, `input.group_id` | `StreamChangedEvent { event_type, timestamp, stream_name, group_id, id?, event }` | a2ui:ui/src/live.ts (stored `stream` bindings) |
| `stream:join`, `stream:leave` | `stream::on-join`, `stream::on-leave` | `input.stream_name`, `input.group_id` | `StreamJoinLeaveEvent { subscription_id, stream_name, group_id, id?, context? }` | none in repo |
| `compose-operation` `{ operation_id?, terminal_only? }` | `compose::on-operation-progress` (Compose namespace) | `{ 'input.operation_id': { $eq: id } }` and `{ 'input.terminal': { $eq: true } }` for `terminal_only` | `ProgressEvent { sequence, operation_id, container?, phase, detail, current?, total?, elapsed_ms, terminal }` | judge:ui/src/session/engine.ts, ide:ui/src/page/new-worker.ts, workers-dev:src/compose.rs, harness:src/bindings/compose.rs, ade (followOperation), harness:tests/quickstart/wait_for_compose_operation.py |
| none | `engine::on-observers-changed` | `input.function_id`, `input.target`, `input.state` | `{ observer_id, function_id, target, state, reason? }` | ade Observers page (§5.2) |

Engine builtins workers read: `engine::functions::list|info` rows carry
`observer_count`, `args_format`, `private`, `observable`;
`engine::workers::info` carries `functions` and `observers`;
`engine::observers::list|get|dispose` replace
`engine::triggers::list|info` and `engine::registered-triggers::list|info`
(decision 6).

## 4. Shared library APIs

### 4.1 crates/config-client

`try_bind` and `RegisterTriggerInput` leave the API: `observe` cannot fail
synchronously, so callers observe directly. `BindingSlot` becomes
`ObserverSlot`.

```rust
/// A live observer that follows a boolean knob (the guidance hooks).
#[derive(Clone, Default)]
pub struct ObserverSlot(Arc<Mutex<Option<ObserverHandle>>>);

impl ObserverSlot {
    /// on → `observe()` once; off → dispose the observer (spawned) and drop
    /// the handle. Idempotent under repeated config events.
    pub fn reconcile(
        &self,
        enabled: bool,
        observe: impl FnOnce() -> ObserverHandle,
        on_msg: &str,
        off_msg: &str,
    );
    pub fn is_observing(&self) -> bool;
}

/// Registers the internal `fn_id` reload handler and observes
/// `configuration::on-updated` for `config_id`:
/// filters `{ input.id: { $eq: config_id }, input.event_type: { $eq: "configuration:updated" } }`,
/// `Deliver::Output`. Call `.run()` on the result once to close the boot gap.
pub fn on_change<F, Fut>(
    iii: &Arc<IIIClient>,
    config_id: &'static str,
    fn_id: &'static str,
    description: &'static str,
    reload: F,
) -> Reload
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static;

/// Unchanged signature; resolves `<worker>::configuration-id` with `iii.call`.
pub async fn follow(
    iii: &Arc<IIIClient>,
    worker: &str,
    fn_id: &'static str,
    description: &'static str,
) -> Result<tokio::sync::watch::Receiver<Option<Value>>, String>;
```

`OnConfigChangeEvent { id: Option<String> }` is documented as the
`configuration::on-updated` event delivered as output (handlers read `id`
only). `ensure`, `fetch`, `Reload` and `initialization.rs` keep their
signatures; their calls use `iii.call(CallRequest::new(..).namespace("default"))`.
Dependents of `ObserverSlot`: iii-directory, web, sentinel,
sandbox-code-runner, fp, workflow, browser.

### 4.2 crates/console-ui

`ConsoleUi` keeps its builder (`new`, `script`, `style`, `module`,
`content_function_id`, `watch_env`, `watch_default_dir`, `register`). Each
asset is one observer:

```rust
iii.observe(
    Source::function("console::ui::asset").args(json!({ "kind": kind.as_str(), "path": path })),
    content_function_id,   // `<worker>::ui-content`
)
```

- `AssetKind::trigger_type()` becomes `AssetKind::as_str() -> "script" | "style" | "module"`.
- `WatchedAsset.handle` is an `ObserverHandle`; hot reload observes the new
  asset, then disposes the old handle.
- The content function contract is unchanged: input
  `UiContentInput { path }`, output `UiContentResult { content, content_type }`.
  The ADE calls it through the asset's observer.
- `register_configuration_identity(iii: &IIIClient, worker: &str, id: &'static str)`
  is unchanged: it registers `<worker>::configuration-id`, which consumers
  call with `iii.call`.
- Boot tests that asserted `registertrigger` frames for `console:*` assert
  `observe` frames for `console::ui::asset` with `{ kind, path }` args.

### 4.3 crates/judge-provider, crates/provider-integration-testkit

```rust
// judge-provider/src/secrets.rs (was register_secret_trigger)
pub fn observe_secret_changes(iii: &IIIClient, cache: Arc<SecretCache>, fn_id: &str) -> ObserverHandle;
// registers `fn_id`, then observes `secrets::on-changed` with Deliver::Output.
// The observer stays pending while the secrets worker is absent.
```

The engine test's fake provider registers a private `secrets::on-changed`
that the test calls, and it waits on the observer handle's state
(`handle.ready()`).

`provider-integration-testkit`: `runtime::call(..)` uses `iii.call`;
`codex_login::observe_state(client)` returns
`(ObserverHandle, mpsc::UnboundedReceiver<ObservedEvent>)` and observes
`state::on-changed` (no filter, `Deliver::Output`).

### 4.4 crates/node-core and crates/python-core (code-runner guests)

Guest surface (`globalThis.iii`, pinned in `tests/golden/iii-surface.txt`):

```ts
iii.call(request: { function_id: string; payload?: unknown; action?: { type: 'void' }; timeout_ms?: number }
                | { observer: GuestObserver; payload?: unknown; action?: { type: 'void' } }): Promise<unknown>
iii.trigger                       // the same function as iii.call
iii.registerFunction(id: string, handler: (input) => unknown, options?: {
  description?: string; request_format?: object; response_format?: object;
  args_format?: object; private?: boolean;
}): { unregister(): void }
iii.onObserverAdded(id: string, hook: ((obs: GuestObserver) => void | Promise<void>) | null): void
iii.onObserverRemoved(id: string, hook: ((obs: GuestObserver) => void | Promise<void>) | null): void
iii.observe(source: string | { function_id: string; namespace?: string; args?: object; filters?: object; deliver?: 'envelope' | 'input' | 'output' },
            target: string,
            options?: { id?: string; metadata?: object; max_fires?: number; expires_in_ms?: number; max_in_flight?: number }
): Promise<{ id: string; dispose(): Promise<void> }>
iii.shutdown(), iii.files    // unchanged

interface GuestObserver { id: string; function_id: string; args: object; filters: object; target: string }
```

A guest hook that throws rejects the observer (`OBSERVE_REJECTED`, or the
thrown error's `code`). Guests have no `onObserverDeliver`: a guest source
is delivered by iii, or calls its observers itself with
`iii.call({ observer, payload })`. `registerTrigger`, `registerTriggerType`
and `unregisterTriggerType` leave the surface.

Host side (`crates/node-core/src/engine.rs`):

```rust
pub trait Engine: Send + Sync + 'static {
    fn call(&self, fn_id: String, payload: Value, timeout_ms: u64, action: Option<String>)
        -> BoxFuture<'static, CallResult>;
    fn call_observer(&self, observer_id: String, payload: Value, timeout_ms: u64, action: Option<String>)
        -> BoxFuture<'static, CallResult>;
    fn register(&self, fn_id: String, description: Option<String>, request_format: Option<Value>,
                response_format: Option<Value>, options: GuestFunctionOptions, handler: ProxyHandler)
        -> UnregisterFn;
    fn on_observer_added(&self, fn_id: String, hook: Option<ProxyHandler>);
    fn on_observer_removed(&self, fn_id: String, hook: Option<ProxyHandler>);
    fn observe(&self, input: Value) -> BoxFuture<'static, Result<String, String>>;   // observer id
    fn dispose_observer(&self, observer_id: String) -> BoxFuture<'static, Result<(), String>>;
}

pub struct GuestFunctionOptions { pub args_format: Option<Value>, pub private: bool }
```

The hook proxies are invoked with `{ method: "observerAdded" | "observerRemoved", observer: GuestObserver }`.
`register_trigger`, `unregister_trigger`, `register_trigger_type`,
`TriggerCallback`, `trigger_callback_payload`, `GuestTriggerInput`,
`parse_guest_trigger`, `TRIGGER_TYPE_METHODS` and
`RegistrationKind::{Trigger, TriggerType}` are removed;
`RegistrationKind::Observer` shares the registration budget.
`code-runner/src/node_bus.rs` implements the trait over the Rust SDK
(`on_observer_added_for` with an adapter that calls the guest), and vets
observe targets as it vetted trigger targets.
`crates/node-core/tests/configuration-contract.mjs` mocks `call` and
`observe` and asserts an observer of `configuration::on-updated` with an
`input.id` filter.

`crates/python-core` is call-only: the guest `iii.call(request)` with
`iii.trigger` as the same function; error strings say `iii.call`.

### 4.5 packages/console-ui (the extension host API)

```ts
export interface ExtensionIii {
  browserId: string
  call<T = unknown>(functionId: string, payload?: Record<string, unknown>,
                    options?: { timeoutMs?: number }): Promise<T>
  /** @deprecated the same function as `call` */
  trigger: ExtensionIii['call']
  /** Registers `<functionId>::<browserId>` (metadata.internal = true). */
  on<P = unknown>(functionId: string, handler: (payload: P) => void | Promise<void>): () => void
  /** Observes `source` with `target` (a function registered with `on`, by its full id). Returns its disposer. */
  observe(source: string | ObserveSource, target: string, options?: ObserverOptions): () => void
  addConnectionStateListener(handler: (state: unknown) => void): () => void
}

export interface ObserveSource {
  function_id: string; namespace?: string; args?: Record<string, unknown>
  filters?: Record<string, unknown>; deliver?: 'envelope' | 'input' | 'output'
}
export interface ObserverOptions {
  id?: string; metadata?: Record<string, unknown>
  max_fires?: number; expires_in_ms?: number; max_in_flight?: number
}
```

`registerTrigger` is removed. `observe` defaults `deliver` to `'output'` for
extension pages.

```ts
// hooks.mjs
export function useWorkerLive<T>(opts: {
  iii: ExtensionIii
  sources: readonly (string | ObserveSource)[]    // each observed with target `${handlerId}::${browserId}`
  fetch: () => Promise<T>
  handlerId: string
  pollMs?: number
}): WorkerLive<T>
```

`configuration.mjs` resolves `<worker>::configuration-id` with `iii.call`
(`Pick<ExtensionIii, 'call'>`).

Chat slots (decision 32), host-owned, mirrored from
`ade/web/src/types/injectable-ui.ts`:

| Before | After |
|---|---|
| `host.functionTriggers.register(renderer: FunctionTriggerRenderer)` | `host.functionCalls.register(renderer: FunctionCallRenderer)` |
| `FunctionTriggerMessage`, role `'function-trigger'` | `FunctionCallMessage`, role `'function-call'`; readers accept `'function-trigger'` in stored conversations |
| `host.triggerRenderers.register(renderer: TriggerActivityRenderer)` | `host.observerRenderers.register(renderer: ObserverActivityRenderer)` |
| `TriggerActivityRenderer.isMatch(triggerType: string)` | `ObserverActivityRenderer.isMatch(functionId: string)` (the observed function) |
| `TriggerActivityMessage { id, kind, triggerType, config?, label?, action?, conditions?, delivery, lifecycle, subscriptionId?, triggerId?, payload?, firedAt?, outcome?, retirementReason? }` | `ObserverActivityMessage { id, kind: 'registration' \| 'fired' \| 'retirement', functionId, args?, filters?, label?, action?, conditions?, delivery, lifecycle, observerId?, payload?, firedAt?, outcome?, retirementReason? }` |

### 4.6 packages/native-ui and crates/native-ui

```ts
interface Iii {
  call<T = JsonValue>(fn: string, payload?: JsonValue, opts?: CallOptions): Promise<T>   // unchanged
  /** Observes a function of the app's namespace (`<ns>::…`) and runs `handler` with each event. */
  on<P = JsonValue>(
    functionId: string,
    source: { args?: JsonObject; filters?: JsonObject },
    handler: (payload: P) => void,
  ): Unsubscribe & { ready: Promise<void> }
}
export function useObserve<P = JsonValue>(
  functionId: string,
  source: () => { args?: JsonObject; filters?: JsonObject } | null,
  handler: (payload: P) => void,
  opts?: { enabled?: () => boolean },
): void   // replaces useTrigger
```

Wire (catalog.json): request method 2 `on` takes
`{ sub: int, fn: string, args: json, filters: json }`; inbound entry code 9 is
`observed { sub, payload }`; the RemoteScreen `wire.frames` prop is
`{ fn, args }` (`{ fn: 'browser::on-frame-event', args: { session_id: '$session' } }`).
`checkOwn('bind')` accepts only `<ns>::` ids. The host observes with
`deliver: 'output'`. Conformance fixtures, `testing/` (`emitObserved`) and the
generated catalog are regenerated; the native app host changes in lockstep.
`crates/native-ui` pins `iii-sdk = "=0.26.0"`; its API is unchanged.

### 4.7 llm-router provider_scaffold

```rust
// llm-router/src/provider_scaffold/router_client.rs
pub async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error>;
// body: iii.call(CallRequest { function_id, payload, action: None, timeout_ms: Some(15_000) })

/// Observes `router::on-ready` with `target` (`provider::<id>::on_router_ready`),
/// Deliver::Output. Pending while llm-router is down; the router sends the
/// ready event to an observer added after it is ready.
pub fn observe_router_ready(iii: &IIIClient, target: &str) -> ObserverHandle;
```

`resolve`, `reconcile`, `models_get`, `register` keep their signatures;
`provider_scaffold/state.rs` uses `iii.call`. Every `provider-*` worker
replaces its `router::ready` binding with `observe_router_ready`.

## 5. Harness, ADE and rbac-proxy surfaces

### 5.1 Harness (decision 30)

**Hook functions.** `harness::hook::on-pre-turn`, `on-pre-generate`,
`on-post-generate`, `on-pre-call`, `on-post-call`, `on-post-turn`. Private,
`args_format`:

```ts
interface HookArgs {
  functions?: string[]      // on-pre-call, on-post-call: target function id globs
  sessions?: string[]       // session id globs (omit = every session)
  payload?: object          // on-post-turn template mode: the payload sent to the target
  result_into?: string      // on-post-turn template mode: JSON pointer for the result (default /value)
  retry_prompt?: string     // on-post-turn: corrective prompt sent when the validator denies
  priority?: number         // chain order, ascending; ties by target (default 0)
  timeout_ms?: number       // per call (default 5000)
  on_error?: 'fail_open' | 'fail_closed'   // default fail_closed for pre-* and on-post-turn, fail_open otherwise
  inject_prompt?: string    // on-pre-generate: static system-prompt contribution
}
```

`inject_prompt` is an arg: the harness never sees observer metadata. Workers
that set it as binding metadata (web, sandbox-code-runner, fp, workflow,
browser) pass it in args.

How the harness calls them: at each hook point the harness calls the hook
function with the hook input (the envelope its targets receive today), and
awaits the result. The function's deliver hook:

1. selects the observers whose `sessions` and `functions` globs match the
   input and that `obs.matches(call)`;
2. orders them by `priority`, then target;
3. calls each in order through its observer,
   `iii.call(CallRequest::observer(&obs, payload))`, bounded by `timeout_ms`
   (the payload is the hook input, or `payload` with the result at
   `result_into` in post-turn template mode);
4. applies `on_error` when a call fails, times out, or is refused
   (`OVERFLOW`, `OBSERVER_NOT_ACTIVE`): `fail_closed` denies;
5. merges the decisions as today, adds the `inject_prompt` of the matching
   `on-pre-generate` observers, and returns the merged outcome.

A consumer:

```rust
iii.observe(
    Source::function("harness::hook::on-pre-generate")
        .args(json!({ "priority": 100, "timeout_ms": 3000, "on_error": "fail_open" })),
    "judge::mentions::pre-generate",
);
```

**Event functions:** rows 104-110 and `harness::on-timer` (row 117).

**Agent tools.** `harness::observe` and `harness::unobserve` are the agent's
observe surface. The harness intercepts them at the dispatch point and
creates the observer itself.

```ts
// harness::observe
{
  function_id: string           // the observed function
  namespace?: string            // default: the session's namespace, then `default`, then Compose's
  args?: object                 // for harness::on-timer: { at } or { in_ms } (turned into `at`)
  filters?: Filter
  target?: { function_id: string; payload?: object; event_into?: string }  // omitted: wake this session
  label?: string
  once?: boolean
  conditions?: { function_id: string; config?: object }[]
  lifecycle?: { once?: boolean; max_fires?: number; expires_in_ms?: number }
}
→ { observer_id: string; once: boolean; note?: string }

// harness::unobserve
{ id: string } → { removed: boolean }
```

- `harness::*` sources are refused for agents except `harness::on-timer`.
- The harness observes
  `Source::function(function_id).namespace(ns).args(args).filters(filters)`
  with target `harness::observer::deliver`, options
  `id(binding.id)`, `metadata({ "__binding": binding.id })` and
  `expires_in` from the lifecycle as a backstop; never `max_fires` (the
  harness counts delivered fires). Deliveries take the `Observed` envelope.
  The registration waits on `handle.ready()` with a bounded timeout, so
  `INVALID_ARGS`, `FORBIDDEN`, `NOT_OBSERVABLE` and rejections reach the
  agent.
- Stored bindings with a `trigger_type` are translated at boot through §2
  and §3; one that does not translate is retired with an owner notice.
- `harness::triggers::list` and `harness::triggers::unregister` become
  `harness::observers::list { session_id? } → { observers: ObserverRow[] }`
  and `harness::observers::dispose { observer_id, session_id } → { removed }`.
  `ObserverRow { observer_id, function_id, namespace?, args, filters, target,
  conditions, label, action, once, max_fires, expires_at, fires, created_at }`.
- Records: new transcript records use the custom type `observer_fired` with
  `observer_id`, `function_id`, `args`, `filters` fields; readers accept
  `trigger_fired`.
- Call surface: `harness::function::trigger` becomes
  `harness::function::call`; the expose mode and model-facing tool
  `agent_trigger` become `agent_call` (configuration readers accept
  `agent_trigger` in stored config). Prompts, SKILL.md files and the kits'
  agent docs teach `harness::observe`.

The harness observes, for its own plumbing: `engine::on-workers-changed`,
`engine::on-functions-changed`, `directory::skills::on-change`,
`cron::on-tick` (`{ expression }`, pending sweep), `session::on-deleted`,
`configuration::on-updated`, `compose::on-operation-progress`
(`input.terminal` `$eq` true), `console::ui::asset`, and
`harness::hook::on-post-turn` for agent self-validators.

### 5.2 ADE (decisions 29, 32)

- **`console::ui::asset`** (rows 118-120). The ADE keeps one serialized queue
  for asset and subscriber events. `onObserverAdded` validates `{ kind, path }`
  (`path` at most 512 bytes) and returns; a task awaits `obs.accepted()` and
  fetches the content with
  `iii.call(CallRequest::observer(&obs, json!({ "path": path })))`, retried
  as today, and stores it keyed by `path` (last writer wins). A failed fetch
  calls `obs.reject("UI_ASSET_FETCH_FAILED", ..)`; a superseded asset's
  observer gets `obs.reject("UI_ASSET_SUPERSEDED", ..)`. Each change calls
  `console::ui::on-assets-changed` with `set` or `delete`.
- **`console::ui::on-assets-changed`** (row 121): browser tabs observe it;
  each new observer receives `{ op: 'sync', assets }` through itself after it
  is accepted. `console::ui-manifest` is unchanged.
- **`console::workspace::on-changed`** (row 122) and
  **`console::compose::on-changed`** (row 123).
- **Consumers in the ADE:** `console::on-config-change` observes
  `configuration::on-updated` (§3); `console::working-directory::stamp-session`
  and the context-tracking functions observe `harness::hook::on-pre-call` with
  args `{ functions, timeout_ms, on_error }` and
  `harness::hook::on-pre-generate`.
- **Browser client** (`web/src/lib/iii-client.ts`): `IiiClient` has
  `call` (with `trigger` as the same function) and
  `observe(source, target, options) => () => void`; `registerTrigger` and the
  `RegisterTriggerInput` export are removed. `engine-trigger.ts` becomes
  `observe-events.ts` with
  `subscribeObserved(functionId, { args?, filters? }, onEvent, options) => Promise<() => void>`.
  `SESSION_TRIGGER_TYPES` becomes `SESSION_FUNCTIONS` (rows 79-84).
- **Observers screen** (decision 32): the Triggers screen is Observers
  (`ext:observers`; stored `ext:triggers` workspace and local state
  migrates), built on `engine::observers::list|get|dispose`, grouped by
  observed function, refreshed by `engine::on-observers-changed`.
  `trigger-kinds.ts` becomes `observer-kinds.ts` and reads args
  (`http::on-request` `args.method`/`args.path`, `cron::on-tick`
  `args.expression`, `queue::on-message` `args.topic`) and filters
  (`state::on-changed` `input.scope`/`input.key`). Functions show an
  Observers tab (observers of it, observers targeting it).
- **Proxy** (`ade/src/proxy.rs`): drops a browser `registerfunction` whose id
  is one of the ADE's provider ids (`console::ui::*`,
  `console::workspace::on-changed`, `console::compose::on-changed`) and strips
  `private`, `args_format` and `observable` from browser registrations; it
  keeps the namespace and `metadata.internal` stamping.

### 5.3 rbac-proxy (decision 31)

What other workers and operators depend on:

- **Features.** The proxy advertises `observe.v1` and `observe.v2`, forwards
  `observe`, `unobserve`, `observerack`, `invocationsobserved`,
  `localinvocationstats` downstream and `functionregistered`,
  `observeradded`, `observerremoved`, `observeresult`,
  `observerstatechanged` upstream, with the session prefix rewritten both
  ways. It passes `invocationresult.observed`, `output_is_input`, and
  top-level `args_format`, `private` and `observable` on `registerfunction`.
  `local-dispatch.v1` is allowed only when the session has no
  `middleware_function_id`.
- **Calls through observers** (`invokefunction` with
  `activation.observer_id`) are allowed only for observers of functions the
  connection hosts; the engine checks the rest (decision 13).
- **Removed trigger frames** (`registertriggertype`,
  `unregistertriggertype`, `registertrigger`, `unregistertrigger`,
  `triggerregistrationresult`) are dropped with one `warn` per connection
  and message type, naming the replacement.
- **RBAC config.** `on_trigger_registration_function_id` and
  `on_trigger_type_registration_function_id` (RbacConfig) and
  `allowed_trigger_types` / `allow_trigger_type_registration` (AuthResult)
  are ignored with one `warn` at load naming the observe equivalent
  (decision 10). Observe registrations are gated by
  `on_observer_registration_function_id`, called with the engine's input
  `{ observer_id, function_id, namespace, args, filters, target_function_id,
  target_namespace, metadata, context }`; an object answer may rewrite
  `function_id`, `target_function_id`, `args`, `filters` and `metadata`, and
  anything else is `FORBIDDEN`. Source and target are both checked with
  `access_allowed`, and `engine::*` targets are refused. The
  `on_function_registration_function_id` input gains `private`,
  `observable` and `args_format`, and its answer may replace them.
- **Discovery.** `engine::observers::list|get` are discovery overrides
  filtered by access (observed function and target), with the prefix
  stripped; `engine::observers::dispose` is limited to observers the
  connection created. `engine::triggers::*` and
  `engine::registered-triggers::*` leave `DISCOVERY_FUNCTIONS`.
- **Its own observers:** `rbac-proxy::on-config-change` observes
  `configuration::on-updated` (§3), and `rbac-proxy::on-functions-available`
  becomes `rbac-proxy::on-functions-changed`, observing
  `engine::on-functions-changed` (`iii-permissions.yaml` follows the rename).
- The ADE configuration form for the rbac-proxy
  (`ade/ui/src/configuration-forms/specs/collections-services.ts`) shows
  `rbac.on_observer_registration_function_id` and drops the two trigger hook
  fields.

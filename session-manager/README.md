# session-manager

Durable, reactive store for conversations. A session is an append-only
log of typed message entries (user, assistant, function_result, custom
— with optional fork branches) plus a small metadata record (title,
description, coarse status, app-defined metadata). Any worker or client
appends and reads over the bus; every mutation fires an event consumers
observe (`session::on-*`, iii 0.25) or bind to as a trigger type, so a
chat UI, bot, or dashboard renders live with no polling and no separate
publish call. It runs no agent logic — it is
the conversation database the [harness](https://github.com/iii-hq/workers/tree/main/harness) family drives, and
it is independently useful as a real-time chat store for any app.

## Install

```bash
iii trigger compose::add worker=session-manager
```

`iii trigger compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

## Quickstart

Create a session, append a message, read the transcript back:

```rust
use iii_sdk::{protocol::TriggerRequest, register_worker, InitOptions};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let created = iii.trigger(TriggerRequest {
        function_id: "session::create".into(),
        payload: json!({ "title": "Weather question", "metadata": { "owner": "u_1" } }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;
    let session_id = created["session_id"].as_str().unwrap().to_string();

    iii.trigger(TriggerRequest {
        function_id: "session::append".into(),
        payload: json!({
            "session_id": session_id,
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": "What's the weather?" }],
                "timestamp": 1717800000000_i64
            }
        }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    let transcript = iii.trigger(TriggerRequest {
        function_id: "session::messages".into(),
        payload: json!({ "session_id": session_id }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    println!("{transcript:#?}");
    Ok(())
}
```

To render live instead of polling, register a handler function and
observe the matching `session::on-*` function with the binding config as
args (iii 0.25; see [Observable functions](#observable-functions-iii-025)):

```rust
use iii_sdk::{Error, Observed, RegisterFunction, Source};
use serde_json::{json, Value};

iii.register_function(
    "my-ui::on-message-updated",
    RegisterFunction::new_async(|event: Observed<Value>| async move {
        // event.input: full updated message + monotonic revision; keep the highest
        let update = event.input.unwrap_or_default();
        println!("{} rev {}", update["entry_id"], update["revision"]);
        Ok::<_, Error>(json!({ "ok": true }))
    }),
);

let live = iii.observe(
    Source::function("session::on-message-updated")
        .args(json!({ "session_id": session_id, "roles": ["assistant"] })),
    "my-ui::on-message-updated",
);
live.ready().await?;
```

On an older engine or SDK, bind the trigger type instead (deprecated in
0.25; the handler then receives the event payload itself, not an
`Observed` envelope):

```rust
use iii_sdk::protocol::RegisterTriggerInput;

#[allow(deprecated)]
iii.register_trigger(RegisterTriggerInput::new(
    "session::message-updated",
    "my-ui::on-message-updated",
    json!({ "session_id": session_id, "roles": ["assistant"] }),
))?;
```

Streaming an assistant reply uses the same primitives: append an
(initially empty) assistant message, then call `session::update-message`
as tokens arrive — each update fires `session::message-updated` (and
`session::on-message-updated`) with an incremented `revision`.

The full function surface (21 `session::*` functions: lifecycle,
messages, branching/fork, attachments) is documented in the spec and on each
function's registered schema — see `iii worker info session-manager`.
`session::messages` items carry the entry `origin` (hook annotations —
e.g. which memory bank fed a generate), so consumers can render
provenance without a second lookup.

Sessions can be mentioned in a chat as `@session(id="<session_id>")`
(see [`crates/mention-contract`](../crates/mention-contract/README.md)).
Two internal functions provide it: `session::mention::search` ranks
sessions by title words or id (top-level chats before sub-agents, e2e
sessions and the asking session left out) and `session::mention::get`
returns the pill, preview card and agent summary — never the parked
draft — and declares the provider, whose details call is `session::get`.

## Custom trigger types

Six trigger types cover every mutation. Each binding's `config` filters
what reaches the handler; all configs additionally accept
`metadata` — a subset-equality match against `SessionMeta.metadata`
(the tenancy hook, e.g. `{ "owner": "u_1" }`). Malformed configs are
rejected at registration.

| Trigger type | Fires when | Config filters |
|---|---|---|
| `session::created` | A session is created (create / ensure / fork) | `metadata` |
| `session::message-added` | An entry is appended | `session_id`, `roles`, `metadata` |
| `session::message-updated` | A message's content changes (streaming deltas) | `session_id`, `roles`, `metadata` |
| `session::status-changed` | Status moves between idle/working/done/error | `session_id`, `metadata` |
| `session::meta-updated` | Title/description/metadata change | `session_id`, `metadata` |
| `session::deleted` | A session and its entries are removed | `session_id`, `metadata` |

Delivery is fire-and-forget, at-least-once, and unordered — reconcile
message updates by `revision` (keep the highest) and transcript order
by the parent chain, never by arrival order.

### Observable functions (iii 0.25)

On a 0.25 engine a new consumer observes an owner-delivered function
instead of binding a trigger type. Each shares its trigger type's
subscriber registry and takes that type's binding config as args, so an
observer receives exactly the event a binding with the same config
receives, at the same moments and through the same filters:

| Function | Args | Event |
|---|---|---|
| `session::on-created` | `{ metadata? }` | as `session::created` |
| `session::on-message-added` | `{ session_id?, roles?, metadata? }` | as `session::message-added` |
| `session::on-message-updated` | `{ session_id?, roles?, metadata? }` | as `session::message-updated` |
| `session::on-status-changed` | `{ session_id?, metadata? }` | as `session::status-changed` |
| `session::on-meta-updated` | `{ session_id?, metadata? }` | as `session::meta-updated` |
| `session::on-deleted` | `{ session_id?, metadata? }` | as `session::deleted` |

```typescript
iii.registerFunction('my-ui::render', async ({ observer_id, input }) => render(input))

const live = iii.observe(
  { function_id: 'session::on-message-updated', args: { session_id, roles: ['assistant'] } },
  'my-ui::render',
)
```

Events are routed inside session-manager: invoking any `session::on-*`
function directly is refused with `FORBIDDEN`. Malformed args are
rejected when the observer is added, as malformed configs are at
registration. The six trigger types stay registered, unchanged, so
consumers that bind them keep working on 0.24 and 0.25 engines (binding
is deprecated in iii 0.25; new consumers observe instead); the
internal `session::store::events` feed is unchanged.

## Storage adapters

Two adapters, selected by an `adapter` block (a `name` plus a nested
`config`):

- **`fs`** (default) — one append-only JSONL file per session under
  `data_dir` (`<encoded_session_id>.jsonl`): typed `meta` / `entry` /
  `leaf` records, replayed last-wins on startup, file removed on
  session delete. This is the durable, single-instance setup. An append
  is one `append` record (entry, leaf and metadata together) with a
  `commit` marker per committed entry; a build from before these record
  types skips them as malformed lines, so **do not roll back to an older
  session-manager once this version has written to a `data_dir`**: the
  entries appended since would not replay, and older metadata would
  return without an error.
- **`bridge`** — this instance keeps all domain logic (idempotency,
  revisions, branching, locks) but stores through a **main** instance
  running its own session-manager (`adapter name: fs`) on another iii
  engine, via the internal `session::store::*` protocol.

Event propagation in a bridge topology: the main is the single fan-out
point. A bridged instance publishes each mutation's events to the main
(`session::store::publish-events`); the main delivers to its own
subscribers and forwards an envelope to **every** attached bridged
instance over its internal `session::store::events` feed; each bridge
re-emits through its local trigger types with its own subscribers'
filters. With several bridged instances attached to one main, a
mutation made anywhere reaches every instance's subscribers exactly
once. (Consequence: an originator's own subscribers hear events after
the round trip through the main — same at-least-once, unordered
contract as always.)

The `session::store::*` functions are deployment plumbing served only
by fs-mode instances — deny them to agents like every other mutating
surface.

## Configuration

Runtime settings live in the **`configuration` worker** under id
**`session-manager`**. At startup the worker registers its JSON Schema,
fetches the live, env-expanded value via `configuration::get`, and binds a
`configuration` trigger so changes apply without a restart. Persisted values
default to `./data/configuration/session-manager.yaml` (the configuration
worker's `fs` adapter) — edit that file directly or call `configuration::set`
and the change propagates. `adapter` is an adjacently tagged enum, so the
console's worker-config form renders a variant picker (`fs` / `bridge`) and
only the selected adapter's `config` fields (`data_dir`, or `url` /
`timeout_ms`), plus the list limits — all as editable inputs.

```yaml
adapter:
  name: fs                                   # fs | bridge
  config:
    data_dir: data/session-manager           # relative to III_COMPOSE_DIR

# adapter:
#   name: bridge
#   config:
#     url: ws://127.0.0.1:49134   # main engine WebSocket URL
#     timeout_ms: 5000            # per store/publish call timeout

default_list_limit: 50   # page size when list/messages omit `limit`
max_list_limit: 500      # hard cap on any requested `limit`
```

**Reload policy.** Every field hot-reloads on `configuration:updated`, no
restart required. `default_list_limit` / `max_list_limit` swap the shared
snapshot the list/messages calls read. A change to the `adapter` (fs↔bridge, a
new `data_dir`, a bridge url/timeout) rebuilds the store and event plumbing and
swaps it in atomically; the new store's current state is then replayed through
the `session::*` triggers so open subscribers (the console sidebar/transcript,
the harness, ...) stay live without a refetch. A reload that cannot be built
(e.g. an unreadable `data_dir`, or a self-referential bridge `url`) keeps the
previous runtime (last-good) and is surfaced by `session::config-status`. An
invalid bridge `config` is still rejected at parse time, and at boot a
misconfigured bridge is fatal (it never silently falls back to a local fs store).
Switching `data_dir` or bridge `url` changes the backing storage immediately and
does not migrate existing sessions.

**First boot.** When no value is stored yet for id `session-manager`, the worker
registers [`WorkerConfig::default()`](src/config.rs) as `initial_value`.
Optionally pass `--config <path>` to seed from a YAML file instead (one-time,
never overwrites an existing stored value). `${VAR:default}` placeholders in
stored values are expanded by the configuration worker on every read.

## Local development & testing

```bash
cargo run --release -- --url ws://127.0.0.1:49134
cargo test                       # unit + manifest + BDD (engine scenarios self-skip)
cargo test --test bdd -- --tags @pure    # no engine required
cargo test --test bdd -- --tags @engine  # requires a running `iii`
```

The BDD suite under [`tests/features/`](tests/features) is the
behavioural contract: @pure scenarios drive the production handlers
against a real fs store over a tempdir with deterministic ids/clock
(including restart-replay scenarios); @engine scenarios drive the same
code over a live engine, including JSONL persistence readbacks, real
trigger fan-out, and full bridge propagation (two bridged instances +
the main, asserting who received what, exactly once).

## Architecture documentation

Deep documentation lives in [`architecture/`](architecture):
[`architecture/integration.md`](architecture/integration.md) is the
self-contained handoff contract for workers that build on this one
(functions, events, filters, error codes, patterns, topologies);
[`architecture/internals.md`](architecture/internals.md) is the
maintainer deep-dive (module map, invariants, storage formats, event
pipeline, testing architecture).

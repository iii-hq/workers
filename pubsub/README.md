# pubsub

> **Deprecated:** pubsub is deprecated (pubsub) and will be removed in an upcoming release. Behavior is unchanged for now. Migration guide: https://github.com/iii-hq/iii/blob/6901a589dd39d9c34f9d5b6bd8bc82170ee21548/docs/next/upgrading/migrate-from-streams.mdx
>
> This covers the `publish` function and the `pubsub` configuration entry.
> For new code, observe a worker-owned event function for fire-and-forget
> fan-out, or use the `queue` worker (`iii::durable::publish` observed
> through `queue::on-message`) when delivery must not be lost. While
> deprecated, this worker logs a warning (target `iii::deprecation`) once at
> startup and on `publish` calls, at most once every 10 minutes per entry
> point and caller. Payloads and topic names are never logged. Traffic on
> topic `stream.events` (the engine's internal iii-stream bridge) does not
> warn.

Topic-based publish/subscribe messaging. Registers the `publish` service
function. This standalone worker replaces the legacy built-in pub/sub
service. A subscriber is an observer of `publish` with a filter on
`input.topic`: every call of `publish` with that topic reaches it, as a
fire-and-forget delivery.

## Install

```bash
iii call compose::add worker=pubsub
```

`iii call compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

## Functions

| Function | Input | Returns | Args | Delivery | Event |
|---|---|---|---|---|---|
| `publish` | `{ topic, data }` | `null` | none; filter on `input.topic` | iii delivers every call to each observer whose filters match; observers use `deliver: 'input'` | `{ topic, data }` (the call's input) |

`publish` is registered under the bare id **`publish`** (matching the builtin
exactly, so existing callers and the engine's stream bridge keep working).
Any worker may call it.

| Field | Description |
|---|---|
| `topic` | Topic to publish to. Empty → `topic_not_set` error. |
| `data` | JSON payload delivered to each observer of the topic. |

## Quickstart

Observe `publish` with a filter on the topic, then call it with
`{ topic, data }`:

```rust
use iii_sdk::{
    errors::Error, register_worker, CallRequest, Deliver, InitOptions, RegisterFunction, Source,
};
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    iii.register_function(
        "orders::on_created",
        RegisterFunction::new_async(|event: Value| async move {
            // event: { "topic": "orders", "data": { "id": 42 } }
            println!("order event: {}", event["data"]);
            Ok::<Value, Error>(Value::Null)
        }),
    );

    iii.observe(
        Source::function("publish")
            .filters(json!({ "input.topic": { "$eq": "orders" } }))
            .deliver(Deliver::Input),
        "orders::on_created",
    );

    // Any caller (function, worker, or the stream bridge) publishes with:
    iii.call(CallRequest {
        function_id: "publish".into(),
        payload: json!({ "topic": "orders", "data": { "id": 42 } }),
        action: None,
        timeout_ms: Some(5000),
    })
    .await?;

    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

```typescript
iii.observe(
  { function_id: 'publish', filters: { 'input.topic': { $eq: 'orders' } }, deliver: 'input' },
  'orders::on-created',
)
```

With `deliver: 'input'` the observer receives `{ topic, data }`, so one
target can serve several topics (`{ 'input.topic': { $in: ['orders',
'invoices'] } }`). Filters read any field of the event, including
`input.data.*`. Delivery is fire-and-forget: `publish` returns a null result
once dispatched and does not wait for or report observer outcomes.
Publishing to a topic nobody observes is a silent no-op; publishing with an
empty topic fails with a `topic_not_set` error and reaches no observer.

## Configuration

| Field | Default | Description |
|---|---|---|
| `adapter.name` | `local` | Backend: `local` (events stay on this engine) or `redis` (Redis Pub/Sub carries each event to the pubsub workers of other engines). |
| `adapter.config.redis_url` | `redis://localhost:6379` | Redis connection URL (only for the `redis` adapter). |

With the `redis` adapter every pubsub worker on the same Redis publishes the
events published on its engine to the Redis channel `iii:pubsub` as
`{ origin, topic, data }` and listens on it. An event from another worker is
called as `publish` on this engine (with call metadata
`{ "pubsub_relayed_from": <origin> }`), so the observers here receive it
exactly as a local publish; it is not carried to Redis again.

Configuration is owned by the `configuration` worker — edit it from the
console (**Configuration → Workers → pubsub**) or seed it once via
`--config <file>.yaml` on first boot. The adapter **hot-swaps at runtime**: a
change rebuilds the backend, routes new publishes through it, then tears
down the previous backend. A build failure keeps the previous backend and
config.

### Requires removing the legacy built-in pub/sub service

The legacy built-in pub/sub service also owns the `publish` function. Two
owners of the same function on one engine collide, so this worker requires
it to be absent: omit it from the engine's `config.yaml` (a config that
doesn't list a worker won't run it).

On boot, this worker queries the engine for connected workers and refuses to
start with a clear error if the legacy built-in is still active, so a stale config
fails loudly instead of silently racing the built-in worker for ownership of
`publish`.

## Parity vs builtin

| Behavior | Builtin | This worker |
|---|---|---|
| Service functions | `publish` (bare id — the `#[service]` prefix is discarded by the macro) | `publish` (exact same id) |
| `publish` input | `{topic, data}` | same |
| `publish` success result | null | same |
| `publish` empty topic | error `topic_not_set` / "Topic is not set" | same (code prefixes the message: `topic_not_set: Topic is not set`) |
| Selecting a topic | `subscribe` binding `{topic}` | observer filter `{ 'input.topic': { $eq: topic } }` (any operator, any event field) |
| Delivery payload | raw `data` | `{ topic, data }` with `deliver: 'input'` |
| Delivery semantics | fire-and-forget, results ignored, no retries | same |
| No subscribers | silent no-op | same |
| Backends | local (in-process), redis | same |
| Redis wire format | one channel per topic, raw `data` | one channel `iii:pubsub`, `{ origin, topic, data }` |
| Adapter hot-swap | gated build-first | same |
| Config entry id | legacy engine configuration | `pubsub` (new entry, seeded on first boot) |
| Config schema shape | hand-built `oneOf` union (local/redis) | plain derive — accepted values identical; explicit Console form |

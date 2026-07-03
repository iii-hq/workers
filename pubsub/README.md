# pubsub

Topic-based publish/subscribe messaging. Registers the `subscribe` trigger
type and the `publish` service function, replacing the engine builtin
`iii-pubsub`. Any function bound to a `subscribe` trigger on a topic receives
every event published to that topic; `publish` fans an event's `data` out to
all of a topic's subscribers as fire-and-forget invocations.

## Install

```bash
iii worker add pubsub
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

## Quickstart

Bind a function to the `subscribe` trigger type with a `topic`, then call the
`publish` function with `{ topic, data }`:

```rust
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{InitOptions, RegisterFunction, errors::Error, register_worker};
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    iii.register_function(
        "orders::on_created",
        RegisterFunction::new_async(|data: Value| async move {
            println!("order event: {data}");
            Ok::<Value, Error>(Value::Null)
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_pubsub::TRIGGER_TYPE.to_string(), // "subscribe"
        function_id: "orders::on_created".into(),
        config: json!({ "topic": "orders" }),
        metadata: None,
    })?;

    // Any caller (function, worker, or the stream bridge) publishes with:
    iii.trigger(TriggerRequest {
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

The subscriber receives the **raw** `data` value (no envelope). Delivery is
fire-and-forget: `publish` returns a null result once dispatched and does not
wait for or report subscriber outcomes. Publishing to a topic with no
subscribers is a silent no-op; publishing with an empty topic fails with a
`topic_not_set` error.

## Trigger type

This worker always registers the `subscribe` trigger type. Bind a function to
it with:

| Field | Required | Default | Description |
|---|---|---|---|
| `topic` | yes | — | Topic to subscribe to. A missing or empty topic registers nothing (a warning is logged). |
| `condition_function_id` | no | — | Accepted for schema parity with the builtin; **never evaluated**. |

## The `publish` function

Registered under the bare id **`publish`** (matching the builtin exactly, so
existing callers and the engine's stream bridge keep working). Input:

| Field | Description |
|---|---|
| `topic` | Topic to publish to. Empty → `topic_not_set` error. |
| `data` | JSON payload delivered verbatim to each subscriber. |

Returns a null result on success.

## Configuration

| Field | Default | Description |
|---|---|---|
| `adapter.name` | `local` | Backend: `local` (in-process broadcast) or `redis` (cross-instance Redis Pub/Sub). |
| `adapter.config.redis_url` | `redis://localhost:6379` | Redis connection URL (only for the `redis` adapter). |

Configuration is owned by the `configuration` worker — edit it from the
console (**Configuration → Workers → pubsub**) or seed it once via
`--config <file>.yaml` on first boot. The adapter **hot-swaps at runtime**: a
change rebuilds the backend, re-subscribes the live subscriptions onto it
*before* the swap (no delivery gap — a brief double-delivery overlap is
accepted for fire-and-forget pub/sub), then tears down the previous backend.
A build failure keeps the previous backend and config.

### Requires removing the built-in `iii-pubsub` worker

The built-in `iii-pubsub` worker also owns the `subscribe` trigger type and
the `publish` function. Two owners of the same trigger type on one engine
collide — whichever registers last wins — so this worker requires `iii-pubsub`
to be absent: omit it from the engine's `config.yaml` (a config that doesn't
list a worker won't run it).

On boot, this worker queries the engine for connected workers and refuses to
start with a clear error if `iii-pubsub` is still active, so a stale config
fails loudly instead of silently racing the built-in worker for ownership of
`subscribe`/`publish`.

## Parity vs builtin

| Behavior | Builtin | This worker |
|---|---|---|
| Trigger type | `subscribe` | same |
| Trigger config | `{topic}` (+ advertised `condition_function_id`, never evaluated) | same (still never evaluated) |
| Service functions | `publish` (bare id — the `#[service]` prefix is discarded by the macro) | `publish` (exact same id) |
| `publish` input | `{topic, data}` | same |
| `publish` success result | null | same |
| `publish` empty topic | error `topic_not_set` / "Topic is not set" | same (code prefixes the message: `topic_not_set: Topic is not set`) |
| Delivery payload | raw `data`, no envelope | same |
| Delivery semantics | fire-and-forget, results ignored, no retries | same |
| No subscribers | silent no-op | same |
| Subscribe with missing/empty topic | warn + register nothing (Ok) | same |
| Backends | local (in-process), redis | same |
| Redis: subscriptions per topic per instance | one (second warns, dropped) | same |
| Local unsubscribe | drops the ENTIRE topic entry (bug: kills co-subscribers) | removes only the given id (**deliberate fix**) |
| Adapter hot-swap | gated build-first, resubscribe-before-swap | same |
| Config entry id | `iii-pubsub` | `pubsub` (new entry, seeded on first boot) |
| Config schema shape | hand-built `oneOf` union (local/redis) | plain derive — accepted values identical, plainer console form |

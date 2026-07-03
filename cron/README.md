# cron

Schedules registered functions with cron expressions. Any function bound to a
`cron` trigger is invoked by this worker at the next matching UTC time. The
worker replaces the built-in `iii-cron` worker while keeping the trigger type
and payload shape stable.

## Install

```bash
iii worker add cron
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

## Quickstart

Register a function and bind it to this worker's trigger type (`cron`) with an
`expression`:

```rust
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{errors::Error, register_worker, InitOptions, RegisterFunction};
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    iii.register_function(
        "jobs::tick",
        RegisterFunction::new_async(|payload: Value| async move {
            println!("cron fired: {payload}");
            Ok::<Value, Error>(json!({"ok": true}))
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_cron::TRIGGER_TYPE.to_string(),
        function_id: "jobs::tick".into(),
        config: json!({ "expression": "*/5 * * * * *" }),
        metadata: None,
    })?;

    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

The function receives:

```json
{
  "trigger": "cron",
  "job_id": "<trigger-id>",
  "scheduled_time": "2026-07-03T12:00:00+00:00",
  "actual_time": "2026-07-03T12:00:00.123456789+00:00"
}
```

## Configuration

| Field | Default | Description |
|---|---|---|
| `adapter.name` | `local` | Lock backend. Use `local` for process-local locking or `redis` for multi-instance mutual exclusion. |
| `adapter.config.redis_url` | `redis://localhost:6379` | Redis URL when `adapter.name` is `redis`. |

Configuration is owned by the `configuration` worker - edit it from the
console (**Configuration -> Workers -> cron**) or seed it once via
`--config <file>.yaml` on first boot. Changing the lock adapter hot-swaps the
scheduler under a serialized apply lock: existing jobs are stopped, re-created
with the new backend, and never run in two scheduler instances at once.

## Trigger type

This worker always registers the `cron` trigger type. Bind a function to it
with:

| Field | Required | Default | Description |
|---|---|---|---|
| `expression` | yes | - | Cron expression parsed by the Rust `cron` crate. Six-field `sec min hour day month weekday` expressions are supported, and a seventh year field is accepted. |
| `condition_function_id` | no | - | Function invoked before the scheduled handler. Only an explicit JSON `false` return blocks the fire; missing/null/truthy returns allow it. Condition errors skip that fire. |

All schedules use UTC. Missed fires while the worker is stopped are skipped;
there is no catch-up replay.

### Requires removing the built-in `iii-cron` worker

The built-in `iii-cron` worker also owns the `cron` trigger type. Two owners
of the same trigger type on one engine collide - whichever registers last
wins - so this worker requires `iii-cron` to be absent: omit it from the
engine's `config.yaml` (a config that doesn't list a worker won't run it).

On boot, this worker queries the engine for connected workers and refuses to
start with a clear error if `iii-cron` is still active, so a stale config
fails loudly instead of silently racing the built-in worker for ownership of
`cron`.

## Parity vs builtin

| Behavior | Builtin | This worker |
|---|---|---|
| Expressions | 6-7 field (`cron` crate) | same |
| Timezone | UTC only | same |
| Missed runs | skipped, no catch-up | same |
| Condition | only explicit `false` blocks | same |
| Lock TTL | 30s | same |
| Lock backends | kv (process-local), redis | local (process-local), redis |
| Service functions | none | none |

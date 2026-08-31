# cron

Schedules registered functions with cron expressions. Any function bound to a
`cron` trigger is invoked by this worker at the next matching UTC time. The
worker replaces the legacy built-in cron worker while keeping the trigger type
and payload shape stable.

## Install

```bash
iii trigger compose::add worker=cron
```

`iii trigger compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

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
        trigger_type: "cron".to_string(),
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

## Console page

While the worker is connected it injects a **cron** page into the console
(`#/ext/cron`): every agent-owned schedule with its cadence, next UTC run and
fire count, the cron bindings other workers registered for themselves, and a
composer that turns "every weekday at 09:00, summarise open PRs" into a
registered schedule. Schedules created there live in a session of their own,
so each routine keeps its own transcript.

## Trigger type

This worker always registers the `cron` trigger type. Bind a function to it
with:

| Field | Required | Default | Description |
|---|---|---|---|
| `expression` | yes | - | Cron expression parsed by the Rust `cron` crate. Six-field `sec min hour day month weekday` expressions are supported, and a seventh year field is accepted. |
| `condition_function_id` | no | - | Function invoked before the scheduled handler. Only an explicit JSON `false` return blocks the fire; missing/null/truthy returns allow it. Condition errors skip that fire. |

All schedules use UTC. Missed fires while the worker is stopped are skipped;
there is no catch-up replay.

Write the day of week as a name (`Mon` ... `Sun`). Numerically the crate counts
Sunday as 1, so `0 0 9 * * 1` fires on Sunday, not Monday.

## Console trigger activity

The worker injects a cron-specific source section into Consoles that support
`host.triggerRenderers`. Trigger registration, firing, and retirement show a
plain-language schedule, the exact expression, explicit UTC, and the optional
`condition_function_id`. Expressions that cannot be summarized without
hiding cron semantics keep an honest “custom schedule” label and the raw
expression.

The Console retains the surrounding activity, delivery target/result,
lifecycle state and controls, and raw JSON. Disabling or disconnecting this
worker's injected UI therefore falls back to the generic trigger view rather
than removing trigger activity.

For local UI development:

```bash
pnpm --dir cron/ui test
pnpm --dir cron/ui build
# terminal 1
pnpm --dir cron/ui watch
# terminal 2
cd cron && III_CRON_UI_WATCH=1 cargo run
```

The Rust build embeds `cron/page.js` and `cron/styles.css`; production does
not require a separate asset server.

### Requires removing the legacy built-in cron worker

The legacy built-in cron worker also owns the `cron` trigger type. Two owners
of the same trigger type on one engine collide - whichever registers last
wins - so this worker requires it to be absent: omit it from the
engine's `config.yaml` (a config that doesn't list a worker won't run it).

On boot, this worker queries the engine for connected workers and refuses to
start with a clear error if the legacy built-in is still active, so a stale config
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

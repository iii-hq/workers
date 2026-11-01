# cron

Runs functions on cron schedules. A function runs on a schedule when it
observes `cron::on-tick` with a cron expression as args: this worker calls
`cron::on-tick` once per fire of that expression, at the next matching UTC
time, and every observer of the expression receives the tick.

## Install

```bash
iii call compose::add worker=cron
```

`iii call compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

## Quickstart

Register a function and observe `cron::on-tick` with it, passing the schedule
as args:

```typescript
iii.registerFunction('reports::daily', async (tick: CronTick) => {
  // tick: { expression, scheduled_time, actual_time }
  await sendDigest(tick.scheduled_time)
})

const digest = iii.observe(
  { function_id: 'cron::on-tick', args: { expression: '0 0 9 * * Mon-Fri' }, deliver: 'output' },
  'reports::daily',
)
await digest.ready()   // rejected with INVALID_CRON_EXPRESSION for a bad expression
// later: await digest.dispose() stops the schedule
```

The same in Rust:

```rust
use iii_sdk::{errors::Error, register_worker, Deliver, InitOptions, RegisterFunction, Source};
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    iii.register_function(
        "jobs::tick",
        RegisterFunction::new_async(|tick: Value| async move {
            println!("cron fired: {tick}");
            Ok::<Value, Error>(json!({ "ok": true }))
        }),
    );

    iii.observe(
        Source::function("cron::on-tick")
            .args(json!({ "expression": "*/5 * * * * *" }))
            .deliver(Deliver::Output),
        "jobs::tick",
    );

    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

With `deliver: 'output'` the function receives the tick:

```json
{
  "expression": "*/5 * * * * *",
  "scheduled_time": "2026-07-03T12:00:00+00:00",
  "actual_time": "2026-07-03T12:00:00.123456789+00:00"
}
```

Without it, the function receives the `Observed` envelope, with the tick as
its `input` and `output` and the observer's id as `observer_id`.

## Functions

| Function | Args | What it is |
|---|---|---|
| `cron::on-tick` | `{ expression }` | One call per fire of a cron expression. Observers observe the tick; each observer receives the ticks of its own expression. |

### `cron::on-tick`

| Arg | Required | Description |
|---|---|---|
| `expression` | yes | Cron expression parsed by the Rust `cron` crate: six fields `sec min hour day month weekday`, and an optional seventh year field. UTC. |

Rejections, through the observer's `ready()`:

| Code | When |
|---|---|
| `INVALID_ARGS` | `expression` is missing or not a string, or another arg is present. |
| `INVALID_CRON_EXPRESSION` | `expression` is empty or does not parse; the message names the expression. |

The event, the function's input and what its observers observe:

| Field | Description |
|---|---|
| `expression` | The expression that fired, exactly as its observers passed it in args. |
| `scheduled_time` | The fire's scheduled time: RFC 3339 in UTC with a `+00:00` offset, whole seconds (`2026-11-01T09:00:00+00:00`). |
| `actual_time` | When the worker fired it: RFC 3339 in UTC with a `+00:00` offset. |

- **Delivery.** The worker runs one job per distinct expression and calls
  `cron::on-tick` once per fire. iii delivers the tick to the observers of
  that expression (matched by `args.expression` and `input.expression`) whose
  filters accept it. The job starts with the expression's first observer and
  stops with its last.
- **Filters** select ticks by `input`, e.g.
  `{ 'input.scheduled_time': { $gte: '2026-11-01T00:00:00+00:00' } }`.
  Filters compare strings by code point, so a time bound is written in the
  form the worker emits: RFC 3339 with `+00:00`.
- **Metadata.** The observer's `metadata` arrives as each delivery's sidecar.
- **Missed fires.** Fires while the worker is stopped, or disconnected from
  the engine, are skipped; there is no catch-up replay.
- **Day of week.** Write it as a name (`Mon` ... `Sun`). Numerically the crate
  counts Sunday as 1, so `0 0 9 * * 1` fires on Sunday, not Monday.
- **Private.** Only this worker calls `cron::on-tick`; a call from another
  worker is refused with `FORBIDDEN`, and the function cannot be an
  observer's target.
- **One host.** The function registers as a standby: a second instance of the
  worker waits, and takes over the observers when the first one leaves. Each
  expression fires once per scheduled time. The engine admits one live worker
  per worker name and namespace, so the second instance runs under its own
  name (`III_WORKER_NAME=cron-standby`). An instance started under a name that
  a live worker holds is refused (`WORKER_NAMESPACE_CONFLICT`) and exits.

A target that needs to know which schedule fired reads `observer_id` from the
envelope, or observes with its own target per schedule.

## Configuration

The worker has no settings. The engine passes a worker's `config:` block as
`--config <file>`; this worker does not read it.

## Console page

While the worker is connected it injects a **cron** page into the console
(page `cron`): every agent-owned schedule with its cadence, next UTC run and
fire count, the `cron::on-tick` observers other workers created for their own
functions, and a composer that turns "every weekday at 09:00, summarise open
PRs" into a schedule. The composer and the manual form ask a harness agent to
call `harness::observe` with `{ function_id: 'cron::on-tick', args: {
expression } }`; schedules created there live in a session of their own, so
each routine keeps its own transcript.

## Console observer activity

The worker injects a cron-specific source section into Consoles that support
`host.observerRenderers`. The activity of a `cron::on-tick` observer (added,
fired, retired) shows a plain-language schedule, the exact expression, and
explicit UTC. Expressions that cannot be summarized without hiding cron
semantics keep an honest “custom schedule” label and the raw expression.

The Console retains the surrounding activity, delivery target/result,
lifecycle state and controls, and raw JSON. Disabling or disconnecting this
worker's injected UI falls back to the generic observer view.

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

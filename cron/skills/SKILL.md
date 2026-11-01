---
name: cron
tags: cron, scheduling, observers, jobs
description: >-
  Run any registered function on a 6- or 7-field cron expression with the
  standalone cron worker. A function runs on a schedule by observing
  `cron::on-tick` with `{ expression }` as args.
---

# cron

The `cron` worker runs registered functions on recurring cron expressions. A
schedule is an observer of `cron::on-tick`: the args hold the expression, and
the target is the function that runs on it:
`iii.observe({ function_id: 'cron::on-tick', args: { expression }, deliver: 'output' }, target)`.

Install it with `iii call compose::add worker=cron`.

The worker runs one job per distinct expression and calls `cron::on-tick`
once per fire, with a tick holding `expression`, `scheduled_time`, and
`actual_time`. Every observer of that expression receives the tick.
`scheduled_time` and `actual_time` let handlers see drift without querying
the scheduler.

The schedule grammar is the Rust `cron` crate dialect: six or seven fields,
`second minute hour day-of-month month day-of-week [year]`. The year field is
optional. The leading field is always seconds, so `0 */5 * * * *` fires every
5 minutes at second 0, while `*/5 * * * * *` fires every 5 seconds.

Write the day of week as a name: `Mon`, `Tue`, `Wed`, `Thu`, `Fri`, `Sat`,
`Sun`. Numerically this dialect counts Sunday as 1, one ahead of the Unix
convention, so `0 0 9 * * 1` fires on Sunday and a `1` written for Monday
fires a day early. Names avoid the whole question.

## When to Use

- A function should run periodically without a separate scheduler process or
  system crontab entry.
- You need recurring cleanup, reporting, maintenance, or batch jobs that should
  fire on UTC cron windows.
- One function should run on several schedules: observe `cron::on-tick` once
  per expression with the same target.
- A scheduled job should skip some fires: add `filters` on the tick's `input`
  to the observer.

## Boundaries

- Never call `cron::on-tick`: it is private to the cron worker, and a call from
  another worker is refused with `FORBIDDEN`. Observe it with one of your own
  functions as the target.
- The worker does not catch up missed runs after downtime. It schedules the
  next upcoming UTC fire from the time it is running.
- Five-field crontab expressions are refused (`INVALID_CRON_EXPRESSION`).
  Include the seconds field.
- `cron::on-tick` has one host per namespace. A second instance of the worker
  waits as a standby and takes over the schedules when the first one leaves,
  so each expression fires once per scheduled time.
- For data-change or stream-change reactions, use the relevant state, stream,
  queue, or database worker instead; `cron` fires on the clock only.

## Running on a schedule

Observe `cron::on-tick` when a handler should run on a recurring schedule. The
handler runs as a normal function invocation for each fire.

Reach for it when:

- You need recurring execution such as cleanup, reports, or maintenance.
- A schedule should apply only to some fires: filter on the tick, e.g.
  `{ 'input.scheduled_time': { $gte: '2026-11-01T00:00:00+00:00' } }`.
  Filters compare strings by code point: write time bounds as the worker
  emits them, RFC 3339 with `+00:00`.
- One function should be driven by several schedules; observe once per
  expression and branch on the tick's `expression`.

### How to observe

1. Register a handler: `iii.registerFunction('jobs::cleanup-old-data', handler)`.
2. Observe `cron::on-tick` with the expression as args:

```typescript
iii.registerFunction('jobs::cleanup-old-data', async (tick: CronTick) => {
  // tick: { expression, scheduled_time, actual_time }
})

const cleanup = iii.observe(
  {
    function_id: 'cron::on-tick',
    args: { expression: '0 0 2 * * *' },  // sec min hour dom month dow [year]; daily at 02:00:00 UTC.
    deliver: 'output',
  },
  'jobs::cleanup-old-data',
)
await cleanup.ready()
// later: await cleanup.dispose() stops the schedule
```

`args` takes `expression` only; anything else fails `ready()` with
`INVALID_ARGS`, and an expression that does not parse fails it with
`INVALID_CRON_EXPRESSION`. The handler's return value is ignored.

With `deliver: 'output'` the handler receives the tick:

```json
{
  "expression": "0 0 2 * * *",
  "scheduled_time": "2026-07-03T02:00:00+00:00",
  "actual_time": "2026-07-03T02:00:00.123456789+00:00"
}
```

Without it, the handler receives the `Observed` envelope: the tick is its
`input` and `output`, and `observer_id` names the observer that fired. The
observer's `metadata` arrives as each delivery's metadata sidecar.

An agent schedules through the harness with `harness::observe`:

```json
{ "function_id": "cron::on-tick", "args": { "expression": "0 30 9 * * Mon" }, "label": "Monday standup notes", "once": false }
```

and retires a schedule with `harness::unobserve` and its `observer_id`.

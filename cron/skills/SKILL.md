---
type: index
name: cron
tags: cron, scheduling, triggers, jobs
description: >-
  Schedule any registered function on a 6- or 7-field cron expression with the
  standalone cron worker. Its whole surface is the `cron` trigger type.
---

# cron

The `cron` worker schedules registered functions to run on recurring cron
expressions. It exposes no callable `cron::*` functions; its entire surface is
the `cron` trigger type, bound through a worker SDK trigger registration such
as `iii.registerTrigger({ type: 'cron', function_id, config })`.

Install it with `iii worker add cron`. The engine builtin `iii-cron` must not
run on the same engine because it also owns the `cron` trigger type. Remove
`iii-cron` from the engine config before starting this worker; the standalone
worker refuses to boot when the builtin is active.

On every firing, the worker optionally evaluates a condition function, acquires
a lock through the configured backend, and invokes the target function with an
event payload containing `trigger`, `job_id`, `scheduled_time`, and
`actual_time`. `scheduled_time` and `actual_time` let handlers observe drift
without querying the scheduler.

The schedule grammar is the Rust `cron` crate dialect: six or seven fields,
`second minute hour day-of-month month day-of-week [year]`. The year field is
optional. The leading field is always seconds, so `0 */5 * * * *` fires every
5 minutes at second 0, while `*/5 * * * * *` fires every 5 seconds.

Two lock backends govern duplicate firing. `local` is the default and is only
process-local; every worker instance can fire the same job in a multi-instance
deployment. `redis` uses Redis locking and is required for once-only firing
across a fleet. The lock TTL is 30 seconds.

## When to Use

- A function should run periodically without a separate scheduler process or
  system crontab entry.
- You need recurring cleanup, reporting, maintenance, or batch jobs that should
  fire on UTC cron windows.
- You need once-only firing across a fleet and can configure the `redis` lock
  backend.
- A scheduled job should be conditionally skipped by setting
  `condition_function_id`, without putting the condition inside the handler.

## Boundaries

- No callable functions are exposed. Never invoke `cron::*`; bind the `cron`
  trigger type to one of your own functions.
- The default `local` backend is process-local. Do not rely on it for once-only
  jobs in a multi-instance deployment.
- The worker does not catch up missed runs after downtime. It schedules the
  next upcoming UTC fire from the time it is running.
- A condition function only blocks when it returns explicit JSON `false`.
  Truthy, null, or missing results allow the fire; condition errors skip that
  fire.
- Five-field crontab expressions are not the intended format. Include the
  seconds field.
- For data-change or stream-change reactions, use the relevant state, stream,
  queue, or event worker instead; `cron` fires on the clock only.

## Reactive triggers

Bind a `cron` trigger when a handler should run on a recurring schedule. The
handler runs through the engine as a normal function invocation. With the
`redis` backend, only one worker instance should own a scheduled fire across
the fleet.

Reach for it when:

- You need recurring execution such as cleanup, reports, or maintenance.
- You want condition-gated firing: set `condition_function_id` and the worker
  evaluates it before each run.
- One function should be driven by multiple schedules; register multiple
  triggers and branch on the `job_id` payload field.

### How to bind

1. Register a handler: `iii.registerFunction('jobs::cleanup-old-data', handler)`.
2. Register the trigger:

```typescript
iii.registerTrigger({
  type: 'cron',
  function_id: 'jobs::cleanup-old-data',
  config: {
    expression: '0 0 2 * * * *',  // sec min hour dom month dow [year]; daily at 02:00:00 UTC.
    // condition_function_id is optional.
  },
})
```

`expression` is required and must parse. Bind one `function_id` to several
triggers with distinct trigger ids to drive multiple schedules into one
handler; the trigger id arrives as `job_id` in the event. The handler's return
value is ignored.

Event payload:

```json
{
  "trigger": "cron",
  "job_id": "<trigger-id>",
  "scheduled_time": "2026-07-03T12:00:00+00:00",
  "actual_time": "2026-07-03T12:00:00.123456789+00:00"
}
```

Configure the lock backend through the central configuration entry for worker
`cron`:

```yaml
adapter:
  name: redis
  config:
    redis_url: redis://localhost:6379
```

Use `adapter.name: local` for single-process development and
`adapter.name: redis` for multi-instance once-only scheduling.

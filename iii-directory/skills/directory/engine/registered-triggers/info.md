---
type: how-to
function_id: directory::engine::registered-triggers::info
title: Inspect one registered trigger (instance + type + function)
---

# When to use

Call `directory::engine::registered-triggers::info` when you have a
registered trigger id (from
`directory::engine::registered-triggers::list`) and want EVERYTHING it
links together in a single payload: the per-instance config + the full
trigger-type detail (schemas, instance count) + the full function
detail (schemas, owning worker, how-to).

It denormalizes three lookups into one composite call so the agent
doesn't need to fan out three follow-ups to understand a single
subscription.

# Inputs

```json
{ "id": "trg-mem-compact" }
```

`id` is the registered-trigger instance id (the unique row id, not the
trigger type).

# Outputs

```json
{
  "id":           "trg-mem-compact",
  "trigger_type": "directory::skills::on-change",
  "function_id":  "agent-memory::compact",
  "worker_name":  "agent-memory",
  "config":       { "interval_ms": 1000 },
  "metadata":     null,
  "trigger":      { /* same shape as directory::engine::triggers::info */ },
  "function":     { /* same shape as directory::engine::functions::info, including how_guide ({title, skill_id, body}) and related_skills */ }
}
```

`trigger` or `function` come back as `null` only if the type or target
was unregistered between the time the instance was created and when
you call this — usually both are populated.

# Worked example

```json
{ "id": "trg-mem-compact" }
```

Returns the subscriber row, the schemas for
`directory::skills::on-change`, the schemas for
`agent-memory::compact`, and the bundled how-to for
`agent-memory::compact` (if any) all in one payload.

# Related

- `directory::engine::registered-triggers::list` — find the instance id
  you want to inspect.
- `directory::engine::triggers::info` — for just the trigger TYPE detail.
- `directory::engine::functions::info` — for just the function detail.

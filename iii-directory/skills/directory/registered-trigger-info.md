---
type: how-to
function_id: directory::registered-trigger-info
title: Inspect one registered trigger (instance + type + function)
---

# When to use

Call `directory::registered-trigger-info` when you have a registered
trigger id (from `directory::registered-trigger-list`) and want
EVERYTHING it links together in a single payload: the per-instance
config + the full trigger-type detail (schemas, instance count) + the
full function detail (schemas, owning worker, how-to).

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
  "trigger_type": "mem::on-change",
  "function_id":  "agentmemory::compact",
  "worker_name":  "agentmemory",
  "config":       { "interval_ms": 1000 },
  "metadata":     null,
  "trigger":      { /* same shape as directory::trigger-info */ },
  "function":     { /* same shape as directory::function-info, including how_guide */ }
}
```

`trigger` or `function` come back as `null` only if the type or target
was unregistered between the time the instance was created and when
you call this — usually both are populated.

# Worked example

```json
{ "id": "trg-mem-compact" }
```

Returns the subscriber row, the schemas for `mem::on-change`, the
schemas for `agentmemory::compact`, and the bundled how-to for
`agentmemory::compact` (if any) all in one payload.

# Related

- `directory::registered-trigger-list` — find the instance id you
  want to inspect.
- `directory::trigger-info` — for just the trigger TYPE detail.
- `directory::function-info` — for just the function detail.

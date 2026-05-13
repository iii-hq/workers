---
type: how-to
function_id: directory::registered-trigger-list
title: List registered trigger instances (subscriber rows)
---

# When to use

Use `directory::registered-trigger-list` to enumerate the SUBSCRIBER
rows — each one is a link between a trigger TYPE (template) and a
target function, plus per-instance configuration.

This is the right call when you want to answer:

- "Who's listening to `mem::on-change` right now?"
- "What triggers fire `agentmemory::compact`?"
- "Which subscribers does the `scheduler` worker own?"

For trigger TYPES (templates) instead, use `directory::trigger-list`.

# Inputs

```json
{
  "search":       "...",                // optional, case-insensitive substring vs id + trigger_type + function_id
  "trigger_type": "mem::on-change",     // optional, exact match
  "function_id":  "agentmemory::compact", // optional, exact match
  "worker":       "scheduler"           // optional, exact worker-name match (worker that owns the function)
}
```

All filters are optional and combinable.

# Outputs

```json
{
  "registered_triggers": [
    {
      "id":             "trg-mem-compact",
      "trigger_type":   "mem::on-change",
      "function_id":    "agentmemory::compact",
      "worker_name":    "agentmemory",
      "config_summary": "{\"interval_ms\":1000}"   // truncated to ~80 chars; use registered-trigger-info for full
    }
  ]
}
```

Rows are sorted lexicographically by `id`.

# Worked example

Show every subscriber pointing at the `mem::on-change` trigger:

```json
{ "trigger_type": "mem::on-change" }
```

Show every subscriber owned by the `agentmemory` worker:

```json
{ "worker": "agentmemory" }
```

# Related

- `directory::registered-trigger-info` — full config + denormalized
  trigger detail + function detail for one subscriber row.
- `directory::trigger-list` — list trigger TYPES instead of instances.
- `directory::function-info.registered_triggers` — same data scoped to
  a single target function.

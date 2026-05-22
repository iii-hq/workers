---
type: how-to
function_id: directory::engine::registered-triggers::list
title: List registered trigger instances (subscriber rows)
---

> **Function id:** `directory::engine::registered-triggers::list` — pass this to `agent_trigger { function: "directory::engine::registered-triggers::list" }` (NOT the skill path you saw in `directory::skills::list`; that's a documentation id, not a callable function id).

# When to use

Use `directory::engine::registered-triggers::list` to enumerate the
SUBSCRIBER rows — each one is a link between a trigger TYPE (template)
and a target function, plus per-instance configuration.

This is the right call when you want to answer:

- "Who's listening to `directory::skills::on-change` right now?"
- "What triggers fire `agent-memory::compact`?"
- "Which subscribers does the `scheduler` worker own?"

For trigger TYPES (templates) instead, use
`directory::engine::triggers::list`.

# Inputs

```json
{
  "search":       "...",                         // optional, case-insensitive substring vs id + trigger_type + function_id
  "trigger_type": "directory::skills::on-change", // optional, exact match
  "function_id":  "agent-memory::compact",       // optional, exact match
  "worker":       "scheduler"                    // optional, exact worker-name match (worker that owns the function)
}
```

All filters are optional and combinable.

# Outputs

```json
{
  "registered_triggers": [
    {
      "id":             "trg-mem-compact",
      "trigger_type":   "directory::skills::on-change",
      "function_id":    "agent-memory::compact",
      "worker_name":    "agent-memory",
      "config_summary": "{\"interval_ms\":1000}"   // truncated to ~80 chars; use registered-triggers::info for full
    }
  ]
}
```

Rows are sorted lexicographically by `id`.

# Worked example

Show every subscriber pointing at the `directory::skills::on-change`
trigger:

```json
{ "trigger_type": "directory::skills::on-change" }
```

Show every subscriber owned by the `agent-memory` worker:

```json
{ "worker": "agent-memory" }
```

# Related

- `directory::engine::registered-triggers::info` — full config +
  denormalized trigger detail + function detail for one subscriber row.
- `directory::engine::triggers::list` — list trigger TYPES instead of
  instances.
- `directory::engine::functions::info` `.registered_triggers` — same
  data scoped to a single target function.

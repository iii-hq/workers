---
type: how-to
function_id: directory::worker-info
title: Inspect one connected worker's full surface
---

# When to use

Call `directory::worker-info` to see everything one connected worker
exposes: the worker envelope (same shape as `worker-list` rows) plus
the full lists of functions, trigger types, and registered triggers it
owns.

Use it after `directory::worker-list` when you want to drill into a
specific worker's surface.

This is the LOCAL view. For the published metadata of a worker (readme,
api_reference, version history), use `registry::worker-info` — same
top-level `worker` envelope so callers learn one parser.

# Inputs

```json
{ "name": "agentmemory" }
```

`name` is the worker's registered name (NOT its connection id).

# Outputs

```json
{
  "worker": {
    "name":              "agentmemory",   // shared core fields with worker-list rows + registry::worker-info.worker
    "description":       null,            // engine carries no description; always null here
    "version":           "0.4.0",
    "id":                "w-abc123",
    "runtime":           "rust",
    "os":                "darwin",
    "status":            "connected",
    "function_count":    9,
    "connected_at_ms":   1715520000000,
    "active_invocations": 0,
    "isolation":         null,
    "ip_address":        null
  },
  "functions": [
    { "function_id": "mem::observe", "name": "observe", "description": "Record an event." }
  ],
  "trigger_types": [
    { "id": "mem::on-change", "name": "on-change", "description": "Fires when memory changes." }
  ],
  "registered_triggers": [
    { "id": "trg-mem-compact", "trigger_type": "mem::on-change", "function_id": "agentmemory::compact" }
  ]
}
```

The top-level `worker` field has the same shape as
`registry::worker-info.worker` so a parser written for one surface
works on the other.

# Worked example

```json
{ "name": "iii-directory" }
```

Returns this worker itself: 13 functions across `skills::*`,
`prompts::*`, `directory::*`, `registry::*`, plus the `skills::on-change`
and `prompts::on-change` trigger types.

# Related

- `directory::worker-list` — discover the name you want to inspect.
- `registry::worker-info` — same `worker` envelope against the public
  registry, with `readme` / `api_reference` / `skills_tree` extras.
- `directory::function-info` — single-function detail (with how-to).

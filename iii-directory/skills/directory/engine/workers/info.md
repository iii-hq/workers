---
type: how-to
function_id: directory::engine::workers::info
title: Inspect one connected worker's full surface
---

# When to use

Call `directory::engine::workers::info` to see everything one connected
worker exposes: the worker envelope (same shape as `workers::list`
rows) plus the full lists of functions, trigger types, and registered
triggers it owns.

Use it after `directory::engine::workers::list` when you want to drill
into a specific worker's surface.

This is the LOCAL view. For the published metadata of a worker (readme,
api_reference, version history), use `directory::registry::workers::info`
— the top-level `worker` envelope shares a fixed set of core fields
(`name`, `description`, `version`) across both surfaces; everything else
(connection state here, registry metadata there) is surface-specific
and should be treated as optional by clients.

# Inputs

```json
{ "name": "agent-memory" }
```

`name` is the worker's registered name (NOT its connection id).

# Outputs

```json
{
  "worker": {
    "name":              "agent-memory",  // shared core fields with workers::list rows + directory::registry::workers::info.worker
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
    { "function_id": "agent-memory::observe", "description": "Record an event." }
  ],
  "trigger_types": [
    { "id": "agent-memory::on-change", "description": "Fires when memory changes." }
  ],
  "registered_triggers": [
    { "id": "trg-mem-compact", "trigger_type": "agent-memory::on-change", "function_id": "agent-memory::compact" }
  ]
}
```

The top-level `worker` field shares its core fields (`name`,
`description`, `version`) with
`directory::registry::workers::info.worker`, so a parser that touches
only those keys works on both surfaces. The remaining fields shown
above are LOCAL-specific runtime state and may not appear in the
registry envelope.

# Worked example

```json
{ "name": "iii-directory" }
```

Returns this worker itself: 15 functions across `directory::skills::*`,
`directory::prompts::*`, `directory::engine::*`, and
`directory::registry::*`, plus the `directory::skills::on-change` and
`directory::prompts::on-change` trigger types.

# Related

- `directory::engine::workers::list` — discover the name you want to
  inspect.
- `directory::registry::workers::info` — same `worker` envelope against
  the public registry, with `readme` / `api_reference` / `skills_tree`
  extras.
- `directory::engine::functions::info` — single-function detail (with
  how-to).

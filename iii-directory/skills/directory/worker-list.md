---
type: how-to
function_id: directory::worker-list
title: List workers connected to the engine
---

# When to use

Use `directory::worker-list` to enumerate every worker currently
connected to the engine, with its runtime metadata (status, version,
runtime, function count, ...). Filter by name, runtime, or status.

This is the LOCAL view. For the registry view (workers PUBLISHED, not
connected), use `registry::worker-list` — rows share a fixed set of
core fields (`name`, `description`, `version`) so a parser can walk
both surfaces. Each surface adds its own optional fields beyond that.

# Inputs

```json
{
  "search":  "agent",     // optional, case-insensitive substring vs name
  "runtime": "rust",      // optional, exact runtime match (e.g. "rust", "node")
  "status":  "connected"  // optional, exact status match (e.g. "connected", "disconnected")
}
```

# Outputs

```json
{
  "workers": [
    {
      "name":              "agentmemory",  // shared core field with registry::worker-list
      "description":       null,           // shared core field; engine carries no description, always null here
      "version":           "0.4.0",        // shared core field
      "id":                "w-abc123",     // engine-assigned connection id (directory-specific)
      "runtime":           "rust",
      "os":                "darwin",
      "status":            "connected",
      "function_count":    9,
      "connected_at_ms":   1715520000000,
      "active_invocations": 0,
      "isolation":         null,
      "ip_address":        null
    }
  ]
}
```

Rows are sorted lexicographically by `name`.

The first three fields (`name`, `description`, `version`) are SHARED
with `registry::worker-list` rows so callers can write one parser that
handles both surfaces. Everything else is directory-specific
runtime-state.

# Worked example

Show only connected Rust workers:

```json
{ "runtime": "rust", "status": "connected" }
```

# Related

- `directory::worker-info` — single-worker detail with its full
  function/trigger surface.
- `registry::worker-list` — same row shape against the public
  registry.
- `directory::function-list` — function-side view across all workers.

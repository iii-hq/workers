---
type: how-to
function_id: directory::function-list
title: List functions registered with the engine
---

# When to use

Reach for `directory::function-list` when you need to discover what's
callable on the engine right now. It returns one row per registered
function with the bare-minimum metadata needed to decide whether to
follow up with `directory::function-info`.

Common situations:

- An agent is exploring "what can I do here?" and wants to scope down
  by namespace or worker.
- You suspect a worker is missing or disconnected — list functions and
  check which `worker_name`s show up.
- You want to enumerate every function in a namespace before drilling
  into schemas.

# Inputs

```json
{
  "search":  "...",   // optional, case-insensitive substring vs function_id + description
  "prefix":  "mem::", // optional, exact prefix match on function_id
  "worker":  "..."    // optional, exact worker-name match
}
```

All filters are optional and combinable. Empty input returns every
function the engine is exposing right now.

# Outputs

```json
{
  "functions": [
    {
      "function_id": "mem::observe",
      "name": "observe",                  // last :: segment
      "worker_name": "agentmemory",       // resolved owner; falls back to first :: segment
      "description": "Record an event."   // optional
    }
  ]
}
```

Rows are sorted lexicographically by `function_id`.

# Worked example

Find every function the `agentmemory` worker exposes:

```json
{ "worker": "agentmemory" }
```

Find every `directory::*` function that mentions "trigger" in its
description:

```json
{ "prefix": "directory::", "search": "trigger" }
```

# Related

- `directory::function-info` — schemas + how-to for one function.
- `directory::worker-list` — discover which workers are connected.
- `directory::worker-info` — show the function set owned by one worker.
- `registry::worker-list` — same shape against the public registry.

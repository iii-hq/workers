---
type: how-to
function_id: directory::engine::functions::list
title: List functions registered with the engine
---

# When to use

Reach for `directory::engine::functions::list` when you need to
discover what's callable on the engine right now. It returns one row
per registered function with the bare-minimum metadata needed to decide
whether to follow up with `directory::engine::functions::info`.

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
  "search":  "...",                 // optional, case-insensitive substring vs function_id + description
  "prefix":  "directory::engine::", // optional, exact prefix match on function_id
  "worker":  "..."                  // optional, exact worker-name match
}
```

All filters are optional and combinable. Empty input returns every
function the engine is exposing right now.

# Outputs

```json
{
  "functions": [
    {
      "function_id": "directory::engine::functions::info",
      "worker_name": "directory",          // resolved owner; falls back to first :: segment of function_id
      "description": "Full detail for ..." // optional
    }
  ]
}
```

Rows are sorted lexicographically by `function_id`.

# Worked example

Find every function the `directory` worker exposes:

```json
{ "worker": "directory" }
```

Find every `directory::engine::*` function that mentions "trigger" in
its description:

```json
{ "prefix": "directory::engine::", "search": "trigger" }
```

# Related

- `directory::engine::functions::info` — schemas + how-to for one function.
- `directory::engine::workers::list` — discover which workers are connected.
- `directory::engine::workers::info` — show the function set owned by one worker.
- `directory::registry::workers::list` — same shape against the public registry.

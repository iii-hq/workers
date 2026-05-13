---
type: how-to
function_id: directory::trigger-list
title: List trigger types registered with the engine
---

# When to use

Use `directory::trigger-list` to enumerate trigger TYPES — the
templates that workers register and which other workers can subscribe
to. This is the catalog of "what events does the engine know how to
fan out?"

If you want the actual subscription rows (the link between a trigger
type and a target function), reach for
`directory::registered-trigger-list` instead.

# Inputs

```json
{
  "search":  "...",      // optional, case-insensitive substring vs id + description
  "prefix":  "mem::",    // optional, exact prefix match on the trigger-type id
  "worker":  "..."       // optional, first :: segment of the id (best-signal owner)
}
```

# Outputs

```json
{
  "triggers": [
    {
      "id":          "mem::on-change",
      "name":        "on-change",                 // last :: segment
      "worker_name": "mem",                       // first :: segment
      "description": "Fires when memory changes."
    }
  ]
}
```

Rows are sorted lexicographically by `id`.

# Worked example

Find every trigger type the `mem` worker publishes:

```json
{ "worker": "mem" }
```

Find every `*::on-change` trigger across all workers:

```json
{ "search": "on-change" }
```

# Related

- `directory::trigger-info` — schemas + instance count for one type.
- `directory::registered-trigger-list` — listing of who's subscribed
  to which trigger type.
- `directory::function-list` — for the call surface, not the event
  surface.

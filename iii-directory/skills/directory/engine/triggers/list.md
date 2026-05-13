---
type: how-to
function_id: directory::engine::triggers::list
title: List trigger types registered with the engine
---

# When to use

Use `directory::engine::triggers::list` to enumerate trigger TYPES —
the templates that workers register and which other workers can
subscribe to. This is the catalog of "what events does the engine know
how to fan out?"

If you want the actual subscription rows (the link between a trigger
type and a target function), reach for
`directory::engine::registered-triggers::list` instead.

# Inputs

```json
{
  "search":  "...",                 // optional, case-insensitive substring vs id + description
  "prefix":  "directory::skills::", // optional, exact prefix match on the trigger-type id
  "worker":  "..."                  // optional, first :: segment of the id (best-signal owner)
}
```

# Outputs

```json
{
  "triggers": [
    {
      "id":          "directory::skills::on-change",
      "worker_name": "directory",                 // first :: segment of id
      "description": "Fires when skills change."
    }
  ]
}
```

Rows are sorted lexicographically by `id`.

# Worked example

Find every trigger type the `directory` worker publishes:

```json
{ "worker": "directory" }
```

Find every `*::on-change` trigger across all workers:

```json
{ "search": "on-change" }
```

# Related

- `directory::engine::triggers::info` — schemas + instance count for one type.
- `directory::engine::registered-triggers::list` — listing of who's
  subscribed to which trigger type.
- `directory::engine::functions::list` — for the call surface, not the
  event surface.

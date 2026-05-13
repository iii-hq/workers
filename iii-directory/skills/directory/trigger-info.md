---
type: how-to
function_id: directory::trigger-info
title: Inspect one trigger type's schemas + live instance count
---

# When to use

Call `directory::trigger-info` once you've identified a trigger TYPE id
(e.g. `mem::on-change`) and you want its configuration schema, return
schema, the worker that registered it, and a live count of how many
instances are currently subscribed to it.

Useful before subscribing a new function to a trigger so the agent
crafts a valid configuration block.

# Inputs

```json
{ "id": "mem::on-change" }
```

`id` is the full trigger-type identifier (`{worker}::{name}`).

# Outputs

```json
{
  "id":                   "mem::on-change",
  "name":                 "on-change",                 // last :: segment
  "worker_name":          "mem",                       // first :: segment
  "description":          "Fires when memory changes.",
  "configuration_schema": { "type": "object", ... },   // shape passed when registering an instance
  "return_schema":        { "type": "object", ... },   // shape received by the target function
  "instance_count":       3                            // how many registered_triggers point at this type right now
}
```

# Worked example

```json
{ "id": "skills::on-change" }
```

Returns the trigger schema this worker (`iii-directory`) publishes plus
the current subscriber count.

# Related

- `directory::trigger-list` — find the trigger type id you want to
  inspect.
- `directory::registered-trigger-list` — list the actual subscriber
  rows for this type.
- `directory::registered-trigger-info` — composite view of one
  subscriber row + its type + its target function.

---
type: how-to
function_id: directory::engine::triggers::info
title: Inspect one trigger type's schemas + live instance count
---

> **Function id:** `directory::engine::triggers::info` — pass this to `agent_trigger { function: "directory::engine::triggers::info" }` (NOT the skill path you saw in `directory::skills::list`; that's a documentation id, not a callable function id).

# When to use

Call `directory::engine::triggers::info` once you've identified a
trigger TYPE id (e.g. `directory::skills::on-change`) and you want its
configuration schema, return schema, the worker that registered it, and
a live count of how many instances are currently subscribed to it.

Useful before subscribing a new function to a trigger so the agent
crafts a valid configuration block.

# Inputs

```json
{ "id": "directory::skills::on-change" }
```

`id` is the full trigger-type identifier (`{worker}::{...}`).

# Outputs

```json
{
  "id":                   "directory::skills::on-change",
  "worker_name":          "directory",                  // first :: segment of id
  "description":          "Fires when skills change.",
  "configuration_schema": { "type": "object", ... },    // shape passed when registering an instance
  "return_schema":        { "type": "object", ... },    // shape received by the target function
  "instance_count":       3                             // how many registered_triggers point at this type right now
}
```

# Worked example

```json
{ "id": "directory::skills::on-change" }
```

Returns the trigger schema this worker (`iii-directory`) publishes plus
the current subscriber count.

# Related

- `directory::engine::triggers::list` — find the trigger type id you
  want to inspect.
- `directory::engine::registered-triggers::list` — list the actual
  subscriber rows for this type.
- `directory::engine::registered-triggers::info` — composite view of
  one subscriber row + its type + its target function.

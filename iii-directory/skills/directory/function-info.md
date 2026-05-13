---
type: how-to
function_id: directory::function-info
title: Inspect one function's schemas, owner, and how-to skill
---

# When to use

Call `directory::function-info` once you've identified a function id
(via `directory::function-list` or otherwise) and you want everything
the engine knows about it: input/output JSON Schemas, owning worker,
the registered trigger instances pointing at it, and any matching
how-to skill from `skills_folder`.

Use it before invoking an unfamiliar function so the agent can craft a
correct payload.

# Inputs

```json
{ "function_id": "mem::observe" }
```

`function_id` is required. Anything else (search, paging) is delegated
to `directory::function-list`.

# Outputs

```json
{
  "function_id":     "mem::observe",
  "name":            "observe",
  "worker_name":     "agentmemory",
  "description":     "Record an event in agent memory.",
  "request_schema":  { "type": "object", "properties": { ... } },
  "response_schema": { "type": "object", "properties": { ... } },
  "metadata":        null,
  "registered_triggers": [
    { "id": "trg-1", "trigger_type": "scheduler::tick", "config": { ... } }
  ],
  "how_guide": {
    "skill_id":   "agentmemory/observe",
    "abs_path":   "/.../skills/agentmemory/observe.md",
    "frontmatter": { "type": "how-to", "functions": ["mem::observe"], "title": "..." },
    "body":       "# How to observe ..."
  }
}
```

`how_guide` is `null` when no markdown in `skills_folder` carries
`type: how-to` plus a matching `function_id` (or `functions: [...]`
array, or a body link to `iii://fn/<dotted/path>`).

# Worked example

```json
{ "function_id": "directory::worker-list" }
```

Returns the input/output schemas for `worker-list`, attributes it to
the `iii-directory` worker, and surfaces this very skill (you're
reading it via the `how_guide` field) when called against a function
that has a bundled how-to.

# Related

- `directory::function-list` — find the id you want to inspect.
- `directory::worker-info` — group by worker instead of function.
- `directory::registered-trigger-info` — look up a trigger that calls
  this function.

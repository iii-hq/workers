---
type: how-to
function_id: directory::engine::functions::info
title: Inspect one function's schemas, owner, and how-to skill
---

# When to use

Call `directory::engine::functions::info` once you've identified a
function id (via `directory::engine::functions::list` or otherwise) and
you want everything the engine knows about it: input/output JSON
Schemas, owning worker, the registered trigger instances pointing at
it, and any matching how-to skill from `skills_folder`.

Use it before invoking an unfamiliar function so the agent can craft a
correct payload.

# Inputs

```json
{ "function_id": "agent-memory::observe" }
```

`function_id` is required. Anything else (search, paging) is delegated
to `directory::engine::functions::list`.

# Outputs

```json
{
  "function_id":     "agent-memory::observe",
  "worker_name":     "agent-memory",
  "description":     "Record an event in agent memory.",
  "request_schema":  { "type": "object", "properties": { ... } },
  "response_schema": { "type": "object", "properties": { ... } },
  "metadata":        null,
  "registered_triggers": [
    { "id": "trg-1", "trigger_type": "scheduler::tick", "config": { ... } }
  ],
  "how_guide": {
    "title":    "How to use memory observe",
    "skill_id": "agent-memory/observe",
    "body":     "# How to use memory observe ..."
  },
  "related_skills": [
    { "title": "Memory tour",         "skill_id": "agent-memory/index" },
    { "title": "Compaction strategy", "skill_id": "agent-memory/compact" }
  ]
}
```

`how_guide` is the **primary** how-to. It's `null` (or omitted) when no
markdown in `skills_folder` carries `type: how-to` plus a matching
`function_id` / `functions: [...]` array / body link to
`iii://fn/<dotted/path>`. Title precedence: frontmatter `title` → first
`# H1` in the body → `skill_id`.

`related_skills` lists every **other** skill (any frontmatter `type`)
that mentions this function — either via the literal `function_id` or
via the `iii://fn/<dotted/path>` URI. The bodies are intentionally
omitted; titles are surfaced for picker UIs and the bodies should be
loaded on demand via `directory::skills::fetch-skill iii://<skill_id>`.
The skill already returned as `how_guide` is excluded from this list to
avoid duplication.

# Worked example

```json
{ "function_id": "directory::engine::workers::list" }
```

Returns the input/output schemas for `workers::list`, attributes it to
the `directory` worker, and surfaces this very skill (you're reading it
via the `how_guide` field) when called against a function that has a
bundled how-to. Any other skill in `skills_folder` that mentions
`directory::engine::workers::list` (e.g. via
`iii://fn/directory/engine/workers/list`) shows up in `related_skills`,
so callers can drill in via `directory::skills::fetch-skill` when they
want the full body.

# Related

- `directory::engine::functions::list` — find the id you want to inspect.
- `directory::engine::workers::info` — group by worker instead of function.
- `directory::engine::registered-triggers::info` — look up a trigger that
  calls this function.

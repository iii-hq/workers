---
type: how-to
function_id: registry::worker-info
title: Inspect one worker's full registry metadata
---

# When to use

Call `registry::worker-info` to pull the FULL published metadata for
one worker from the public registry: worker envelope (name,
description, version, repo, author), readme markdown, the API
reference (functions + triggers with schemas), and the list of skill /
prompt files the bundle ships.

This is the REMOTE counterpart to `directory::worker-info`. Both
responses wrap the worker payload in a top-level `worker` field, and
the core fields (`name`, `description`, `version`) are guaranteed on
both surfaces so a parser that touches only those keys works against
either; everything else is surface-specific (registry adds `repo` /
`author` plus the top-level `readme`, `api_reference`, `skills_tree`,
directory adds runtime/connection state).

| Question                                              | Use this                |
|-------------------------------------------------------|-------------------------|
| What is THIS worker (connected to my engine) running? | `directory::worker-info` |
| What does the published version of THAT worker look like? | `registry::worker-info`  |

# Inputs

```json
{
  "name":    "agent-memory",  // required, non-empty
  "version": "1.2.3",         // optional, mutually exclusive with `tag`
  "tag":     "latest"         // optional, defaults to "latest" when neither version nor tag is given
}
```

You may pass either `version` or `tag`, not both. With neither the
worker info defaults to `tag: "latest"`.

# Outputs

```json
{
  "worker": {                                          // same shape as registry::worker-list rows
    "name":        "agent-memory",                     // shared core field with directory::worker-info.worker
    "description": "Persistent memory tier for agents.", // shared core field
    "version":     "1.2.3",                            // shared core field (the resolved version)
    "repo":        "https://github.com/iii-hq/workers",
    "author":      { "name": "iii", "is_verified": true }
  },
  "readme": "# agent-memory\n\nDocs here.",            // optional; null if registry omits it
  "api_reference": {
    "functions": [
      {
        "name":            "observe",
        "description":     "Record an event.",
        "request_schema":  { "type": "object", "...": "..." },
        "response_schema": { "type": "object", "...": "..." },
        "metadata":        null
      }
    ],
    "triggers": [
      {
        "name":              "on-change",
        "description":       "Fires when memory changes.",
        "invocation_schema": { "type": "object", "...": "..." },
        "return_schema":     { "type": "object", "...": "..." },
        "metadata":          null
      }
    ]
  },
  "skills_tree": {
    "skills":  [ { "path": "index.md" }, { "path": "agentmemory/observe.md" } ],
    "prompts": [ { "name": "summarize", "description": "Summarize a session." } ]
  }
}
```

# Caching

Each unique `(name, version|tag)` pair is cached for
`registry_cache_ttl_ms` (default 60s). Repeat calls within the TTL
window don't hit the registry — they return the same response from
in-process memory. To bust the cache, wait out the TTL or call with a
different version/tag.

# Worked example

Latest published metadata for `agent-memory`:

```json
{ "name": "agent-memory" }
```

Pin to an exact version:

```json
{ "name": "agent-memory", "version": "1.2.3" }
```

# Related

- `registry::worker-list` — discover the worker name first.
- `directory::worker-info` — same `worker` envelope against the
  connected engine.
- `skills::download` — install the worker's skill bundle locally
  (uses the same registry under the hood).

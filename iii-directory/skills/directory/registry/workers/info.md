---
type: how-to
function_id: directory::registry::workers::info
title: Inspect one worker's full registry metadata
---

> **Function id:** `directory::registry::workers::info` — pass this to `agent_trigger { function: "directory::registry::workers::info" }` (NOT the skill path you saw in `directory::skills::list`; that's a documentation id, not a callable function id).

# When to use

Call `directory::registry::workers::info` to pull the FULL published
metadata for one worker from the public registry: worker envelope
(name, description, version, repo, author, plus the publication
metadata `type` / `config` / `supported_targets` / `total_downloads` /
`dependencies` / optional `image`), readme markdown, the API
reference (functions + triggers with schemas), and the list of skill /
prompt files the bundle ships.

This is the REMOTE counterpart to `directory::engine::workers::info`.
Both responses wrap the worker payload in a top-level `worker` field
and the core fields (`name`, `description`, `version`) are guaranteed
on both surfaces, so a parser that only touches those keys works
against either; everything else is surface-specific (registry adds
publication metadata plus the top-level `readme`, `api_reference`,
`skills_tree`; the engine view adds runtime / connection state).

| Question                                                  | Use this                              |
|-----------------------------------------------------------|---------------------------------------|
| What is THIS worker (connected to my engine) running?     | `directory::engine::workers::info`    |
| What does the published version of THAT worker look like? | `directory::registry::workers::info`  |

# Inputs

```json
{
  "name":    "agent-memory",  // required, non-empty
  "version": "1.2.3",         // optional, mutually exclusive with `tag`
  "tag":     "latest"         // optional, defaults to "latest" when neither version nor tag is given
}
```

You may pass either `version` or `tag`, not both. With neither, the
worker info defaults to `tag: "latest"`. The worker rewrites both
inputs to `?version=…` on the wire (per the OpenAPI contract — the
registry's `?version` query param accepts both tags and exact semvers).

# Outputs

```json
{
  "worker": {
    "name":              "agent-memory",                       // shared core field
    "description":       "Persistent memory tier for agents.", // shared core field
    "type":              "binary",                             // binary | image | engine
    "version":           "1.2.3",                              // shared core field (resolved)
    "repo":              "https://github.com/iii-hq/workers",
    "config":            {},
    "supported_targets": ["x86_64-unknown-linux-gnu"],
    "total_downloads":   4242,
    "dependencies":      [],
    "author":            { "name": "iii", "pfp": null, "verified": true }
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
    "skills":  [ { "path": "index.md" }, { "path": "agent-memory/observe.md" } ],
    "prompts": [ { "name": "summarize", "description": "Summarize a session." } ]
  }
}
```

`worker` / `readme` / `api_reference` come from `GET /w/{slug}?version=…`.
`skills_tree` comes from a parallel `GET /w/{slug}/skills?version=…`
call — the worker fans both out concurrently and merges them, dropping
the markdown `content` and prompt `args_schema` from the skills payload
(call `directory::skills::download` to materialise bodies on disk).

# Caching

Each unique `(name, version|tag)` pair is cached for
`registry_cache_ttl_ms` (default 60s). Repeat calls within the TTL
window don't hit the registry — they return the same merged response
from in-process memory. To bust the cache, wait out the TTL or call
with a different version/tag.

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

- `directory::registry::workers::list` — discover the worker name first.
- `directory::engine::workers::info` — same core `worker` fields
  (`name` / `description` / `version`) against the connected engine.
- `directory::skills::download` — install the worker's skill bundle
  locally (uses the same registry under the hood).

---
type: how-to
function_id: directory::registry::workers::list
title: List workers from the public registry
---

# When to use

Use `directory::registry::workers::list` to search the public workers
registry (`api.workers.iii.dev`) by free-text term and get back a list
of PUBLISHED workers — the workers a user could install, regardless of
whether any of them are currently connected to this engine.

This is the REMOTE counterpart to `directory::engine::workers::list`.
Rows on both surfaces share a fixed set of core fields (`name`,
`description`, `version`) so a parser that touches only those keys
works against either; everything else is surface-specific (registry
adds `repo` / `author`, directory adds runtime/connection state).

| Question                                          | Use this                              |
|---------------------------------------------------|---------------------------------------|
| What workers are connected to MY engine right now? | `directory::engine::workers::list`    |
| What workers exist in the public registry?         | `directory::registry::workers::list`  |

# Inputs

```json
{
  "search": "memory",   // required, non-empty; forwarded to GET /search?q=...
  "limit":  20          // optional, default 20, capped at 100
}
```

The registry doesn't currently expose an unscoped browse endpoint, so
`search` MUST be non-empty. (Local `directory::engine::workers::list`
allows empty `search` since it has access to all connected workers
locally.)

# Outputs

```json
{
  "workers": [
    {
      "name":        "agent-memory",                            // shared core field
      "description": "Persistent memory tier for agents.",      // shared core field
      "version":     "0.4.0",                                   // shared core field (latest published)
      "repo":        "https://github.com/iii-hq/workers",
      "author":      { "name": "iii", "profile_picture": null, "is_verified": true }
    }
  ]
}
```

The first three fields (`name`, `description`, `version`) are shared
with `directory::engine::workers::list` rows.

# Caching

Each unique `(search, limit)` pair is cached for `registry_cache_ttl_ms`
(default 60s). Repeat calls within the TTL window don't hit the
registry — they return the same response from in-process memory.

# Worked example

Find every published worker mentioning "memory":

```json
{ "search": "memory" }
```

Top 5 results for "router":

```json
{ "search": "router", "limit": 5 }
```

# Related

- `directory::registry::workers::info` — full registry detail for one
  worker.
- `directory::engine::workers::list` — same row shape against connected
  workers.
- `directory::skills::download` — install a worker's skill bundle by
  name.

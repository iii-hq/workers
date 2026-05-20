---
type: how-to
function_id: directory::registry::workers::list
title: List workers from the public registry
---

# When to use

Use `directory::registry::workers::list` to browse or search the public
workers registry (`api.workers.iii.dev`) and get back a page of
PUBLISHED workers — the workers a user could install, regardless of
whether any of them are currently connected to this engine.

This is the REMOTE counterpart to `directory::engine::workers::list`.
Rows on both surfaces share the core fields `name` / `description` /
`version` (so a parser that only touches those keys works against
either), but the registry row also surfaces publication metadata
(`type`, `config`, `supported_targets`, `total_downloads`,
`dependencies`, optional `image`) that the engine view doesn't have.

| Question                                          | Use this                              |
|---------------------------------------------------|---------------------------------------|
| What workers are connected to MY engine right now? | `directory::engine::workers::list`    |
| What workers exist in the public registry?         | `directory::registry::workers::list`  |

# Inputs

```json
{
  "search": "memory",   // optional free-text query (matched fuzzy by pg_trgm against name + description)
  "cursor": "..."       // optional opaque cursor returned by a previous call's pagination.next_cursor
}
```

Both fields are optional. With no `search`, the registry orders by
`total_downloads DESC`. With `search`, it ranks by similarity. Page
size is server-authored — the client cannot override it.

# Outputs

```json
{
  "workers": [
    {
      "name":              "agent-memory",                       // shared core field
      "description":       "Persistent memory tier for agents.", // shared core field
      "type":              "binary",                             // binary | image | engine
      "version":           "0.4.0",                              // shared core field (latest published)
      "repo":              "https://github.com/iii-hq/workers",
      "config":            {},
      "supported_targets": ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"],
      "total_downloads":   4242,
      "dependencies":      [],
      "author":            { "name": "iii", "pfp": null, "verified": true }
    }
  ],
  "pagination": {
    "next_cursor": "eyJzIjo0Mi4wLCJpZCI6IjBkNTRhMWZmLTJjMjMtNGY4MC05ZTRkLTRmNmVkM2EwYTgxMiJ9",
    "has_more":    true,
    "page_size":   20
  }
}
```

The first three fields (`name`, `description`, `version`) are shared
with `directory::engine::workers::list` rows; everything else is
registry-only metadata.

`pagination.next_cursor` is opaque — pass it back as `cursor:` to fetch
the next page. `null` on the last page (with `has_more: false`).
`page_size` is the server's choice; clients can't override it.

# Caching

Each unique `(search, cursor)` pair is cached for `registry_cache_ttl_ms`
(default 60s). Repeat calls within the TTL window don't hit the
registry — they return the same response from in-process memory.

# Worked example

Browse the most-downloaded workers (no search):

```json
{}
```

Find every published worker mentioning "memory":

```json
{ "search": "memory" }
```

Fetch the next page (using a cursor from a previous call):

```json
{ "search": "memory", "cursor": "eyJzIjo0Mi4wLCJpZCI6IjBkNTRhMWZmLTJjMjMtNGY4MC05ZTRkLTRmNmVkM2EwYTgxMiJ9" }
```

# Related

- `directory::registry::workers::info` — full registry detail for one
  worker.
- `directory::engine::workers::list` — same shared core fields against
  connected workers.
- `directory::skills::download` — install a worker's skill bundle by
  name.

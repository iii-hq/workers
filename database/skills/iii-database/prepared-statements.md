---
type: how-to
functions: [database::prepareStatement, database::runStatement]
title: Prepare a SQL statement once, run it many times against a pinned connection
---

# When to use

Use the `prepareStatement` + `runStatement` pair when a single SQL string
will run repeatedly with different bind values, or when you need
session-scoped state (temp tables, `SET LOCAL`, transaction snapshots) to
persist across several calls. `prepareStatement` parses + plans the SQL
once and **pins a pool connection under a UUID handle**; `runStatement`
re-executes that handle with new params on the same connection.

| Question                                                         | Use this                                                  |
|------------------------------------------------------------------|-----------------------------------------------------------|
| Will this exact SQL run more than a handful of times?            | `database::prepareStatement` first, then re-use.      |
| Do I have a UUID handle already?                                 | `database::runStatement` with new `params`.           |
| Need session-scoped state (e.g. a Postgres advisory lock) across calls? | `database::prepareStatement` pins one connection.     |
| Just running this SQL once?                                      | [`database::query`](iii://database/query) — no handle, no pool pinning. |

**A live handle pins one pool connection until its TTL expires.** The
default TTL is 1 hour and the cap is 24 hours; while a handle exists,
`pool.max - 1` connections are available for everyone else. Hold only as
many handles as you need concurrently, and let them expire (or simply
stop calling them) when the workload ends — there is no `release` or
`close` function.

Use [`database::query`](iii://database/query) instead when the
SQL runs only once. Use
[`database::transaction`](iii://database/transaction) instead
when several statements need atomic commit/rollback semantics — handles
are not transactions; commit boundaries are defined by the SQL you run
through them.

# `database::prepareStatement`

## Inputs

```json
{
  "db":          "primary",                                   // required; key from your `databases:` config
  "sql":         "SELECT id, email FROM users WHERE id > ?",  // required; non-empty after trim
  "ttl_seconds": 3600                                         // optional; default 3600, capped at 86400
}
```

`db` and `sql` are required. Empty/whitespace-only `sql` is rejected
with `DRIVER_ERROR` carrying `message: "empty SQL"`. The guard runs
**before** the worker acquires a pool connection, so a typo cannot
silently leak a pinned conn.

`ttl_seconds` values above 86400 (24 h) are silently clamped down — no
error is returned. A background evictor sweeps expired entries every
30 s; a `runStatement` against an entry whose TTL elapsed between
evictor sweeps fails fast with `STATEMENT_NOT_FOUND` and removes the
entry as a side effect.

## Outputs

```json
{
  "handle": {
    "id":         "550e8400-e29b-41d4-a716-446655440000", // RFC 4122 v4 UUID
    "expires_at": "2026-05-19T18:30:00Z"                  // RFC 3339 UTC; now + ttl_seconds (clamped)
  }
}
```

- `handle.id` is the opaque UUID `runStatement` accepts. Stable until
  expiry; treat it as a session token.
- `handle.expires_at` is when the worker will stop honouring the
  handle. After that point `runStatement` returns `STATEMENT_NOT_FOUND`.

# `database::runStatement`

## Inputs

```json
{
  "handle_id": "550e8400-e29b-41d4-a716-446655440000", // required; UUID returned by prepareStatement
  "params":    [42]                                    // optional; positionally bound at run time
}
```

`handle_id` is required and must reference a live handle. Unknown or
expired ids return `STATEMENT_NOT_FOUND`; the response carries the
`handle_id` echo so callers can correlate failures.

`params` follows the same JSON-to-driver coercion as
[`database::query`](iii://database/query). The placeholder
syntax matches the SQL given to `prepareStatement` (`?` for
sqlite/mysql, `$1`/`$2`/... for postgres).

There is **no `timeout_ms`** input on `runStatement`. The handle pins a
pool connection for its full TTL, and per-call timeouts would not
short-circuit the underlying network round-trip on Postgres or MySQL —
configure the per-call ceiling via the connection's session lifetime
(e.g. `statement_timeout` in `postgresql.conf`) or shorten
`ttl_seconds` on `prepareStatement`.

## Outputs

```json
{
  "rows":      [{ "id": 1, "email": "a@x" }],                  // row-of-objects, same shape as `database::query`
  "row_count": 1,
  "columns":   [
    { "name": "id",    "type_name": "INTEGER" },
    { "name": "email", "type_name": "TEXT"    }
  ]
}
```

The envelope is bit-for-bit identical to
[`database::query`](iii://database/query) — same row coercion
rules, same `columns[i]` metadata, same empty-result handling. Callers
can share one parser for both surfaces.

`runStatement` does not surface write counts or `last_insert_id`. To
re-run an INSERT/UPDATE/DELETE many times and read those fields, use
[`database::execute`](iii://database/execute) per call — write
statements are typically cheap to re-parse and the prepared path saves
less than the pool-pinning cost.

# Worked example

Prepare a paginated SELECT once, then advance the cursor twice. Same
flow on every driver; the SQL changes its placeholder syntax.

SQLite or MySQL — prepare:

```json
{
  "db":          "primary",
  "sql":         "SELECT id, body FROM outbox WHERE id > ? ORDER BY id LIMIT 50",
  "ttl_seconds": 3600
}
```

Returns
`{ "handle": { "id": "550e8400-...", "expires_at": "..." } }`.

Run with cursor `0` (first page):

```json
{ "handle_id": "550e8400-...", "params": [0] }
```

Run again with cursor `50` (second page) on the same handle:

```json
{ "handle_id": "550e8400-...", "params": [50] }
```

If a `runStatement` returns `STATEMENT_NOT_FOUND` mid-loop, the handle
expired (or the worker process restarted): re-prepare and continue from
the last successfully-read cursor. Do **not** retry the same
`handle_id`.

# Related

- `database::query` — drop the handle altogether for one-shot reads; same response envelope.
- `database::execute` — for repeated writes; pair its own `last_insert_id` with `affected_rows` per call.
- `database::transaction` — group writes into an atomic batch instead of holding a pinned connection across many calls.
- Error code `STATEMENT_NOT_FOUND` — re-prepare and retry with the new `handle.id`; the old one is gone.
- Error code `POOL_TIMEOUT` — too many live handles can starve the pool. Bump `pool.max` in your `databases:` config or shorten `ttl_seconds`.

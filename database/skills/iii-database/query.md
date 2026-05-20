---
type: how-to
function_id: database::query
title: Run a read-only SQL query and return rows
---

# When to use

Call `database::query` for any read-side SQL — `SELECT`, `WITH`,
`PRAGMA`, `EXPLAIN`, anything that produces a result set you want
materialized as JSON. The response carries the rows as objects keyed by
column name plus a `columns` array with per-column type metadata, so
callers don't need to issue a follow-up describe call.

Reach for it when:

- You need a row-of-objects shape for a UI table or a JSON-returning
  function. Each row is `{column_name: value}`, not a positional array.
- You only run the SQL once or a small handful of times. Bind parameters
  inline via `params` rather than reaching for prepared statements.
- You want column-name + driver-type metadata alongside the rows
  (`columns[i].name` and `columns[i].type_name`).

Use [`database::execute`](iii://database/execute) instead when the
statement writes (INSERT/UPDATE/DELETE/DDL) — `query` only returns rows;
write counts and `last_insert_id` come from `execute`.

Use [`database::prepareStatement` + `runStatement`](iii://database/prepared-statements)
instead when you'll re-run the same SQL many times in a hot loop — the
prepared path skips the per-call parse/plan cost and pins a pool
connection so isolation primitives like temp tables stay alive across
calls.

# Inputs

```json
{
  "db":         "primary",                     // required; key from your `databases:` config
  "sql":        "SELECT id, email FROM users WHERE id > ?",  // required; non-empty after trim
  "params":     [42],                          // optional; bound positionally (`?` for sqlite/mysql, `$1`/`$2`/... for postgres)
  "timeout_ms": 30000                          // optional; per-call cap, default 30000
}
```

`db` and `sql` are required. Empty/whitespace-only `sql` is rejected at
the handler boundary with a `DRIVER_ERROR` carrying `message: "empty SQL"`
— this is uniform across all three drivers (Postgres' `tokio-postgres`
treats empty SQL as a successful no-op, sqlite/mysql parse-error it; the
worker normalizes the contract).

`params` accepts JSON primitives (`null`, `bool`, integer, float,
string), arrays, and objects — arrays/objects bind as the driver's JSON
type. A number that fits neither `i64` nor `f64` is rejected with
`INVALID_PARAM` carrying the offending index. Placeholder syntax differs
per driver: `?` for sqlite and mysql, `$1`/`$2`/... for postgres.

`timeout_ms` exceeded yields `QUERY_TIMEOUT` with the `db` and the
configured cap.

# Outputs

```json
{
  "rows": [                                    // row-of-objects, ordered to match the SQL
    { "id": 1, "email": "a@x" },
    { "id": 2, "email": "b@x" }
  ],
  "row_count": 2,                              // == rows.length; convenience field
  "columns": [                                 // per-column metadata in projection order
    { "name": "id",    "type_name": "INTEGER" },
    { "name": "email", "type_name": "TEXT"    }
  ]
}
```

- `rows` is always an array of objects keyed by `columns[i].name`. Empty
  result sets return `[]`, not `null`.
- `row_count` is exactly `rows.length`; included so callers can branch on
  presence without re-measuring.
- `columns[i].type_name` is the driver's native type name (e.g.
  `INTEGER`, `TEXT`, `int4`, `varchar`); use it as a hint, not a
  contract.
- Cell coercion follows fixed rules:
  - Integer columns become JSON numbers when they fit `i64`.
  - 64-bit identity columns serialize as JSON **strings** to preserve
    precision past `Number.MAX_SAFE_INTEGER` in JS clients.
  - `BYTEA` / `BLOB` cells are base64-encoded strings (standard alphabet,
    with padding).
  - `TIMESTAMP` / `TIMESTAMPTZ` cells are RFC 3339 strings in UTC
    (`...Z`), seconds precision.
  - `NUMERIC` / `DECIMAL` cells are JSON strings (no precision loss).
  - `JSON` / `JSONB` columns pass through as the embedded value.

# Worked example

Read every user past a given id, with the cursor bound positionally.

SQLite or MySQL:

```json
{
  "db":     "primary",
  "sql":    "SELECT id, email FROM users WHERE id > ? ORDER BY id LIMIT ?",
  "params": [42, 100]
}
```

Postgres uses numbered placeholders for the same call:

```json
{
  "db":     "primary",
  "sql":    "SELECT id, email FROM users WHERE id > $1 ORDER BY id LIMIT $2",
  "params": [42, 100]
}
```

Both return the same envelope; the `columns[i].type_name` strings will
differ (e.g. sqlite `INTEGER` vs postgres `int4`) but `rows` are
shape-compatible.

# Related

- `database::execute` — for the write side (INSERT/UPDATE/DELETE/DDL); returns affected-row counts instead of materialized rows.
- `database::prepareStatement` + `database::runStatement` — re-run the same SQL many times without re-parsing; also pins a pool connection so session-scoped state (temp tables, `SET LOCAL`, ...) survives across calls.
- `database::transaction` — group several statements (mixed read/write) into one atomic batch with a single `committed` flag.

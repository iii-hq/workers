---
type: how-to
function_id: database::execute
title: Run a write statement and return affected rows
---

# When to use

Call `database::execute` for any write-side SQL — `INSERT`, `UPDATE`,
`DELETE`, or DDL (`CREATE TABLE`, `ALTER TABLE`, `DROP INDEX`, ...). The
response carries `affected_rows`, an optional `last_insert_id`, and a
`returned_rows` array populated when the caller asks for `RETURNING`-style
output.

Reach for it when:

- You need the count of rows the write touched (`affected_rows`).
- You inserted into a table with an autoincrement primary key and want
  the newly-assigned id without a follow-up `SELECT`.
- You want a write to surface specific columns from each affected row
  (e.g. server-defaulted `id` + `created_at`) — set `returning` on
  Postgres or SQLite.

Use [`database::query`](iii://database/query) instead when the
statement reads — `execute` does run a `SELECT` if you give it one but
discards the rows and reports `affected_rows: 0`, which is rarely what a
SELECT caller wants.

Use [`database::transaction`](iii://database/transaction) instead
when you need several writes to commit atomically — `execute` runs each
call as its own implicit transaction.

# Inputs

```json
{
  "db":        "primary",                                            // required; key from your `databases:` config
  "sql":       "INSERT INTO users (email) VALUES (?) RETURNING id",  // required; non-empty after trim
  "params":    ["a@x"],                                              // optional; bound positionally
  "returning": ["id", "created_at"]                                  // optional; SQLite + Postgres only — see Driver compatibility
}
```

`db` and `sql` are required. Empty/whitespace-only `sql` is rejected
uniformly with `DRIVER_ERROR` carrying `message: "empty SQL"` (matches
[`database::query`](iii://database/query)'s contract).

`params` accepts JSON primitives, arrays, and objects exactly like
`query` — same per-driver placeholder syntax (`?` for sqlite/mysql,
`$1`/`$2`/... for postgres) and same `INVALID_PARAM` rule for
out-of-range numbers.

`returning` is the column projection the driver fills into
`returned_rows` when supported. **MySQL ignores it** (warns once and
returns `returned_rows: []`); SQLite implements it via the native
`RETURNING` clause; Postgres reads it from the SQL's own `RETURNING`
list. To stay portable, prefer writing the `RETURNING ...` clause
directly in `sql` for sqlite/postgres rather than passing
`returning: [...]`.

# Outputs

```json
{
  "affected_rows":  1,                                  // rows the engine reports as inserted/updated/deleted
  "last_insert_id": "42",                               // string-encoded; null when not applicable — see below
  "returned_rows":  [{ "id": 42, "created_at": "..." }] // empty array when no RETURNING is used
}
```

- `affected_rows` is the engine-reported write count. DDL statements
  return `0` on every driver.
- `last_insert_id` is **always a JSON string or `null`** so the field can
  carry sequence values that overflow JS `Number.MAX_SAFE_INTEGER`. It is
  populated only for inserts:
  - **SQLite / MySQL**: surfaces the engine's `last_insert_rowid()` /
    `LAST_INSERT_ID()` for the connection. **Only set on INSERT** — an
    `UPDATE` that runs immediately after an `INSERT` on the same pooled
    connection returns `null` (not the prior INSERT's rowid). Falls back
    to `null` when the engine reports `0` (no INSERT has run on that
    connection yet).
  - **Postgres**: has no engine-level `LASTVAL()` equivalent in this
    surface. The worker reads the **first column of the first
    `RETURNING` row** as the id. Put your primary key first:
    `RETURNING id, name` works; `RETURNING name, id` returns `name` as
    `last_insert_id`. With no `RETURNING` clause, the field is `null`.
- `returned_rows` mirrors the row-of-objects shape from
  [`database::query`](iii://database/query). Empty `[]` when the
  statement omits `RETURNING` or runs on MySQL.

# Worked example

Insert one row and capture the autoincrement id (SQLite or MySQL):

```json
{
  "db":     "primary",
  "sql":    "INSERT INTO users (email) VALUES (?)",
  "params": ["a@x"]
}
```

Returns `{ "affected_rows": 1, "last_insert_id": "1", "returned_rows": [] }`.

Same intent on Postgres — the worker pulls `last_insert_id` from the
first `RETURNING` column, so put `id` first:

```json
{
  "db":  "primary",
  "sql": "INSERT INTO users (email) VALUES ($1) RETURNING id, email",
  "params": ["a@x"]
}
```

Returns
`{ "affected_rows": 1, "last_insert_id": "1", "returned_rows": [{ "id": 1, "email": "a@x" }] }`.

A bulk update reports the count and leaves `last_insert_id` null
(no INSERT happened):

```json
{
  "db":     "primary",
  "sql":    "UPDATE users SET active = ? WHERE last_seen_at < ?",
  "params": [false, "2026-01-01T00:00:00Z"]
}
```

Returns `{ "affected_rows": 17, "last_insert_id": null, "returned_rows": [] }`.

# Related

- `database::query` — for read SQL; returns materialized rows + column metadata instead of `affected_rows`.
- `database::transaction` — group several writes into one atomic batch with rollback on first failure.
- `database::prepareStatement` + `database::runStatement` — re-run the same parameterized write many times without re-parsing on each call.

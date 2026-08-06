---
name: database
description: >-
  Run SQL against PostgreSQL, MySQL, or SQLite from the iii engine — reads,
  writes, transactions, and prepared statements over managed connection pools.
---

# database

The database worker connects to PostgreSQL, MySQL, and SQLite through a
managed per-database connection pool. Every callable surface lives under
the `database::*` namespace. The driver is chosen from each database URL
scheme (`sqlite:`, `postgres://`, `postgresql://`, `mysql://`).

Runtime settings live in the `configuration` worker under id `database`;
pools hot-reload when the value changes. SQLite is the recommended starting
point. Placeholder syntax: `?` for SQLite and MySQL, `$1`/`$2`/… for Postgres.

## When to Use

- You need to read rows from a configured database (`database::query`).
- You need to insert, update, delete, or run DDL and read affected-row
  counts or autoincrement ids (`database::execute`).
- Several statements must commit or roll back together as one unit
  (`database::transaction`, `database::executeBatch`, or the interactive
  transaction surface).
- The same parameterized SQL will run many times and you want to skip
  per-call parse/plan cost (`database::prepareStatement` +
  `database::runStatement`).
- You need read-your-writes across round-trips with logic between steps
  (`database::beginTransaction` … `commitTransaction` / `rollbackTransaction`).

## Boundaries

- Not a migration tool, ORM, or schema designer — pass raw SQL only.
- Not a general pub/sub bus. `database::row-changed` reports only what THIS
  worker wrote, on commit — not change data capture; a write from psql or
  another worker is invisible to it.
- `database::query` is read-oriented; use `database::execute` for writes.
  Running a SELECT through `execute` discards rows.
- Prepared handles pin a pool connection until TTL expiry — not transactions.
  Batch `database::transaction` / `database::executeBatch` need every
  statement up front; use the interactive surface when code must branch
  between steps.
- MySQL ignores the `returning` option on `execute` (warn-once). SQLite
  degrades `read_committed` / `repeatable_read` isolation to serializable.
- For filesystem or shell operations, use the `shell` worker instead.

## Functions

- `database::query` — run read-only SQL and return rows, row count, and
  column metadata.
- `database::execute` — run write SQL (INSERT/UPDATE/DELETE/DDL) and
  return affected rows, optional last insert id, and optional RETURNING rows.
- `database::executeBatch` — convenience form of `transaction`: statements
  may be bare SQL strings or `{sql, params}` objects (prefer `params` for
  dynamic values). Same atomic semantics, envelope, and `failed_index`
  reporting as `transaction`.
- `database::prepareStatement` — parse and plan SQL once; return a handle
  that pins a pool connection until TTL expiry.
- `database::runStatement` — re-execute a prepared handle with new bind
  params; response shape matches `query`.
- `database::transaction` — run an ordered batch of statements atomically;
  rolls back on first failure and reports `failed_index`.
- `database::beginTransaction` — open an interactive transaction and
  return an id plus expiry deadline.
- `database::transactionQuery` — read SQL inside an open interactive
  transaction; same envelope as `query`.
- `database::transactionExecute` — write SQL inside an open interactive
  transaction; same envelope as `execute`. Rejects bare transaction-control
  SQL — finalize via `commitTransaction` or `rollbackTransaction`.
- `database::commitTransaction` — commit and finalize an interactive
  transaction.
- `database::rollbackTransaction` — roll back and finalize an interactive
  transaction.
- `database::listDatabases` — every configured database with its driver,
  credential-redacted URL, pool settings and TLS mode. Config only; use
  `database::health` for live state.

Interactive transactions auto-roll back when `timeout_ms` elapses (default
30 s, max 5 min). Prepared handles default to a 1 h TTL (max 24 h) with no
explicit release call — let them expire or stop using them when done.

### Reading the schema

One shape across all three drivers — prefer these over hand-writing
`sqlite_master` / `information_schema` / `PRAGMA`.

- `database::listTables` — tables and views, with kind and (postgres) schema.
- `database::describeTable` — columns with type, nullability, default, primary
  key and a structured `foreign_key` of `{ schema, table, column }`; plus
  indexes and a planner row estimate.
- `database::describeSchema` — the same for every table in one pass. Use this
  rather than looping `describeTable`.
- `database::schemaDiagram` — positioned nodes, routed foreign-key edges, hub
  `degree` and `isolated` tables. For reasoning about a schema's shape, not
  only for drawing it.

### Reading data

- `database::browseTable` — paged, sorted, filtered reads with no SQL. Filters
  are `{ column, op, value }` and `total` honours them. Follow a foreign key
  with an equality filter at `page_size: 1`.
- `database::explain` — the plan as a tree with costs and warnings. `analyze`
  **runs** the statement, so it defaults to false and is refused for anything
  that is not a single read.
- `database::columnStats` — planner statistics by default (approximate,
  labelled `source: planner`); `exact: true` scans. To profile rows you already
  hold, use the `fp` worker on a `browseTable` result instead.

### Operations and reuse

- `database::health` — pool occupancy, active queries, table sizes, locks,
  cache ratio. Each section is `available`, `unsupported` or `denied`, so a
  driver gap is never mistaken for an empty result.
- `database::terminateQuery` — end a session, or cancel its statement with
  `cancel_only`. Takes an id from `database::health`.
- `database::saveQuery`, `database::listSavedQueries`,
  `database::deleteSavedQuery` — named queries per database, kept in the
  `state` worker.
- `database::history` — recent queries, newest first. Best effort, not an audit
  log; bind `database::row-changed` for that.

## Reacting to writes

Register a `database::row-changed` trigger to be told when this worker commits
a change, instead of polling:

```json
{ "trigger_type": "database::row-changed", "config": { "db": "primary", "table": "orders", "ops": ["insert"] } }
```

The event is `{ db, table, op, affected_rows, returning?, at }`. It fires on
commit — an interactive transaction's writes are announced by
`commitTransaction`, and a rollback announces nothing. `table` is null when the
statement's table cannot be read off the SQL (a CTE-wrapped write), and
`runStatement` does not fire because it has no affected-row count to report.
Delivery is best-effort: it is not durable with the commit and has no replay or
exactly-once guarantee.

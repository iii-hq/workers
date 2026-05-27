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
  (`database::transaction` or the interactive transaction surface).
- The same parameterized SQL will run many times and you want to skip
  per-call parse/plan cost (`database::prepareStatement` +
  `database::runStatement`).
- You need read-your-writes across round-trips with logic between steps
  (`database::beginTransaction` … `commitTransaction` / `rollbackTransaction`).
- You want to react to Postgres row-level changes once logical replication
  streaming ships (`database::row-change` trigger — see below).

## Boundaries

- Not a migration tool, ORM, or schema designer — pass raw SQL only.
- Not a general pub/sub bus — use `database::row-change` only for Postgres
  table change feeds, not for application events.
- `database::query` is read-oriented; use `database::execute` for writes.
  Running a SELECT through `execute` discards rows.
- Prepared handles pin a pool connection until TTL expiry — not transactions.
  Batch `database::transaction` needs every statement up front; use the
  interactive surface when code must branch between steps.
- MySQL ignores the `returning` option on `execute` (warn-once). SQLite
  degrades `read_committed` / `repeatable_read` isolation to serializable.
- For filesystem or shell operations, use the `shell` worker instead.

## Functions

- `database::query` — run read-only SQL and return rows, row count, and
  column metadata.
- `database::execute` — run write SQL (INSERT/UPDATE/DELETE/DDL) and
  return affected rows, optional last insert id, and optional RETURNING rows.
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

Interactive transactions auto-roll back when `timeout_ms` elapses (default
30 s, max 5 min). Prepared handles default to a 1 h TTL (max 24 h) with no
explicit release call — let them expire or stop using them when done.

## Reactive triggers

Register a `database::row-change` trigger when a function should run
automatically on Postgres INSERT/UPDATE/DELETE for specific tables — without
polling with `database::query`.

Reach for it when:

- A downstream worker or workflow must react to row mutations in near real
  time on Postgres.
- You need decoded row payloads (old/new values) from logical replication
  rather than polling an outbox table.

Do not bind when:

- The writer already has the new row in its `execute` or `transactionExecute`
  return payload.
- You are on SQLite or MySQL — this trigger type is Postgres-only.
- You need events today — v1.0.0 returns `UNSUPPORTED` on `registerTrigger`
  pending an upstream `tokio-postgres` replication API release.

### How to bind

1. Register a handler: `registerFunction('stream::on-row-change', handler)`.
2. Register the trigger:

```typescript
iii.registerTrigger({
  type: 'database::row-change',
  function_id: 'stream::on-row-change',
  config: {
    db: 'primary',
    schema: 'public',
    tables: ['orders', 'payments'],
    // optional: slot_name, publication_name — see get function info
  },
})
```

Config: `db`, `schema` (default `public`), `tables`. Slot/publication names
derive from `trigger_id` unless overridden. For event payload shape, call
`get function info` on the trigger type or handler function id.

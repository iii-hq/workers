---
type: index
title: iii-database
---

# iii-database

Connect to PostgreSQL, MySQL, and SQLite from the iii engine. Run read-only
queries, write statements, atomic transactions, and prepared-statement
sequences over a managed per-database connection pool. Every callable
surface lives under the single `iii-database::*` namespace; SQLite is the
recommended starting point because it needs no server, just a file.

The worker resolves the driver from each database's URL scheme (`sqlite:`,
`postgres://`, `postgresql://`, `mysql://`). For the `databases:` config
block, TLS modes, error-code reference, and the per-driver compatibility
table (e.g. `returning:` is a no-op on MySQL; SQLite degrades
`read_committed` / `repeatable_read` to `serializable`), see
[the README](../README.md).

## How-tos

### `iii-database::*`

- [`iii-database::query`](iii://iii-database/query) — read-only SQL; returns `{ rows, row_count, columns }` for SELECT-style statements.
- [`iii-database::execute`](iii://iii-database/execute) — write SQL (INSERT/UPDATE/DELETE/DDL); returns `{ affected_rows, last_insert_id, returned_rows }`.
- [`iii-database::prepareStatement` + `iii-database::runStatement`](iii://iii-database/prepared-statements) — prepare-once, run-many parameterized SQL; the prepare step pins a pool connection until TTL expiry, so always run before re-issuing the same statement many times.
- [`iii-database::transaction`](iii://iii-database/transaction) — atomic batch sequence; one call, pass every statement together, rolls back on first failure with a `failed_index` pointer at the offending step.
- [`iii-database::beginTransaction` + `transactionQuery` / `transactionExecute` + `commitTransaction` / `rollbackTransaction`](iii://iii-database/interactive-transaction) — stateful interactive transaction with a configurable timeout-driven auto-rollback. Use this when you need read-your-writes across several round-trips inside a single transaction.

`iii-database::row-change` (Postgres logical replication via `pgoutput`) is
registered as a trigger type but is **not yet functional in v1.0.0**:
`register_trigger` returns `UNSUPPORTED` while the streaming decode loop
waits on an upstream `tokio-postgres` replication API release. Operators
can pre-provision slots and publications now; see the **Triggers** section
of [the README](../README.md) for current status and cleanup commands.

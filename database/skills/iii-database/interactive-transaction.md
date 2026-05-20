---
type: how-to
functions: [database::beginTransaction, database::transactionQuery, database::transactionExecute, database::commitTransaction, database::rollbackTransaction]
title: Run a stateful transaction across multiple RPC calls with a timeout-driven auto-rollback
---

# When to use

Use the interactive transaction surface when you need **read-your-writes
across several round-trips** inside one transaction — e.g. issue a write,
inspect the resulting state, branch in your application code, then issue
follow-up writes that all commit (or all roll back) together. The worker
pins one pool connection inside a server-side `BEGIN ... COMMIT/ROLLBACK`
under a UUID handle; subsequent calls re-use the same connection.

Reach for it when:

- You need to take a decision in application code *between* statements
  that must commit together (the batch
  [`database::transaction`](iii://database/transaction) requires
  every statement up-front, so it can't carry inter-statement logic).
- You want a single transaction id to thread through a long-running
  workflow without holding a network connection open in your code.
- You need stronger-than-default isolation for a multi-step read+write
  flow (pass `isolation: "serializable"` to `beginTransaction`).

Use [`database::transaction`](iii://database/transaction) instead
when every statement is known in advance — it skips the registry overhead
and the per-call round-trip cost. Use
[`database::execute`](iii://database/execute) for one-off writes;
each `execute` call is its own auto-committed transaction.

# Lifecycle

```
beginTransaction(db, isolation?, timeout_ms?)         → { transaction: { id, expires_at } }
  ├─ transactionQuery   ( transaction_id, sql, params )  → { rows, row_count, columns }
  ├─ transactionExecute ( transaction_id, sql, params )  → { affected_rows, last_insert_id, returned_rows }
  ├─ ... (repeat) ...
  ├─ commitTransaction   ( transaction_id )              → { committed: true }
  └─ rollbackTransaction ( transaction_id )              → { rolled_back: true }
```

After `commitTransaction` / `rollbackTransaction` returns, the id is gone
— any subsequent call against it returns `TRANSACTION_NOT_FOUND`. If the
configured `timeout_ms` elapses before either finalizer lands, the worker
auto-rolls back and removes the id; the next call also gets
`TRANSACTION_NOT_FOUND`.

# `database::beginTransaction`

## Inputs

```json
{
  "db":         "primary",          // required; key from your `databases:` config
  "isolation":  "serializable",     // optional; read_committed | repeatable_read | serializable
  "timeout_ms": 30000               // optional; default 30000, clamped to max 300000 (5 min)
}
```

`db` is required. `isolation` accepts the same three values as the batch
[`database::transaction`](iii://database/transaction); any other
string is rejected with `INVALID_PARAM`. On SQLite, `read_committed` /
`repeatable_read` log a one-line `tracing::warn!` and fall back to
`BEGIN IMMEDIATE` (serializable in practice).

`timeout_ms` is the **total lifetime of the transaction**, not an
inactivity timeout. It defaults to 30 s and is clamped at 5 min so a
buggy client can't pin a pool connection indefinitely. A background
sweep task fires `ROLLBACK` once the deadline elapses, then removes the
id from the registry.

## Outputs

```json
{
  "transaction": {
    "id":         "550e8400-e29b-41d4-a716-446655440000",
    "expires_at": "2026-05-19T18:30:00Z"
  }
}
```

`transaction.id` is the opaque UUID every subsequent handler accepts.
Treat it as a session token: scope it to one workflow, hand it off
between functions if needed, but never share it across unrelated requests.

`transaction.expires_at` is RFC 3339 UTC; once it elapses the worker has
already auto-rolled back. Re-issue `beginTransaction` and start over;
the original id is gone.

# `database::transactionQuery` / `database::transactionExecute`

The envelopes are **identical to the standalone
[`query`](iii://database/query) and
[`execute`](iii://database/execute)** handlers — same row-of-objects
shape, same `columns` metadata, same `affected_rows` /
`last_insert_id` / `returned_rows` semantics. The only difference: SQL
runs on the pinned transaction connection.

```json
{
  "transaction_id": "550e8400-...",        // required; id from beginTransaction
  "sql":            "SELECT n FROM t WHERE id = ?",
  "params":         [42]
}
```

`transactionExecute` adds a `returning` array exactly like
[`execute`](iii://database/execute) for Postgres + SQLite
`RETURNING` clauses; MySQL ignores it (logged warn-once).

`transactionExecute` **rejects** bare `BEGIN`, `COMMIT`, `ROLLBACK`,
`END`, `SAVEPOINT`, `RELEASE`, and `SET TRANSACTION ...` SQL with
`INVALID_PARAM` (`reason` points at `commitTransaction` /
`rollbackTransaction`). Finalization is a first-class handler, not a
side-channel — this makes the lifecycle observable: every commit and
every rollback shows up as its own engine call + log event.

Concurrent calls against the same `transaction_id` serialize on the
per-conn mutex. The worker doesn't pipeline statements within one
transaction.

# `database::commitTransaction` / `database::rollbackTransaction`

```json
{ "transaction_id": "550e8400-..." }
```

Returns `{ "committed": true }` or `{ "rolled_back": true }` on success.
The id is removed atomically before the connection is locked, so a
racing `transactionQuery` / `transactionExecute` either landed before
finalization (succeeded inside the same transaction) or after
(returns `TRANSACTION_NOT_FOUND`).

If `commitTransaction` fails (e.g. serialization conflict in Postgres),
the worker issues a best-effort `ROLLBACK` before returning the error —
the pool's recycler doesn't run rollback for us, so without this the
next caller on the recycled connection would see "current transaction is
aborted, commands ignored".

# Errors

| Code | Surface | Meaning |
|---|---|---|
| `UNKNOWN_DB` | `beginTransaction` | `db` not in your `databases:` config. |
| `INVALID_PARAM` | `beginTransaction` | Unknown `isolation`. |
| `INVALID_PARAM` | `transactionExecute` | Transaction-control SQL — use the dedicated finalizer handler. |
| `TRANSACTION_NOT_FOUND` | any of `transactionQuery` / `transactionExecute` / `commitTransaction` / `rollbackTransaction` | Id unknown, already finalized, or timed out. |
| `POOL_TIMEOUT` | `beginTransaction` | Pool was busy when `beginTransaction` tried to acquire a connection. Bump `pool.max` or shorten the longest-running transaction. |
| `DRIVER_ERROR` | any of the above | Wraps the underlying driver error with `inner_code`; same shape as elsewhere. |

# Observability

Every call lands on its own engine-managed OTel span. The worker emits
structured log events via `iii_sdk::Logger` that attach to the active
span, including `db.system`, `db.name`, `db.transaction.id`,
`db.operation`, and `db.statement` (no params — those can carry PII).

Key event names you can grep for in your trace backend:

- `db_tx_started` — info on `beginTransaction` success.
- `db_tx_statement` — debug per `transactionQuery` / `transactionExecute`.
- `db_tx_statement_failed` — warn on driver-level failure inside a tx.
- `db_tx_unknown` — warn when an id is missing/finalized/timed-out.
- `db_tx_committed` / `db_tx_rolled_back` — info on explicit finalization (carries `duration_ms`).
- `db_tx_commit_failed` — error on COMMIT failure (also indicates whether the follow-up rollback succeeded).
- `db_tx_timed_out` — warn from the background watcher when the deadline auto-rolls back.
- `db_tx_timeout_rollback_failed` — error when the timeout-driven ROLLBACK itself fails (rare).

# Worked example

Transfer money between two accounts with a read-then-write check that
must see the updated balance before deciding whether to credit:

```ts
const { transaction } = await iii.trigger({
  function_id: 'database::beginTransaction',
  payload: {
    db: 'primary',
    isolation: 'serializable',
    timeout_ms: 5000,
  },
})

try {
  const debit = await iii.trigger({
    function_id: 'database::transactionExecute',
    payload: {
      transaction_id: transaction.id,
      sql: 'UPDATE accounts SET balance = balance - ? WHERE id = ? AND balance >= ?',
      params: [10, 1, 10],
    },
  })
  if (debit.affected_rows !== 1) {
    await iii.trigger({
      function_id: 'database::rollbackTransaction',
      payload: { transaction_id: transaction.id },
    })
    throw new Error('insufficient funds')
  }
  await iii.trigger({
    function_id: 'database::transactionExecute',
    payload: {
      transaction_id: transaction.id,
      sql: 'UPDATE accounts SET balance = balance + ? WHERE id = ?',
      params: [10, 2],
    },
  })
  await iii.trigger({
    function_id: 'database::commitTransaction',
    payload: { transaction_id: transaction.id },
  })
} catch (e) {
  // Best-effort rollback; the worker is happy to see TRANSACTION_NOT_FOUND
  // if a prior step already rolled it back or the deadline elapsed.
  try {
    await iii.trigger({
      function_id: 'database::rollbackTransaction',
      payload: { transaction_id: transaction.id },
    })
  } catch {/* ignore */}
  throw e
}
```

# Related

- [`database::transaction`](iii://database/transaction) — atomic
  batch when every statement is known up-front; skip the round-trip
  overhead of the interactive surface.
- [`database::query`](iii://database/query) /
  [`database::execute`](iii://database/execute) — one-off
  read/write outside any transaction.
- [`database::prepareStatement` +
  `runStatement`](iii://database/prepared-statements) — pin a
  connection for repeated parameterized calls **without** transactional
  semantics. Useful for read-heavy workloads where the prepared plan is
  the bottleneck.

---
type: how-to
function_id: iii-database::transaction
title: Run a sequence of statements atomically with rollback on first failure
---

# When to use

Call `iii-database::transaction` when several SQL statements must commit
or roll back together — the canonical "transfer money between accounts"
shape, plus any multi-step write that would leave the DB in an
inconsistent state if a later statement failed. The worker opens a
transaction on a freshly-acquired pool connection, runs every statement
in sequence, and commits if (and only if) every step returned without
error.

Reach for it when:

- You're running two or more writes that share a consistency invariant
  (e.g. debit + credit, parent + child rows, denormalized counter
  updates).
- You need a stronger isolation level than the engine's default — pass
  `isolation: "serializable"` (or `"repeatable_read"` on Postgres/MySQL)
  to upgrade just this batch.
- You want a single response that tells you *which* statement failed
  when something rolls back (`failed_index`).

Use [`iii-database::execute`](iii://iii-database/execute) instead for a
single write — `execute` is implicitly its own transaction and skips the
multi-statement framing cost. Use
[`iii-database::prepareStatement` + `runStatement`](iii://iii-database/prepared-statements)
instead when the goal is repeating one parameterized statement many
times rather than committing several different statements as a unit.

# Inputs

```json
{
  "db":         "primary",                                  // required; key from your `databases:` config
  "statements": [                                           // required; array, run in order
    { "sql": "INSERT INTO accounts (id, balance) VALUES (?, ?)", "params": [1, 100] },
    { "sql": "INSERT INTO accounts (id, balance) VALUES (?, ?)", "params": [2,   0] }
  ],
  "isolation":  "serializable"                              // optional; one of read_committed | repeatable_read | serializable
}
```

`db` and `statements` are required; an empty `statements` array
commits a no-op transaction (`committed: true`, `results: []`). Each
statement carries its own `sql` (non-empty, like
[`iii-database::query`](iii://iii-database/query) and `execute`) plus
optional `params` with the same JSON-to-driver coercion rules.

`isolation` is optional and accepts exactly three values:
`"read_committed"`, `"repeatable_read"`, `"serializable"`. Anything else
(including the empty string) is rejected with `INVALID_PARAM` carrying
`reason: "unknown isolation \`<value>\`"`. Per-driver behaviour:

- **Postgres**: maps directly to `BEGIN ISOLATION LEVEL ...`.
- **MySQL**: maps directly to `SET TRANSACTION ISOLATION LEVEL ...`
  before `BEGIN`.
- **SQLite**: only supports a single isolation level. `serializable`
  uses `BEGIN IMMEDIATE`; `read_committed` and `repeatable_read` log a
  one-line `tracing::warn!` and fall back to `BEGIN IMMEDIATE`. The
  call still succeeds; check your worker logs if you expect strict
  isolation semantics on SQLite.

Omitting `isolation` uses the driver's session default
(`READ COMMITTED` on Postgres + MySQL; serializable on SQLite by
construction).

# Outputs

The response shape changes based on `committed`. The two shapes never
overlap; success has no `failed_index`/`error`, failure has no
`results`.

Success:

```json
{
  "committed": true,
  "results": [                              // one entry per input statement, same order
    { "affected_rows": 1, "rows": [] },     // statement 0
    { "affected_rows": 1, "rows": [] }      // statement 1
  ]
}
```

Failure (rollback):

```json
{
  "committed":    false,
  "failed_index": 1,                        // 0-based index of the offending statement; absent for non-step failures
  "error": {                                // structured DbError; same shape returned by `query`/`execute` on driver failure
    "code":         "DRIVER_ERROR",
    "driver":       "sqlite",
    "inner_code":   null,
    "message":      "constraint failed: NOT NULL on accounts.id",
    "failed_index": 1
  }
}
```

- `committed` is the truthy/falsy split. The transaction either
  committed every statement or rolled back every statement; partial
  commit is impossible.
- `results` is present only on success. Each entry mirrors the
  `affected_rows` count `iii-database::execute` would have produced for
  that statement, plus a positional `rows` array (NOT keyed by column —
  this is intentionally lighter than `query`'s row-of-objects shape;
  parse with the input order in mind).
- `failed_index` is present only when the failing error carries a
  per-statement index — i.e. a `DRIVER_ERROR` thrown by one of the
  inputs. **Connection-level failures leave it absent**: `POOL_TIMEOUT`
  on acquire, `UNKNOWN_DB`, a `BEGIN` that fails, or any error that
  isn't tied to a specific input statement. Treat absence as "the
  transaction never started" or "the failure spans the batch", not as
  "step 0 failed".
- `error` is the same JSON-tagged `DbError` returned elsewhere on
  driver-level failure (`code` is the stable discriminant, `driver` /
  `inner_code` / `message` are diagnostic).

# Worked example

Two related INSERTs that must land together:

```json
{
  "db": "primary",
  "statements": [
    { "sql": "INSERT INTO accounts (id, balance) VALUES (?, ?)", "params": [1, 100] },
    { "sql": "UPDATE accounts SET balance = balance - ? WHERE id = ?", "params": [10, 1] },
    { "sql": "UPDATE accounts SET balance = balance + ? WHERE id = ?", "params": [10, 2] }
  ],
  "isolation": "serializable"
}
```

Returns `{ "committed": true, "results": [...] }` with `affected_rows`
populated per step.

If the third UPDATE hits a `NOT NULL` constraint failure, the response
becomes:

```json
{
  "committed":    false,
  "failed_index": 2,
  "error": { "code": "DRIVER_ERROR", "driver": "sqlite", "message": "...", "failed_index": 2 }
}
```

The first two statements rolled back; no row in `accounts` reflects the
debit, no row reflects the credit, and the original INSERT did not
persist. Re-issue the entire transaction call (don't retry only the
failed step) once the underlying constraint condition is fixed.

# Related

- [`iii-database::beginTransaction` + `transactionQuery` / `transactionExecute` + `commitTransaction` / `rollbackTransaction`](iii://iii-database/interactive-transaction) — **stateful interactive** transaction with a configurable timeout-driven auto-rollback. Use this surface when you need to take a decision in application code *between* statements (read-your-writes across round-trips). The batch handler on this page requires every statement up-front, so it can't carry that inter-statement logic.
- `iii-database::execute` — single-statement variant; skips the BEGIN/COMMIT framing for one-shot writes.
- `iii-database::query` — read-only; cannot be combined with writes inside this surface but is fine to mix into the `statements` array if you only need its rows for `affected_rows`-equivalent counts.
- `iii-database::prepareStatement` + `iii-database::runStatement` — for repeating one parameterized statement many times; not a substitute for atomic multi-statement commit.

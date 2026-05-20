import type { TestCase } from './cases.ts'
import { expect, expectEqual } from './cases.ts'

/**
 * Side-channel finalization bypass repros.
 *
 * Each case demonstrates a separate way the
 * `transactionExecute`/`transactionQuery` defense in
 * `database/src/handlers/transaction_execute.rs::is_transaction_control_sql`
 * can be bypassed to finalize an interactive transaction without going through
 * `commitTransaction` / `rollbackTransaction`.
 *
 * Each case is structured as:
 *   - drive the worker through a sequence that SHOULD be rejected with
 *     `INVALID_PARAM` (per the docstring on `transactionExecute`)
 *   - if the worker rejects → PASS (defense holds, fix shipped)
 *   - if the worker accepts AND the transaction-registry/conn state desyncs
 *     → FAIL with a label describing the observed bypass
 *
 * Run with: `./run-tests.sh --bypass-only`
 *
 * Findings (see /review output on branch feat/database-and-skills):
 *   1. comment-prefix in transactionExecute  (handlers/transaction_execute.rs:42-63)
 *   2. transactionQuery missing the guard    (handlers/transaction_query.rs)
 *   3. `START TRANSACTION` not in keyword set (handlers/transaction_execute.rs:55, MySQL-specific)
 *
 * These cases will FAIL until the fix is shipped. After the fix, they MUST
 * pass on every driver they apply to.
 */
export const TX_CONTROL_BYPASS_CASES: TestCase[] = [
  {
    // Finding #1: `/* ... */COMMIT` — the first whitespace-delimited token is
    // `/*`, never matches the COMMIT keyword, the SQL is passed to the driver,
    // and PG/MySQL strip server-side comments and execute the COMMIT. SQLite
    // does the same. The registry still tracks the txn handle; subsequent
    // calls see TRANSACTION_NOT_FOUND only after the watcher sweep removes
    // the entry (1+ second later), or — worse — succeed against a non-tx conn
    // until the watcher fires.
    name: 'bypass #1: transactionExecute /* */COMMIT must be rejected',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1)
      const table = 'bypass_comment_commit'
      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${table} (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
      })

      const begin = await call('database::beginTransaction', { db: driver })
      const id = begin.transaction.id

      try {
        // Stage a row inside the txn. Visible from inside, not yet committed.
        await call('database::transactionExecute', {
          transaction_id: id,
          sql: `INSERT INTO ${table} (n) VALUES (${ph1})`,
          params: [1],
        })

        // The bypass attempt. The defense MUST reject this — the user must
        // be told to use `commitTransaction`.
        let rejected = false
        let acceptedErr: unknown = null
        try {
          await call('database::transactionExecute', {
            transaction_id: id,
            sql: '/* sneak */COMMIT',
          })
        } catch (e: any) {
          const msg = e?.message ?? String(e)
          if (msg.includes('INVALID_PARAM')) {
            rejected = true
          } else {
            acceptedErr = e
          }
        }

        if (!rejected) {
          // The worker accepted the SQL. Now prove it desynced the state.
          // Read from outside the txn — if the COMMIT landed at the driver
          // level, the staged row is visible from a fresh connection.
          const outside = await call('database::query', {
            db: driver,
            sql: `SELECT COUNT(*) AS c FROM ${table}`,
          })
          const leakedCount = Number(outside.rows[0].c)
          throw new Error(
            `BYPASS CONFIRMED [#1 comment-prefix]: '/* */COMMIT' was accepted ` +
              `by transactionExecute; outside-tx COUNT=${leakedCount} ` +
              `(>0 means the side-channel COMMIT landed at the driver, ` +
              `desyncing the registry from the connection state)` +
              (acceptedErr ? ` — non-INVALID_PARAM error: ${acceptedErr}` : ''),
          )
        }
        expect(rejected, 'transactionExecute must reject /* */COMMIT with INVALID_PARAM')
      } finally {
        try {
          await call('database::rollbackTransaction', { transaction_id: id })
        } catch {
          /* may already be gone if the bypass succeeded */
        }
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  },
  {
    // Finding #2: `transactionQuery` has NO transaction-control filter at all.
    // PG/MySQL/SQLite all happily execute COMMIT through the prepared-statement
    // path that `run_prepared` uses. Once COMMIT lands, the registry still
    // holds the entry — subsequent calls operate on a now-non-transactional
    // pinned connection until the watcher sweeps.
    name: 'bypass #2: transactionQuery COMMIT must be rejected',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1)
      const table = 'bypass_query_commit'
      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${table} (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
      })

      const begin = await call('database::beginTransaction', { db: driver })
      const id = begin.transaction.id

      try {
        await call('database::transactionExecute', {
          transaction_id: id,
          sql: `INSERT INTO ${table} (n) VALUES (${ph1})`,
          params: [2],
        })

        let rejected = false
        let acceptedErr: unknown = null
        try {
          await call('database::transactionQuery', {
            transaction_id: id,
            sql: 'COMMIT',
          })
        } catch (e: any) {
          const msg = e?.message ?? String(e)
          if (msg.includes('INVALID_PARAM')) {
            rejected = true
          } else {
            acceptedErr = e
          }
        }

        if (!rejected) {
          const outside = await call('database::query', {
            db: driver,
            sql: `SELECT COUNT(*) AS c FROM ${table}`,
          })
          const leakedCount = Number(outside.rows[0].c)
          throw new Error(
            `BYPASS CONFIRMED [#2 transactionQuery]: 'COMMIT' was accepted by ` +
              `transactionQuery (no is_transaction_control_sql guard exists ` +
              `on the query path); outside-tx COUNT=${leakedCount} ` +
              `(>0 means the side-channel COMMIT landed at the driver)` +
              (acceptedErr ? ` — non-INVALID_PARAM error: ${acceptedErr}` : ''),
          )
        }
        expect(rejected, 'transactionQuery must reject COMMIT with INVALID_PARAM')
      } finally {
        try {
          await call('database::rollbackTransaction', { transaction_id: id })
        } catch {
          /* ignore */
        }
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  },
  {
    // Finding #3: `START TRANSACTION` is the MySQL/ANSI SQL-92 spelling of
    // `BEGIN`. The keyword set in `is_transaction_control_sql` only catches
    // `BEGIN`, not `START`. On MySQL, `START TRANSACTION` while inside an
    // existing txn implicitly COMMITs the in-progress one and opens a new
    // one — directly breaking the atomicity contract.
    //
    // On Postgres, `START TRANSACTION` inside a txn issues a NOTICE and
    // no-ops, so the worst case is "user is mildly confused" rather than
    // "registry desyncs"; we still expect the worker to reject for
    // consistency with how `BEGIN` is handled. Gated to mysql_db where the
    // observable atomicity violation lives.
    name: 'bypass #3: transactionExecute START TRANSACTION must be rejected (mysql)',
    applies: ['mysql_db'],
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1)
      const table = 'bypass_start_transaction'
      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${table} (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
      })

      const begin = await call('database::beginTransaction', { db: driver })
      const id = begin.transaction.id

      try {
        // Stage a row in the OUTER txn that — on a healthy worker — would be
        // discarded when we `rollbackTransaction` below. If `START TRANSACTION`
        // is accepted, MySQL implicitly COMMITs that row before opening the
        // new (untracked) txn, leaking the row past the eventual rollback.
        await call('database::transactionExecute', {
          transaction_id: id,
          sql: `INSERT INTO ${table} (n) VALUES (${ph1})`,
          params: [3],
        })

        let rejected = false
        let acceptedErr: unknown = null
        try {
          await call('database::transactionExecute', {
            transaction_id: id,
            sql: 'START TRANSACTION',
          })
        } catch (e: any) {
          const msg = e?.message ?? String(e)
          if (msg.includes('INVALID_PARAM')) {
            rejected = true
          } else {
            acceptedErr = e
          }
        }

        if (!rejected) {
          // Roll back the (now-second) txn the user thinks they're in.
          try {
            await call('database::rollbackTransaction', { transaction_id: id })
          } catch {
            /* ignore */
          }

          const outside = await call('database::query', {
            db: driver,
            sql: `SELECT COUNT(*) AS c FROM ${table}`,
          })
          const leakedCount = Number(outside.rows[0].c)
          throw new Error(
            `BYPASS CONFIRMED [#3 START TRANSACTION on MySQL]: ` +
              `'START TRANSACTION' was accepted by transactionExecute; ` +
              `outside-tx COUNT=${leakedCount} after a rollback that should ` +
              `have undone the staged INSERT (>0 means MySQL implicitly ` +
              `COMMITed the outer txn when START TRANSACTION ran)` +
              (acceptedErr ? ` — non-INVALID_PARAM error: ${acceptedErr}` : ''),
          )
        }
        expect(rejected, 'transactionExecute must reject START TRANSACTION with INVALID_PARAM')
      } finally {
        try {
          await call('database::rollbackTransaction', { transaction_id: id })
        } catch {
          /* may already be gone */
        }
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  },
  {
    // Extra repro: `--COMMIT` (line-comment-prefixed) — same shape as #1 but
    // using `--` instead of `/* */`. The first token after `trim_start_matches`
    // is `--COMMIT`, doesn't match the keyword set, falls through. Postgres
    // strips line comments server-side and executes COMMIT.
    name: 'bypass #1b: transactionExecute --COMMIT (line-comment) must be rejected',
    // Skipped on sqlite_db: rusqlite's prepare on a sql string consisting of
    // only `--text\n…COMMIT` may interpret the line-comment + COMMIT as a
    // single statement OR raise its multi-statement guard; behavior is less
    // predictable. PG/MySQL both reliably strip the line comment.
    applies: ['pg_db', 'mysql_db'],
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1)
      const table = 'bypass_line_comment'
      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${table} (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
      })

      const begin = await call('database::beginTransaction', { db: driver })
      const id = begin.transaction.id

      try {
        await call('database::transactionExecute', {
          transaction_id: id,
          sql: `INSERT INTO ${table} (n) VALUES (${ph1})`,
          params: [4],
        })

        let rejected = false
        let acceptedErr: unknown = null
        try {
          // Newline is required after `--` so the server sees COMMIT as a
          // statement; otherwise the comment swallows the rest of the line.
          await call('database::transactionExecute', {
            transaction_id: id,
            sql: '-- sneak\nCOMMIT',
          })
        } catch (e: any) {
          const msg = e?.message ?? String(e)
          if (msg.includes('INVALID_PARAM')) {
            rejected = true
          } else {
            acceptedErr = e
          }
        }

        if (!rejected) {
          const outside = await call('database::query', {
            db: driver,
            sql: `SELECT COUNT(*) AS c FROM ${table}`,
          })
          const leakedCount = Number(outside.rows[0].c)
          throw new Error(
            `BYPASS CONFIRMED [#1b line-comment]: '-- sneak\\nCOMMIT' was ` +
              `accepted by transactionExecute; outside-tx COUNT=${leakedCount}` +
              (acceptedErr ? ` — non-INVALID_PARAM error: ${acceptedErr}` : ''),
          )
        }
        expectEqual(rejected, true, 'transactionExecute must reject --COMMIT')
      } finally {
        try {
          await call('database::rollbackTransaction', { transaction_id: id })
        } catch {
          /* ignore */
        }
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  },
]

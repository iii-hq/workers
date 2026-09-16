/**
 * Postgres regression cases for the RETURNING routing fix (MOT-4777).
 *
 * `execute` / `transaction` / `transactionExecute` used to decide whether a
 * statement produces rows by searching for the literal substring
 * `" RETURNING "` in the SQL text. Two failure shapes came out of that, and
 * both are contract-level — invisible to the caller, so they need a case
 * each, not just a unit test:
 *
 * - the keyword on its own line or beside a tab missed the check, the rows
 *   were fetched with `client.execute` (which discards them), and the caller
 *   saw `returned_rows: []` with no error;
 * - the keyword inside a string literal or comment matched, routing a plain
 *   write through the query path, which then reported `affected_rows: 0` for
 *   a write that had committed — and a zero-row change fires no
 *   `database::row-changed` event at all.
 *
 * The routing signal is the prepared statement's column list now, so these
 * cases pin both directions: rows come back when the clause is real, and the
 * event fires when it only looks real.
 */

import type { ISdk } from 'iii-sdk'
import type { TestCase } from './cases.ts'
import { expect, expectEqual } from './cases.ts'

const EVENT_TIMEOUT_MS = 5_000

interface RowChangedEvent {
  db: string
  table: string | null
  op: 'insert' | 'update' | 'delete' | 'other'
  affected_rows: number
  returning?: Record<string, unknown>[]
  truncated?: boolean
  at: number
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

function sink(events: RowChangedEvent[], label: string) {
  let cursor = 0
  return {
    async next(): Promise<RowChangedEvent> {
      const deadline = Date.now() + EVENT_TIMEOUT_MS
      while (events.length <= cursor && Date.now() < deadline) await sleep(20)
      if (events.length <= cursor) throw new Error(`${label}: event ${cursor + 1} was not delivered`)
      return events[cursor++]
    },
    expectDrained(): void {
      expectEqual(events.length, cursor, `${label}: unexpected extra event`)
    },
  }
}

async function awaitRegistration(iii: ISdk, functionId: string): Promise<void> {
  const registered = await iii.trigger<
    Record<string, never>,
    { registered_triggers: Array<{ trigger_type: string; function_id: string }> }
  >({ function_id: 'engine::registered-triggers::list', payload: {} })
  expect(
    registered.registered_triggers.some(
      (t) => t.trigger_type === 'database::row-changed' && t.function_id === functionId,
    ),
    `binding for ${functionId} is visible to the engine`,
  )
}

const TABLE = 'e2e_pg_routing'

export const PG_ROUTING_CASES: TestCase[] = [
  {
    name: 'pg returns rows for RETURNING split across lines, tabs, and comments',
    applies: ['pg_db'],
    async run({ driver, dialect, call }) {
      const ph = dialect.placeholder(1)
      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${TABLE} (id SERIAL PRIMARY KEY, n INT)`,
      })
      let activeTransaction: string | undefined
      try {
        // The keyword on its own line: pre-fix this fell through to
        // `client.execute`, and the rows died there without an error.
        const direct = await call('database::execute', {
          db: driver,
          sql: `INSERT INTO ${TABLE} (n) VALUES (${ph})\nRETURNING\nid, n`,
          params: [7],
        })
        expectEqual(direct.affected_rows, 1, 'direct: one row changed')
        expectEqual(direct.returned_rows.length, 1, 'direct: the RETURNING row came back')
        expectEqual(Number(direct.returned_rows[0].n), 7, 'direct: the projected value')
        expect(
          typeof direct.last_insert_id === 'string' && direct.last_insert_id.length > 0,
          'direct: first RETURNING column is the insert id',
        )

        // Tab-separated, inside an atomic batch.
        const batch = await call('database::transaction', {
          db: driver,
          statements: [
            { sql: `INSERT INTO ${TABLE} (n) VALUES (${ph})\tRETURNING\tid, n`, params: [8] },
          ],
        })
        expectEqual(batch.committed, true, 'batch committed')
        expectEqual(batch.results[0].rows.length, 1, 'batch step returned its row')

        // A comment between keyword and projection, inside an interactive
        // transaction.
        activeTransaction = (await call('database::beginTransaction', { db: driver })).transaction.id
        const staged = await call('database::transactionExecute', {
          transaction_id: activeTransaction,
          sql: `INSERT INTO ${TABLE} (n) VALUES (${ph}) RETURNING/*key*/id, n`,
          params: [9],
        })
        expectEqual(staged.returned_rows.length, 1, 'interactive step returned its row')
        await call('database::commitTransaction', { transaction_id: activeTransaction })
        activeTransaction = undefined

        // Each statement ran exactly once.
        const count = await call('database::query', {
          db: driver,
          sql: `SELECT COUNT(*) AS c FROM ${TABLE}`,
        })
        expectEqual(Number(count.rows[0].c), 3, 'three writes, no duplicates')
      } finally {
        if (activeTransaction) {
          try {
            await call('database::rollbackTransaction', { transaction_id: activeTransaction })
          } catch {
            /* already finalized */
          }
        }
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      }
    },
  },
  {
    name: 'pg write whose text merely mentions RETURNING is a plain write that fires',
    applies: ['pg_db'],
    async run({ driver, dialect, call, iii }) {
      const functionId = 'harness::pg_routing_literal'
      const events: RowChangedEvent[] = []
      const seen = sink(events, 'literal-keyword subscriber')
      const ph = dialect.placeholder(1)

      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${TABLE} (id SERIAL PRIMARY KEY, note TEXT)`,
      })
      const fnRef = iii.registerFunction(
        functionId,
        async (payload: RowChangedEvent) => {
          events.push(payload)
          return null
        },
        { description: 'Literal-keyword routing E2E sink.' },
      )
      const triggerRef = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: functionId,
        config: { db: driver, table: TABLE },
      })
      try {
        await awaitRegistration(iii, functionId)

        // The word inside a string literal: pre-fix this routed through the
        // query path, reported affected_rows: 0, and fired nothing.
        const literal = await call('database::execute', {
          db: driver,
          sql: `INSERT INTO ${TABLE} (note) VALUES ('please RETURNING soon')`,
        })
        expectEqual(literal.affected_rows, 1, 'literal: the write reports its row')
        expectEqual(literal.returned_rows.length, 0, 'literal: no rows materialise')
        const literalEvent = await seen.next()
        expectEqual(literalEvent.op, 'insert', 'literal: the change is announced')
        expectEqual(literalEvent.affected_rows, 1, 'literal: with the real count')
        expect(literalEvent.returning === undefined, 'literal: no clause, no projection')

        // …and the same inside a trailing comment.
        const commented = await call('database::execute', {
          db: driver,
          sql: `INSERT INTO ${TABLE} (note) VALUES (${ph}) /* RETURNING id */`,
          params: ['x'],
        })
        expectEqual(commented.affected_rows, 1, 'comment: the write reports its row')
        expectEqual(commented.returned_rows.length, 0, 'comment: no rows materialise')
        const commentEvent = await seen.next()
        expectEqual(commentEvent.op, 'insert', 'comment: the change is announced')

        await sleep(500)
        seen.expectDrained()
      } finally {
        triggerRef.unregister()
        fnRef.unregister()
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      }
    },
  },
  {
    name: 'pg select and values through execute return rows without an insert id',
    applies: ['pg_db'],
    async run({ driver, dialect, call }) {
      const ph = dialect.placeholder(1)
      const sel = await call('database::execute', {
        db: driver,
        sql: `SELECT ${ph}::int AS n`,
        params: [42],
      })
      expectEqual(sel.returned_rows.length, 1, 'SELECT through execute returns its row')
      expectEqual(Number(sel.returned_rows[0].n), 42, 'the selected value')
      expect(
        sel.last_insert_id === null || sel.last_insert_id === undefined,
        `a SELECT's first cell is not an insert id (got ${JSON.stringify(sel.last_insert_id)})`,
      )

      const values = await call('database::execute', { db: driver, sql: 'VALUES (10), (20), (30)' })
      expectEqual(values.affected_rows, 3, 'VALUES reports three rows')
      expectEqual(values.returned_rows.length, 3, 'VALUES returns its rows')
      expect(
        values.last_insert_id === null || values.last_insert_id === undefined,
        'VALUES is not an insert either',
      )
    },
  },
  {
    name: 'pg batch failure reports its step and leaves the pool healthy',
    applies: ['pg_db'],
    async run({ driver, call }) {
      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      await call('database::execute', { db: driver, sql: `CREATE TABLE ${TABLE} (n INT)` })
      try {
        const failed = await call('database::transaction', {
          db: driver,
          statements: [
            { sql: `INSERT INTO ${TABLE} (n) VALUES (1)` },
            { sql: `INSERT INTO ${TABLE}_missing (n) VALUES (2)` },
          ],
        })
        expectEqual(failed.committed, false, 'the batch did not commit')
        expectEqual(failed.failed_index, 1, 'failed_index names the second statement')
        expect(typeof failed.error === 'object' && failed.error !== null, 'a structured error')

        // The first statement rolled back with the batch…
        const count = await call('database::query', {
          db: driver,
          sql: `SELECT COUNT(*) AS c FROM ${TABLE}`,
        })
        expectEqual(Number(count.rows[0].c), 0, 'first statement rolled back')

        // …and the connection the failure left behind is still usable.
        const after = await call('database::query', { db: driver, sql: 'SELECT 7 AS ok' })
        expectEqual(Number(after.rows[0].ok), 7, 'the pool keeps serving after the rollback')
      } finally {
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      }
    },
  },
]

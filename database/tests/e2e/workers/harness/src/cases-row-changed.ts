import type { TestCase } from './cases.ts'
import { expect, expectEqual, observeRowChanged, ROW_CHANGED_FN } from './cases.ts'

const EVENT_TIMEOUT_MS = 5_000
const SILENCE_WINDOW_MS = 500

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

export const ROW_CHANGED_CASES: TestCase[] = [
  {
    // What an agent reads before calling: the description (its first
    // sentence is all directory::search_functions shows) and the request
    // schema (engine::functions::info) must both say that rows come from a
    // RETURNING clause in the SQL — the `returning` option never adds one.
    // Driver-independent; runs once.
    name: 'execute contract tells callers where returned rows come from',
    applies: ['sqlite_db'],
    async run({ iii }) {
      for (const functionId of ['database::execute', 'database::transactionExecute']) {
        const info = await iii.call<{ function_id: string }, any>({
          function_id: 'engine::functions::info',
          payload: { function_id: functionId },
        })
        expect(
          typeof info.description === 'string' && info.description.includes('RETURNING'),
          `${functionId}: description names RETURNING (got ${JSON.stringify(info.description)})`,
        )
        const schema = info.request_format ?? info.request_schema
        const props = schema?.properties ?? {}
        expect(
          String(props.sql?.description ?? '').includes('RETURNING'),
          `${functionId}: the sql field explains the RETURNING clause`,
        )
        expect(
          String(props.returning?.description ?? '').includes('never adds'),
          `${functionId}: the returning option says it never adds the clause`,
        )
        expect(
          !(schema?.required ?? []).includes('returning'),
          `${functionId}: the returning option stays optional`,
        )
      }
    },
  },
  {
    // An observer names the table canonically (bare, unquoted, lowercase):
    // any other spelling is refused when the observer is created, with the
    // spelling to use, instead of matching nothing.
    name: 'row-changed observer must name the canonical table',
    async run({ driver, dialect, call, iii }) {
      const table = 'e2e_row_changed_case'
      const functionId = `harness::row_changed_case_${driver}`
      const events: RowChangedEvent[] = []

      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${table} (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
      })
      const fnRef = iii.registerFunction(
        functionId,
        async (payload: RowChangedEvent) => {
          events.push(payload)
          return null
        },
        { description: 'Canonical table args E2E sink.' },
      )

      const refused = observeRowChanged(iii, { db: driver, table: table.toUpperCase() }, functionId)
      const observer = observeRowChanged(iii, { db: driver, table }, functionId)
      try {
        let refusal: unknown
        try {
          await refused.ready()
        } catch (e) {
          refusal = e
        }
        const message = String((refusal as { message?: string } | undefined)?.message ?? refusal)
        expectEqual(
          (refusal as { code?: string } | undefined)?.code,
          'NON_CANONICAL_TABLE',
          `uppercase table is refused (got ${message})`,
        )
        expect(message.includes(`use \`${table}\``), `the refusal names the canonical table: ${message}`)

        await observer.ready()
        await call('database::execute', {
          db: driver,
          sql: `INSERT INTO ${table} (n) VALUES (${dialect.placeholder(1)})`,
          params: [1],
        })
        const deadline = Date.now() + EVENT_TIMEOUT_MS
        while (events.length === 0 && Date.now() < deadline) await sleep(20)
        expectEqual(events.length, 1, 'the canonical observer hears the insert')
        expectEqual(events[0].op, 'insert', 'canonical observer op')
        expectEqual(events[0].table, table, 'the event names the canonical table')
      } finally {
        await refused.dispose().catch(() => {})
        await observer.dispose()
        fnRef.unregister()
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  },
  {
    // The function is private: only the database worker calls it. A call
    // from any other worker is refused by the engine, and observers hear
    // nothing, so no worker can raise a row change of its own making.
    // Driver-independent; runs once.
    name: 'row-changed refuses a call from another worker',
    applies: ['sqlite_db'],
    async run({ driver, iii }) {
      const functionId = `harness::row_changed_forged_${driver}`
      const events: RowChangedEvent[] = []
      const fnRef = iii.registerFunction(
        functionId,
        async (payload: RowChangedEvent) => {
          events.push(payload)
          return null
        },
        { description: 'Forged row-changed E2E sink.' },
      )
      const observer = observeRowChanged(iii, { db: driver }, functionId)
      try {
        await observer.ready()
        let refusal: unknown
        try {
          await iii.call({
            function_id: ROW_CHANGED_FN,
            payload: {
              db: driver,
              schema: null,
              table: 'forged',
              op: 'delete',
              affected_rows: 999,
              at: Date.now(),
            },
          })
        } catch (e) {
          refusal = e
        }
        expect(refusal !== undefined, 'a call from another worker is refused')
        expect(String((refusal as Error).message ?? refusal).includes('FORBIDDEN'), `refused as FORBIDDEN: ${refusal}`)
        await sleep(SILENCE_WINDOW_MS)
        expectEqual(events.length, 0, 'no observer hears the forged event')
      } finally {
        await observer.dispose()
        fnRef.unregister()
      }
    },
  },
  {
    name: 'row-changed filters ops and emits committed mutations only',
    async run({ driver, dialect, call, iii }) {
      const table = 'e2e_row_changed'
      const functionId = `harness::row_changed_${driver}`
      const insertFunctionId = `${functionId}_inserts`
      const events: RowChangedEvent[] = []
      const insertEvents: RowChangedEvent[] = []
      let cursor = 0
      let insertCursor = 0
      let activeTransaction: string | undefined

      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${table} (id ${dialect.idColumnDDL()}, n INT NOT NULL)`,
      })

      const functionRef = iii.registerFunction(
        functionId,
        async (payload: RowChangedEvent) => {
          events.push(payload)
          return null
        },
        { description: 'Database row-changed E2E event sink.' },
      )
      const insertFunctionRef = iii.registerFunction(
        insertFunctionId,
        async (payload: RowChangedEvent) => {
          insertEvents.push(payload)
          return null
        },
        { description: 'Database row-changed insert-only E2E event sink.' },
      )
      const observer = observeRowChanged(iii, { db: driver, table }, functionId)
      const insertObserver = observeRowChanged(iii, { db: driver, table, ops: ['insert'] }, insertFunctionId)

      const nextEvent = async (): Promise<RowChangedEvent> => {
        const deadline = Date.now() + EVENT_TIMEOUT_MS
        while (events.length <= cursor && Date.now() < deadline) await sleep(20)
        if (events.length <= cursor) throw new Error(`row-changed event ${cursor + 1} was not delivered`)
        return events[cursor++]
      }
      const nextInsertEvent = async (): Promise<RowChangedEvent> => {
        const deadline = Date.now() + EVENT_TIMEOUT_MS
        while (insertEvents.length <= insertCursor && Date.now() < deadline) await sleep(20)
        if (insertEvents.length <= insertCursor) {
          throw new Error(`insert-filtered row-changed event ${insertCursor + 1} was not delivered`)
        }
        return insertEvents[insertCursor++]
      }
      const expectSilence = async (): Promise<void> => {
        await sleep(SILENCE_WINDOW_MS)
        expectEqual(events.length, cursor, 'row-changed emitted an unexpected event')
        expectEqual(
          insertEvents.length,
          insertCursor,
          'insert-filtered row-changed emitted an unexpected event',
        )
      }
      const expectEvent = (event: RowChangedEvent, op: RowChangedEvent['op'], affectedRows = 1): void => {
        expectEqual(event.db, driver, 'row-changed db')
        expectEqual(event.table, table, 'row-changed table')
        expectEqual(event.op, op, 'row-changed op')
        expectEqual(event.affected_rows, affectedRows, 'row-changed affected_rows')
        expect(Number.isFinite(event.at) && event.at > 0, 'row-changed at is an epoch timestamp')
      }

      try {
        // Active once the worker accepted each observer: the next commit
        // reaches both.
        await Promise.all([observer.ready(), insertObserver.ready()])

        const p1 = dialect.placeholder(1)
        const p2 = dialect.placeholder(2)
        // The rows on a statements-path event are the writer's own RETURNING
        // projection, and that clause lives in the SQL — the `returning`
        // OPTION never adds it. MySQL has no RETURNING, so its events are
        // count-only. Nothing here is ever truncated: the cap is native's.
        const hasReturning = driver !== 'mysql_db'
        const returningSql = hasReturning ? ' RETURNING id, n' : ''
        const expectProjection = (event: RowChangedEvent, rows: unknown, label: string): void => {
          if (hasReturning) {
            expectEqual(event.returning, rows, `${label}: the event carries the writer's RETURNING rows`)
          } else {
            expect(event.returning === undefined, `${label}: no RETURNING on mysql, so no rows on the event`)
          }
          expect(event.truncated === undefined, `${label}: statements-path events never truncate`)
        }
        // MySQL permits INSERT without INTO; using that form here also pins
        // the classifier regression while SQLite/PostgreSQL use standard SQL.
        const insertPrefix = driver === 'mysql_db' ? 'INSERT' : 'INSERT INTO'

        // The clause alone, no `returning` option: the rows still come back
        // to the caller and onto the event, identical.
        const direct = await call('database::execute', {
          db: driver,
          sql: `${insertPrefix} ${table} (n) VALUES (${p1})${returningSql}`,
          params: [10],
        })
        const inserted = await nextEvent()
        expectEvent(inserted, 'insert')
        expectEvent(await nextInsertEvent(), 'insert')
        expectProjection(inserted, direct.returned_rows, 'direct insert')
        if (hasReturning) {
          expectEqual(Number(inserted.returning?.[0]?.n), 10, 'row-changed direct RETURNING value')
        }

        await call('database::transaction', {
          db: driver,
          statements: [
            {
              sql: `INSERT INTO ${table} (n) VALUES (${p1})${returningSql}`,
              params: [15],
            },
          ],
        })
        const atomic = await nextEvent()
        expectEvent(atomic, 'insert')
        expectEvent(await nextInsertEvent(), 'insert')
        if (hasReturning) {
          expectEqual(Number(atomic.returning?.[0]?.n), 15, 'row-changed atomic RETURNING value')
        }
        expect(atomic.truncated === undefined, 'atomic batch events never truncate')

        // No clause: the change is announced, but nothing says which row.
        await call('database::execute', {
          db: driver,
          sql: `UPDATE ${table} SET n = ${p1} WHERE n = ${p2}`,
          params: [11, 10],
        })
        const updated = await nextEvent()
        expectEvent(updated, 'update')
        expect(updated.returning === undefined, 'no RETURNING clause, no rows on the update event')
        expect(updated.truncated === undefined, 'update event is not truncated')

        await call('database::execute', {
          db: driver,
          sql: `DELETE FROM ${table} WHERE n = ${p1}`,
          params: [11],
        })
        const deleted = await nextEvent()
        expectEvent(deleted, 'delete')
        expect(deleted.returning === undefined, 'no RETURNING clause, no rows on the delete event')

        await call('database::execute', {
          db: driver,
          sql: `UPDATE ${table} SET n = ${p1} WHERE n = ${p2}`,
          params: [99, -1],
        })
        await expectSilence()

        // The option alongside the clause is harmless — sqlite validates it,
        // postgres and mysql ignore it — and the rows are still the SQL's.
        activeTransaction = (await call('database::beginTransaction', { db: driver })).transaction.id
        const staged = await call('database::transactionExecute', {
          transaction_id: activeTransaction,
          sql: `INSERT INTO ${table} (n) VALUES (${p1})${returningSql}`,
          params: [20],
          returning: hasReturning ? ['id', 'n'] : [],
        })
        await expectSilence()
        await call('database::commitTransaction', { transaction_id: activeTransaction })
        activeTransaction = undefined
        const committed = await nextEvent()
        expectEvent(committed, 'insert')
        expectEvent(await nextInsertEvent(), 'insert')
        expectProjection(committed, staged.returned_rows, 'committed insert')
        if (hasReturning) {
          expectEqual(Number(committed.returning?.[0]?.n), 20, 'row-changed committed RETURNING value')
        }

        activeTransaction = (await call('database::beginTransaction', { db: driver })).transaction.id
        await call('database::transactionExecute', {
          transaction_id: activeTransaction,
          sql: `INSERT INTO ${table} (n) VALUES (${p1})`,
          params: [30],
        })
        await call('database::rollbackTransaction', { transaction_id: activeTransaction })
        activeTransaction = undefined
        await expectSilence()
      } finally {
        if (activeTransaction) {
          try {
            await call('database::rollbackTransaction', { transaction_id: activeTransaction })
          } catch {
            /* transaction may already be finalized */
          }
        }
        await insertObserver.dispose()
        insertFunctionRef.unregister()
        await observer.dispose()
        functionRef.unregister()
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  },
  {
    // Known classifier quirk, pinned on purpose: `sql::classify`
    // (row_changes/sql.rs) reads keywords, not a parse, so a CTE statement whose
    // TEXT contains a DML keyword — even inside a string literal — is
    // reported as an unknown write. The README documents this. If someone
    // teaches the classifier about literals, this case is the deliberate
    // record of the old contract and should change with it.
    //
    // MySQL is excluded: a SELECT through `execute` reports no affected rows
    // there (the result set is dropped), so nothing fires at all.
    name: 'row-changed reports a CTE read whose text mentions a DML word',
    applies: ['sqlite_db', 'pg_db'],
    async run({ driver, call, iii }) {
      const functionId = `harness::row_changed_cte_${driver}`
      const events: RowChangedEvent[] = []
      const fnRef = iii.registerFunction(
        functionId,
        async (payload: RowChangedEvent) => {
          events.push(payload)
          return null
        },
        { description: 'CTE-classification E2E sink.' },
      )
      // db-wide: a statement the classifier cannot pin to a table only
      // reaches observers that did not name one.
      const observer = observeRowChanged(iii, { db: driver }, functionId)
      try {
        await observer.ready()
        const read = await call('database::execute', {
          db: driver,
          sql: `WITH x AS (SELECT 1 AS n) SELECT 'update' AS note FROM x`,
        })
        expectEqual(read.returned_rows.length, 1, 'the read still returns its row')

        const deadline = Date.now() + EVENT_TIMEOUT_MS
        while (events.length === 0 && Date.now() < deadline) await sleep(20)
        expectEqual(events.length, 1, 'the keyword in the text fires one event')
        const event = events[0]
        expectEqual(event.op, 'other', 'unknown write op')
        expectEqual(event.table, null, 'the classifier will not guess a table')
        expectEqual(event.affected_rows, 1, 'the statement count rides along')
        expectEqual(
          event.returning,
          [{ note: 'update' }],
          "statements mode forwards the writer's rows, reads included",
        )
      } finally {
        await observer.dispose()
        fnRef.unregister()
      }
    },
  },
]

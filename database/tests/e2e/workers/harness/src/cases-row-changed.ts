import type { TestCase } from './cases.ts'
import { expect, expectEqual } from './cases.ts'

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
        const info = await iii.trigger<{ function_id: string }, any>({
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
    // A subscriber should not have to guess how the writer spells the
    // table: bindings match case-insensitively (and ignoring any schema
    // qualifier) on the statements path.
    name: 'row-changed table filter matches case-insensitively',
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
        { description: 'Case-insensitive table filter E2E sink.' },
      )
      const triggerRef = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: functionId,
        config: { db: driver, table: table.toUpperCase() },
      })

      try {
        // Registration propagates asynchronously; write only once the engine
        // can see the binding, or a slow ack fails the case on timeout
        // rather than on the behavior under test.
        const registered = await iii.trigger<
          Record<string, never>,
          { registered_triggers: Array<{ trigger_type: string; function_id: string }> }
        >({ function_id: 'engine::registered-triggers::list', payload: {} })
        expect(
          registered.registered_triggers.some(
            (t) => t.trigger_type === 'database::row-changed' && t.function_id === functionId,
          ),
          'case-insensitive binding is visible to the engine',
        )

        await call('database::execute', {
          db: driver,
          sql: `INSERT INTO ${table} (n) VALUES (${dialect.placeholder(1)})`,
          params: [1],
        })
        const deadline = Date.now() + EVENT_TIMEOUT_MS
        while (events.length === 0 && Date.now() < deadline) await sleep(20)
        expectEqual(events.length, 1, 'uppercase binding hears the lowercase table')
        expectEqual(events[0].op, 'insert', 'case-insensitive match op')
      } finally {
        triggerRef.unregister()
        fnRef.unregister()
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
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
      const triggerRef = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: functionId,
        config: { db: driver, table },
      })
      const insertFunctionRef = iii.registerFunction(
        insertFunctionId,
        async (payload: RowChangedEvent) => {
          insertEvents.push(payload)
          return null
        },
        { description: 'Database row-changed insert-only E2E event sink.' },
      )
      const insertTriggerRef = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: insertFunctionId,
        config: { db: driver, table, ops: ['insert'] },
      })

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
        const registered = await iii.trigger<
          Record<string, never>,
          { registered_triggers: Array<{ trigger_type: string; function_id: string }> }
        >({ function_id: 'engine::registered-triggers::list', payload: {} })
        for (const expectedFunction of [functionId, insertFunctionId]) {
          expect(
            registered.registered_triggers.some(
              (trigger) =>
                trigger.trigger_type === 'database::row-changed' &&
                trigger.function_id === expectedFunction,
            ),
            `row-changed trigger registration for ${expectedFunction} is visible to the engine`,
          )
        }

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
        insertTriggerRef.unregister()
        insertFunctionRef.unregister()
        triggerRef.unregister()
        functionRef.unregister()
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  },
]

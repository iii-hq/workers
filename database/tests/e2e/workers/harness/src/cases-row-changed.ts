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
  at: number
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

export const ROW_CHANGED_CASES: TestCase[] = [
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
        const returning = driver === 'mysql_db' ? [] : ['id', 'n']
        const returningSql = returning.length > 0 ? ' RETURNING id, n' : ''
        // MySQL permits INSERT without INTO; using that form here also pins
        // the classifier regression while SQLite/PostgreSQL use standard SQL.
        const insertPrefix = driver === 'mysql_db' ? 'INSERT' : 'INSERT INTO'

        await call('database::execute', {
          db: driver,
          sql: `${insertPrefix} ${table} (n) VALUES (${p1})${returningSql}`,
          params: [10],
          returning,
        })
        const inserted = await nextEvent()
        expectEvent(inserted, 'insert')
        expectEvent(await nextInsertEvent(), 'insert')
        if (returning.length > 0) {
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
        if (returning.length > 0) {
          expectEqual(Number(atomic.returning?.[0]?.n), 15, 'row-changed atomic RETURNING value')
        }

        await call('database::execute', {
          db: driver,
          sql: `UPDATE ${table} SET n = ${p1} WHERE n = ${p2}`,
          params: [11, 10],
        })
        expectEvent(await nextEvent(), 'update')

        await call('database::execute', {
          db: driver,
          sql: `DELETE FROM ${table} WHERE n = ${p1}`,
          params: [11],
        })
        expectEvent(await nextEvent(), 'delete')

        await call('database::execute', {
          db: driver,
          sql: `UPDATE ${table} SET n = ${p1} WHERE n = ${p2}`,
          params: [99, -1],
        })
        await expectSilence()

        activeTransaction = (await call('database::beginTransaction', { db: driver })).transaction.id
        await call('database::transactionExecute', {
          transaction_id: activeTransaction,
          sql: `INSERT INTO ${table} (n) VALUES (${p1})${returningSql}`,
          params: [20],
          returning,
        })
        await expectSilence()
        await call('database::commitTransaction', { transaction_id: activeTransaction })
        activeTransaction = undefined
        const committed = await nextEvent()
        expectEvent(committed, 'insert')
        expectEvent(await nextInsertEvent(), 'insert')
        if (returning.length > 0) {
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

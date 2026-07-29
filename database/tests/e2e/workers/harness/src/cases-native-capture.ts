/**
 * Native change capture (`capture: native`, postgres only).
 *
 * `pg_native_db` points at the same postgres instance as `pg_db` but is
 * configured for trigger + LISTEN/NOTIFY capture. A write that enters the
 * database through the `pg_db` pool is — from `pg_native_db`'s perspective —
 * an external client: different pool, different connections, invisible to
 * SQL classification. If the native subscriber still hears it, the
 * cross-client claim holds end-to-end (DDL triggers → pg_notify → dedicated
 * LISTEN connection → bus → engine → harness).
 *
 * These cases carry `applies: ['pg_db']` so they run once, inside the
 * pg_db driver loop, and address `pg_native_db` explicitly in payloads.
 */

import type { TestCase } from './cases.ts'
import { expect, expectEqual } from './cases.ts'

const EVENT_TIMEOUT_MS = 5_000
const SILENCE_WINDOW_MS = 500
const NATIVE_DB = 'pg_native_db'

interface RowChangedEvent {
  db: string
  table: string | null
  op: 'insert' | 'update' | 'delete' | 'other'
  affected_rows: number
  returning?: Record<string, unknown>[]
  at: number
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

/**
 * Wait until the worker has installed the capture triggers for `table`.
 * Registration acks race the first write otherwise: NOTIFY is at-most-once,
 * so a write committed before CREATE TRIGGER lands is silently unheard.
 * The catalog is queried through the worker itself — no extra client needed.
 */
async function waitForCaptureTriggers(
  call: (functionId: string, payload: unknown) => Promise<any>,
  table: string,
): Promise<void> {
  const deadline = Date.now() + EVENT_TIMEOUT_MS
  for (;;) {
    const r = await call('database::query', {
      db: 'pg_db',
      // `$1::text::regclass`, not `$1::regclass` — a bare regclass cast makes
      // the driver bind the parameter AS regclass (22P03); text binds cleanly
      // and the server does the regclass conversion.
      sql: `SELECT count(*) AS n FROM pg_trigger WHERE tgrelid = $1::text::regclass AND tgname LIKE 'iii_row_changed_%'`,
      params: [table],
    })
    if (Number(r.rows?.[0]?.n) === 3) return
    if (Date.now() > deadline) {
      throw new Error(`capture triggers for ${table} were not installed within ${EVENT_TIMEOUT_MS}ms`)
    }
    await sleep(50)
  }
}

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

export const NATIVE_CAPTURE_CASES: TestCase[] = [
  {
    name: 'native capture hears writes from another client, own writes fire once',
    applies: ['pg_db'],
    async run({ call, iii }) {
      const table = 'e2e_native_capture'
      const nativeEvents: RowChangedEvent[] = []
      const classifiedEvents: RowChangedEvent[] = []
      const native = sink(nativeEvents, 'native subscriber')
      const classified = sink(classifiedEvents, 'classified subscriber')

      // The watched table must exist before the binding registers — the
      // worker installs the capture triggers at registration time.
      await call('database::execute', { db: NATIVE_DB, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: NATIVE_DB,
        sql: `CREATE TABLE ${table} (id BIGSERIAL PRIMARY KEY, n INT NOT NULL)`,
      })

      const nativeFn = iii.registerFunction(
        `harness::native_capture_events`,
        async (payload: RowChangedEvent) => {
          nativeEvents.push(payload)
          return null
        },
        { description: 'Native-capture E2E event sink.' },
      )
      const nativeTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: 'harness::native_capture_events',
        config: { db: NATIVE_DB, table },
      })
      // Statements-path subscriber on pg_db, watching the SAME physical
      // table. Proves the two capture modes coexist and attribute correctly.
      const classifiedFn = iii.registerFunction(
        `harness::native_capture_classified`,
        async (payload: RowChangedEvent) => {
          classifiedEvents.push(payload)
          return null
        },
        { description: 'Statements-path E2E event sink for the native-capture table.' },
      )
      const classifiedTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: 'harness::native_capture_classified',
        config: { db: 'pg_db', table },
      })

      const expectNative = (event: RowChangedEvent, op: RowChangedEvent['op'], affectedRows: number): void => {
        expectEqual(event.db, NATIVE_DB, 'native event db')
        // The database reports schema-qualified names; the worker forwards
        // them verbatim and bindings match ignoring the qualifier.
        expectEqual(event.table, `public.${table}`, 'native event table')
        expectEqual(event.op, op, 'native event op')
        expectEqual(event.affected_rows, affectedRows, 'native event affected_rows')
        expect(event.returning === undefined, 'native events carry no RETURNING rows')
        expect(Number.isFinite(event.at) && event.at > 0, 'native event at is an epoch timestamp')
      }

      try {
        // Wait until BOTH bindings are visible to the engine before writing.
        const registered = await call('engine::registered-triggers::list', {})
        for (const fn of ['harness::native_capture_events', 'harness::native_capture_classified']) {
          expect(
            registered.registered_triggers.some(
              (t: { trigger_type: string; function_id: string }) =>
                t.trigger_type === 'database::row-changed' && t.function_id === fn,
            ),
            `trigger registration for ${fn} is visible to the engine`,
          )
        }
        // …and until the worker's DDL install has landed in the database.
        await waitForCaptureTriggers(call, table)

        // 1. External write: enters postgres through the pg_db pool. The
        // native subscriber must hear it via NOTIFY; the classified
        // subscriber hears the same write attributed to pg_db.
        await call('database::execute', {
          db: 'pg_db',
          sql: `INSERT INTO ${table} (n) VALUES ($1), ($2)`,
          params: [10, 20],
        })
        expectNative(await native.next(), 'insert', 2)
        const viaPg = await classified.next()
        expectEqual(viaPg.db, 'pg_db', 'classified event db')
        expectEqual(viaPg.op, 'insert', 'classified event op')

        // 2. Own write through pg_native_db: must fire exactly ONCE (the
        // NOTIFY path), never twice — self-writes leave the classification
        // path on a native database.
        await call('database::execute', {
          db: NATIVE_DB,
          sql: `UPDATE ${table} SET n = n + 1`,
        })
        expectNative(await native.next(), 'update', 2)

        // 3. A write that changes no rows fires nothing on either path.
        await call('database::execute', {
          db: 'pg_db',
          sql: `UPDATE ${table} SET n = $1 WHERE n = $2`,
          params: [0, -999],
        })

        // 4. Delete via the external client.
        await call('database::execute', {
          db: 'pg_db',
          sql: `DELETE FROM ${table} WHERE n > $1`,
          params: [0],
        })
        expectNative(await native.next(), 'delete', 2)
        const deleted = await classified.next()
        expectEqual(deleted.op, 'delete', 'classified delete op')

        // The zero-row update (step 3) must not have queued anything.
        await sleep(SILENCE_WINDOW_MS)
        native.expectDrained()
        classified.expectDrained()
      } finally {
        nativeTrigger.unregister()
        nativeFn.unregister()
        classifiedTrigger.unregister()
        classifiedFn.unregister()
        await call('database::execute', { db: NATIVE_DB, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  },
  {
    name: 'native capture rejects table-less bindings',
    applies: ['pg_db'],
    async run({ call, iii }) {
      const table = 'e2e_native_tableless'
      const sentinelEvents: RowChangedEvent[] = []
      const tablelessEvents: RowChangedEvent[] = []
      const sentinel = sink(sentinelEvents, 'sentinel subscriber')

      await call('database::execute', { db: NATIVE_DB, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: NATIVE_DB,
        sql: `CREATE TABLE ${table} (id BIGSERIAL PRIMARY KEY, n INT NOT NULL)`,
      })

      // Valid table-scoped binding: installs the triggers and proves events
      // DO flow for this table — without it, the table-less binding's
      // silence below would be vacuous (no triggers, nothing to hear).
      const sentinelFn = iii.registerFunction(
        `harness::native_capture_sentinel`,
        async (payload: RowChangedEvent) => {
          sentinelEvents.push(payload)
          return null
        },
        { description: 'Valid table-scoped sink proving events flow.' },
      )
      const sentinelTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: 'harness::native_capture_sentinel',
        config: { db: NATIVE_DB, table },
      })
      // A db-wide binding is invalid on a native database: per-table
      // triggers are what make external writes visible, so the worker must
      // refuse it. If it were wrongly accepted, its filter (db, no table)
      // would match the sentinel table's events.
      const tablelessFn = iii.registerFunction(
        `harness::native_capture_tableless`,
        async (payload: RowChangedEvent) => {
          tablelessEvents.push(payload)
          return null
        },
        { description: 'Sink that must never receive events (rejected binding).' },
      )
      const tablelessTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: 'harness::native_capture_tableless',
        config: { db: NATIVE_DB },
      })

      try {
        await waitForCaptureTriggers(call, table)
        await sleep(SILENCE_WINDOW_MS) // let the table-less registration settle too
        await call('database::execute', {
          db: NATIVE_DB,
          sql: `INSERT INTO ${table} (n) VALUES ($1)`,
          params: [1],
        })
        const heard = await sentinel.next()
        expectEqual(heard.op, 'insert', 'sentinel hears the insert')
        await sleep(SILENCE_WINDOW_MS)
        expectEqual(tablelessEvents.length, 0, 'rejected table-less binding received an event')
      } finally {
        tablelessTrigger.unregister()
        tablelessFn.unregister()
        sentinelTrigger.unregister()
        sentinelFn.unregister()
        await call('database::execute', { db: NATIVE_DB, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  },
]

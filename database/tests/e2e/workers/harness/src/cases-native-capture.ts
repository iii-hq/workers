/**
 * Native change capture (`capture: native` — postgres and file-backed sqlite).
 *
 * Each native handle points at the same physical database as its
 * statements-path sibling (`pg_native_db` ↔ `pg_db`, `sqlite_native_db` ↔
 * `sqlite_db`). A write that enters through the sibling's pool is — from the
 * native handle's perspective — an external client: different pool,
 * different connections, invisible to SQL classification. If the native
 * subscriber still hears it, the cross-client claim holds end-to-end
 * (postgres: DDL triggers → pg_notify → dedicated LISTEN connection;
 * sqlite: DDL triggers → changelog table → fs-watch drain).
 *
 * Cases carry `applies` for the sibling driver so they run once inside that
 * driver's loop, and address the native handle explicitly in payloads.
 */

import type { DriverKey } from './dialect.ts'
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

/** Everything that differs between the postgres and sqlite native targets. */
interface NativeTarget {
  /** Driver loop that hosts these cases (the statements-path sibling). */
  applies: DriverKey
  /** The `capture: native` handle. */
  nativeDb: string
  idColumnDDL: string
  ph: (i: number) => string
  /** How the database reports the table in events (pg schema-qualifies). */
  eventTable: (table: string) => string
  /** Catalog probe returning the number of installed capture triggers. */
  triggerCountSql: (table: string) => { sql: string; params: unknown[] }
}

const TARGETS: NativeTarget[] = [
  {
    applies: 'pg_db',
    nativeDb: 'pg_native_db',
    idColumnDDL: 'BIGSERIAL PRIMARY KEY',
    ph: (i) => `$${i}`,
    eventTable: (table) => `public.${table}`,
    // `$1::text::regclass`, not `$1::regclass` — a bare regclass cast makes
    // the driver bind the parameter AS regclass (22P03); text binds cleanly
    // and the server does the regclass conversion.
    triggerCountSql: (table) => ({
      sql: `SELECT count(*) AS n FROM pg_trigger WHERE tgrelid = $1::text::regclass AND tgname LIKE 'iii_row_changed_%'`,
      params: [table],
    }),
  },
  {
    applies: 'sqlite_db',
    nativeDb: 'sqlite_native_db',
    idColumnDDL: 'INTEGER PRIMARY KEY AUTOINCREMENT',
    ph: () => '?',
    eventTable: (table) => table,
    triggerCountSql: (table) => ({
      sql: `SELECT count(*) AS n FROM sqlite_master WHERE type = 'trigger' AND tbl_name = ?1 AND name LIKE 'iii_row_changed_%'`,
      params: [table],
    }),
  },
]

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

/**
 * Wait until the worker has installed the capture triggers for `table`.
 * Registration acks race the first write otherwise: delivery starts only
 * once the triggers exist, so a write committed before CREATE TRIGGER lands
 * is silently unheard. The catalog is queried through the worker itself.
 */
async function waitForCaptureTriggers(
  call: (functionId: string, payload: unknown) => Promise<any>,
  target: NativeTarget,
  table: string,
): Promise<void> {
  const probe = target.triggerCountSql(table)
  const deadline = Date.now() + EVENT_TIMEOUT_MS
  for (;;) {
    const r = await call('database::query', { db: target.applies, ...probe })
    if (Number(r.rows?.[0]?.n) === 3) return
    if (Date.now() > deadline) {
      throw new Error(`capture triggers for ${table} were not installed within ${EVENT_TIMEOUT_MS}ms`)
    }
    await sleep(50)
  }
}

function crossClientCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture hears writes from another client, own writes fire once',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_capture_${target.applies}`
      const nativeFnId = `harness::native_capture_events_${target.applies}`
      const classifiedFnId = `harness::native_capture_classified_${target.applies}`
      const nativeEvents: RowChangedEvent[] = []
      const classifiedEvents: RowChangedEvent[] = []
      const native = sink(nativeEvents, 'native subscriber')
      const classified = sink(classifiedEvents, 'classified subscriber')
      const ph = target.ph

      // The watched table must exist before the binding registers — the
      // worker installs the capture triggers at registration time.
      await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: target.nativeDb,
        sql: `CREATE TABLE ${table} (id ${target.idColumnDDL}, n INT NOT NULL)`,
      })

      const nativeFn = iii.registerFunction(
        nativeFnId,
        async (payload: RowChangedEvent) => {
          nativeEvents.push(payload)
          return null
        },
        { description: 'Native-capture E2E event sink.' },
      )
      const nativeTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: nativeFnId,
        config: { db: target.nativeDb, table },
      })
      // Statements-path subscriber on the sibling handle, watching the SAME
      // physical table. Proves the two capture modes coexist and attribute
      // correctly.
      const classifiedFn = iii.registerFunction(
        classifiedFnId,
        async (payload: RowChangedEvent) => {
          classifiedEvents.push(payload)
          return null
        },
        { description: 'Statements-path E2E event sink for the native-capture table.' },
      )
      const classifiedTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: classifiedFnId,
        config: { db: target.applies, table },
      })

      const expectNative = (event: RowChangedEvent, op: RowChangedEvent['op'], affectedRows: number): void => {
        expectEqual(event.db, target.nativeDb, 'native event db')
        expectEqual(event.table, target.eventTable(table), 'native event table')
        expectEqual(event.op, op, 'native event op')
        expectEqual(event.affected_rows, affectedRows, 'native event affected_rows')
        expect(event.returning === undefined, 'native events carry no RETURNING rows')
        expect(Number.isFinite(event.at) && event.at > 0, 'native event at is an epoch timestamp')
      }

      try {
        // Wait until BOTH bindings are visible to the engine before writing.
        const registered = await call('engine::registered-triggers::list', {})
        for (const fn of [nativeFnId, classifiedFnId]) {
          expect(
            registered.registered_triggers.some(
              (t: { trigger_type: string; function_id: string }) =>
                t.trigger_type === 'database::row-changed' && t.function_id === fn,
            ),
            `trigger registration for ${fn} is visible to the engine`,
          )
        }
        // …and until the worker's DDL install has landed in the database.
        await waitForCaptureTriggers(call, target, table)

        // 1. External write: enters through the sibling pool. The native
        // subscriber must hear it via the database; the classified
        // subscriber hears the same write attributed to the sibling.
        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)}), (${ph(2)})`,
          params: [10, 20],
        })
        expectNative(await native.next(), 'insert', 2)
        const viaSibling = await classified.next()
        expectEqual(viaSibling.db, target.applies, 'classified event db')
        expectEqual(viaSibling.op, 'insert', 'classified event op')

        // 2. Own write through the native handle: must fire exactly ONCE,
        // never twice — self-writes leave the classification path on a
        // native database.
        await call('database::execute', {
          db: target.nativeDb,
          sql: `UPDATE ${table} SET n = n + 1`,
        })
        expectNative(await native.next(), 'update', 2)

        // 3. A write that changes no rows fires nothing on either path.
        await call('database::execute', {
          db: target.applies,
          sql: `UPDATE ${table} SET n = ${ph(1)} WHERE n = ${ph(2)}`,
          params: [0, -999],
        })

        // 4. Delete via the external client.
        await call('database::execute', {
          db: target.applies,
          sql: `DELETE FROM ${table} WHERE n > ${ph(1)}`,
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
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

function tablelessRejectionCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture rejects table-less bindings',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_tableless_${target.applies}`
      const sentinelFnId = `harness::native_capture_sentinel_${target.applies}`
      const tablelessFnId = `harness::native_capture_tableless_${target.applies}`
      const sentinelEvents: RowChangedEvent[] = []
      const tablelessEvents: RowChangedEvent[] = []
      const sentinel = sink(sentinelEvents, 'sentinel subscriber')

      await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: target.nativeDb,
        sql: `CREATE TABLE ${table} (id ${target.idColumnDDL}, n INT NOT NULL)`,
      })

      // Valid table-scoped binding: installs the triggers and proves events
      // DO flow for this table — without it, the table-less binding's
      // silence below would be vacuous (no triggers, nothing to hear).
      const sentinelFn = iii.registerFunction(
        sentinelFnId,
        async (payload: RowChangedEvent) => {
          sentinelEvents.push(payload)
          return null
        },
        { description: 'Valid table-scoped sink proving events flow.' },
      )
      const sentinelTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: sentinelFnId,
        config: { db: target.nativeDb, table },
      })
      // A db-wide binding is invalid on a native database: per-table
      // triggers are what make external writes visible, so the worker must
      // refuse it. If it were wrongly accepted, its filter (db, no table)
      // would match the sentinel table's events.
      const tablelessFn = iii.registerFunction(
        tablelessFnId,
        async (payload: RowChangedEvent) => {
          tablelessEvents.push(payload)
          return null
        },
        { description: 'Sink that must never receive events (rejected binding).' },
      )
      const tablelessTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: tablelessFnId,
        config: { db: target.nativeDb },
      })

      try {
        await waitForCaptureTriggers(call, target, table)
        await sleep(SILENCE_WINDOW_MS) // let the table-less registration settle too
        await call('database::execute', {
          db: target.nativeDb,
          sql: `INSERT INTO ${table} (n) VALUES (${target.ph(1)})`,
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
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

export const NATIVE_CAPTURE_CASES: TestCase[] = TARGETS.flatMap((target) => [
  crossClientCase(target),
  tablelessRejectionCase(target),
])

/**
 * Native change capture (`capture: native` — postgres, file-backed sqlite,
 * and mysql).
 *
 * Each native handle points at the same physical database as its
 * statements-path sibling (`pg_native_db` ↔ `pg_db`, `sqlite_native_db` ↔
 * `sqlite_db`, `mysql_native_db` ↔ `mysql_db`). A write that enters through
 * the sibling's pool is — from the native handle's perspective — an
 * external client: different pool, different connections, invisible to SQL
 * classification. If the native subscriber still hears it, the cross-client
 * claim holds end-to-end (postgres: DDL triggers → pg_notify → dedicated
 * LISTEN connection; sqlite: DDL triggers → changelog table → fs-watch
 * drain; mysql: the binlog replication stream, nothing installed).
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

/** Everything that differs between the native targets. */
interface NativeTarget {
  /** Driver loop that hosts these cases (the statements-path sibling). */
  applies: DriverKey
  /** The `capture: native` handle. */
  nativeDb: string
  idColumnDDL: string
  ph: (i: number) => string
  /** How the database reports the table in events (pg schema-qualifies). */
  eventTable: (table: string) => string
  /**
   * Catalog probe returning the number of installed capture triggers.
   * Absent for binlog capture (mysql), which installs nothing — readiness
   * is proven by the warmup write loop instead.
   */
  triggerCountSql?: (table: string) => { sql: string; params: unknown[] }
  /**
   * Whether a native binding spelled in the WRONG case still captures.
   * pg: no — the table name is quoted verbatim into DDL and quoted
   * postgres identifiers are case-sensitive, so install fails loudly.
   * sqlite: yes — quoted identifiers match case-insensitively.
   * mysql: yes — no DDL at all; the bus filter matches case-insensitively.
   */
  uppercaseBindingCaptures: boolean
  /**
   * Whether registering against a table that does not exist is ACCEPTED.
   * pg/sqlite reject at DDL install; mysql has nothing to install, so the
   * binding is accepted and starts capturing when the table appears.
   */
  missingTableBindingAccepted: boolean
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
    uppercaseBindingCaptures: false,
    missingTableBindingAccepted: false,
  },
  {
    applies: 'sqlite_db',
    nativeDb: 'sqlite_native_db',
    idColumnDDL: 'INTEGER PRIMARY KEY AUTOINCREMENT',
    ph: () => '?',
    eventTable: (table) => table,
    // lower() on both sides: a reinstall from a differently-cased binding
    // stores tbl_name with THAT spelling, and sqlite's `=` is case-sensitive.
    triggerCountSql: (table) => ({
      sql: `SELECT count(*) AS n FROM sqlite_master WHERE type = 'trigger' AND lower(tbl_name) = lower(?1) AND name LIKE 'iii_row_changed_%'`,
      params: [table],
    }),
    uppercaseBindingCaptures: true,
    missingTableBindingAccepted: false,
  },
  {
    applies: 'mysql_db',
    nativeDb: 'mysql_native_db',
    idColumnDDL: 'BIGINT AUTO_INCREMENT PRIMARY KEY',
    ph: () => '?',
    eventTable: (table) => table,
    // Binlog capture installs nothing to probe for.
    uppercaseBindingCaptures: true,
    missingTableBindingAccepted: true,
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
 * Wait until native capture is actually delivering for this target.
 * Registration acks race the first write otherwise — and delivery is
 * at-most-once, so a racing write is silently unheard.
 *
 * Trigger-based targets (pg, sqlite): poll the catalog through the worker
 * until the three capture triggers exist. Binlog capture (mysql) installs
 * nothing to probe, so prove the stream is attached empirically: a warmup
 * table with its own binding is poked until an event comes back.
 */
async function waitForCaptureReady(
  call: (functionId: string, payload: unknown) => Promise<any>,
  iii: any,
  target: NativeTarget,
  table: string,
): Promise<void> {
  if (target.triggerCountSql) {
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

  const warmupTable = `e2e_native_warmup_${target.applies}`
  const fnId = `harness::native_warmup_${target.applies}`
  const events: RowChangedEvent[] = []
  await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${warmupTable}` })
  await call('database::execute', {
    db: target.nativeDb,
    sql: `CREATE TABLE ${warmupTable} (id ${target.idColumnDDL}, n INT NOT NULL)`,
  })
  const fnRef = iii.registerFunction(
    fnId,
    async (payload: RowChangedEvent) => {
      events.push(payload)
      return null
    },
    { description: 'Warmup sink proving the capture stream is attached.' },
  )
  const triggerRef = iii.registerTrigger({
    type: 'database::row-changed',
    function_id: fnId,
    config: { db: target.nativeDb, table: warmupTable },
  })
  try {
    const deadline = Date.now() + 15_000
    while (events.length === 0) {
      if (Date.now() > deadline) {
        throw new Error(`capture stream for ${target.nativeDb} did not deliver within 15s`)
      }
      await call('database::execute', {
        db: target.nativeDb,
        sql: `INSERT INTO ${warmupTable} (n) VALUES (${target.ph(1)})`,
        params: [1],
      })
      const poked = Date.now()
      while (events.length === 0 && Date.now() - poked < 700) await sleep(20)
    }
  } finally {
    triggerRef.unregister()
    fnRef.unregister()
    await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${warmupTable}` })
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
        await waitForCaptureReady(call, iii, target, table)

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
        await waitForCaptureReady(call, iii, target, table)
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

/**
 * Commit gating through the real database, not the worker's staging: on a
 * native handle the worker's own classification path is off, so an
 * interactive transaction's visibility is decided entirely by the capture
 * mechanism (pg: NOTIFY is transactional; sqlite: changelog rows ride the
 * writer's transaction; mysql: only committed transactions reach the
 * binlog). Rollback must be absolute silence; commit must deliver.
 */
function txGatingCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture is commit-gated through interactive transactions',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_tx_${target.applies}`
      const fnId = `harness::native_tx_${target.applies}`
      const events: RowChangedEvent[] = []
      const native = sink(events, 'native tx subscriber')
      const ph = target.ph

      await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: target.nativeDb,
        sql: `CREATE TABLE ${table} (id ${target.idColumnDDL}, n INT NOT NULL)`,
      })
      const fnRef = iii.registerFunction(
        fnId,
        async (payload: RowChangedEvent) => {
          events.push(payload)
          return null
        },
        { description: 'Native-capture transaction-gating E2E sink.' },
      )
      const triggerRef = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: fnId,
        config: { db: target.nativeDb, table },
      })

      let activeTransaction: string | undefined
      try {
        await waitForCaptureReady(call, iii, target, table)

        // 1. Uncommitted writes are invisible…
        activeTransaction = (
          await call('database::beginTransaction', { db: target.nativeDb })
        ).transaction.id
        await call('database::transactionExecute', {
          transaction_id: activeTransaction,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)})`,
          params: [1],
        })
        await sleep(SILENCE_WINDOW_MS)
        native.expectDrained()

        // …and a rollback erases them for good.
        await call('database::rollbackTransaction', { transaction_id: activeTransaction })
        activeTransaction = undefined
        await sleep(SILENCE_WINDOW_MS)
        native.expectDrained()

        // 2. A committed transaction delivers — once, after the commit.
        activeTransaction = (
          await call('database::beginTransaction', { db: target.nativeDb })
        ).transaction.id
        await call('database::transactionExecute', {
          transaction_id: activeTransaction,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)})`,
          params: [2],
        })
        await call('database::transactionExecute', {
          transaction_id: activeTransaction,
          sql: `UPDATE ${table} SET n = n + 1`,
        })
        await sleep(SILENCE_WINDOW_MS)
        native.expectDrained()
        await call('database::commitTransaction', { transaction_id: activeTransaction })
        activeTransaction = undefined

        // Exactly two events, one per statement — but back-to-back Void
        // dispatches carry no cross-event ordering guarantee, so assert the
        // set, not the sequence.
        const committed = [await native.next(), await native.next()]
        const ops = committed.map((e) => e.op).sort()
        expectEqual(ops, ['insert', 'update'], 'committed transaction delivers both events')
        for (const event of committed) {
          expectEqual(event.affected_rows, 1, `committed ${event.op} affected_rows`)
        }
        await sleep(SILENCE_WINDOW_MS)
        native.expectDrained()
      } finally {
        if (activeTransaction) {
          try {
            await call('database::rollbackTransaction', { transaction_id: activeTransaction })
          } catch {
            /* transaction may already be finalized */
          }
        }
        triggerRef.unregister()
        fnRef.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

/**
 * Fan-out and filtering on the native path: an all-ops subscriber and a
 * delete-only subscriber share one table. Registering the second binding
 * REINSTALLS the capture DDL (pg/sqlite) — the first subscriber must keep
 * hearing through it. After both unregister, external writes still succeed
 * (orphaned triggers are inert, not broken) and deliver to no one.
 */
function fanOutOpsCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture fans out, filters ops, and survives trigger reinstall',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_fanout_${target.applies}`
      const allFnId = `harness::native_fanout_all_${target.applies}`
      const deletesFnId = `harness::native_fanout_deletes_${target.applies}`
      const allEvents: RowChangedEvent[] = []
      const deleteEvents: RowChangedEvent[] = []
      const all = sink(allEvents, 'all-ops subscriber')
      const deletes = sink(deleteEvents, 'delete-only subscriber')
      const ph = target.ph

      await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: target.nativeDb,
        sql: `CREATE TABLE ${table} (id ${target.idColumnDDL}, n INT NOT NULL)`,
      })
      const allFn = iii.registerFunction(
        allFnId,
        async (payload: RowChangedEvent) => {
          allEvents.push(payload)
          return null
        },
        { description: 'Native-capture fan-out E2E sink (all ops).' },
      )
      const allTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: allFnId,
        config: { db: target.nativeDb, table },
      })
      // Second binding on the SAME table: worker-side this re-runs the DDL
      // install (DROP + CREATE trigger) while the first binding is live.
      const deletesFn = iii.registerFunction(
        deletesFnId,
        async (payload: RowChangedEvent) => {
          deleteEvents.push(payload)
          return null
        },
        { description: 'Native-capture fan-out E2E sink (deletes only).' },
      )
      const deletesTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: deletesFnId,
        config: { db: target.nativeDb, table, ops: ['delete'] },
      })

      let bindingsLive = true
      const unregisterBindings = () => {
        if (!bindingsLive) return
        bindingsLive = false
        allTrigger.unregister()
        deletesTrigger.unregister()
      }

      try {
        const registered = await call('engine::registered-triggers::list', {})
        for (const fn of [allFnId, deletesFnId]) {
          expect(
            registered.registered_triggers.some(
              (t: { trigger_type: string; function_id: string }) =>
                t.trigger_type === 'database::row-changed' && t.function_id === fn,
            ),
            `trigger registration for ${fn} is visible to the engine`,
          )
        }
        await waitForCaptureReady(call, iii, target, table)
        // The second registration's reinstall races the engine ack; give the
        // worker a beat so no write lands mid DROP/CREATE.
        await sleep(300)

        // Insert (external client): all-ops hears, delete-only does not.
        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)})`,
          params: [10],
        })
        expectEqual((await all.next()).op, 'insert', 'all-ops subscriber hears insert')
        await sleep(SILENCE_WINDOW_MS)
        deletes.expectDrained()

        // Delete: both hear exactly one event.
        await call('database::execute', {
          db: target.applies,
          sql: `DELETE FROM ${table} WHERE n = ${ph(1)}`,
          params: [10],
        })
        expectEqual((await all.next()).op, 'delete', 'all-ops subscriber hears delete')
        expectEqual((await deletes.next()).op, 'delete', 'delete-only subscriber hears delete')

        // Unregister both; external writes still succeed and nobody hears.
        unregisterBindings()
        await sleep(SILENCE_WINDOW_MS)
        const r = await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)})`,
          params: [20],
        })
        expectEqual(r.affected_rows, 1, 'write succeeds after unregister (orphan capture is inert)')
        await sleep(SILENCE_WINDOW_MS)
        all.expectDrained()
        deletes.expectDrained()
      } finally {
        unregisterBindings()
        allFn.unregister()
        deletesFn.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

/**
 * Bulk statements stay single events with true counts: 100 rows inserted,
 * updated, deleted must arrive as exactly three events with
 * affected_rows=100 — never one event per row. Each driver earns this a
 * different way (pg statement-level triggers with transition tables,
 * sqlite run-length coalescing of changelog rows, mysql merging of chunked
 * binlog row events), so proving it end-to-end covers all three coalescers.
 */
function bulkCoalescingCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture coalesces bulk statements into single events',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_bulk_${target.applies}`
      const fnId = `harness::native_bulk_${target.applies}`
      const events: RowChangedEvent[] = []
      const native = sink(events, 'bulk subscriber')
      const ROWS = 100

      await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: target.nativeDb,
        sql: `CREATE TABLE ${table} (id ${target.idColumnDDL}, n INT NOT NULL)`,
      })
      const fnRef = iii.registerFunction(
        fnId,
        async (payload: RowChangedEvent) => {
          events.push(payload)
          return null
        },
        { description: 'Native-capture bulk-coalescing E2E sink.' },
      )
      const triggerRef = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: fnId,
        config: { db: target.nativeDb, table },
      })

      try {
        await waitForCaptureReady(call, iii, target, table)

        const values = Array.from({ length: ROWS }, (_, i) => `(${i})`).join(', ')
        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES ${values}`,
        })
        const inserted = await native.next()
        expectEqual(inserted.op, 'insert', 'bulk insert op')
        expectEqual(inserted.affected_rows, ROWS, 'bulk insert arrives as ONE event')

        await call('database::execute', {
          db: target.applies,
          sql: `UPDATE ${table} SET n = n + 1`,
        })
        const updated = await native.next()
        expectEqual(updated.op, 'update', 'bulk update op')
        expectEqual(updated.affected_rows, ROWS, 'bulk update arrives as ONE event')

        await call('database::execute', { db: target.applies, sql: `DELETE FROM ${table}` })
        const deleted = await native.next()
        expectEqual(deleted.op, 'delete', 'bulk delete op')
        expectEqual(deleted.affected_rows, ROWS, 'bulk delete arrives as ONE event')

        // Exactly three events total — a per-row implementation would have
        // flooded 300.
        await sleep(SILENCE_WINDOW_MS)
        native.expectDrained()
      } finally {
        triggerRef.unregister()
        fnRef.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

/**
 * Table-name casing on the NATIVE path is driver-specific, and the
 * difference is deliberate, documented behavior — pin it: an
 * uppercase-spelled binding on a lowercase table captures on sqlite
 * (case-insensitive quoted identifiers) and mysql (no DDL; the bus filter
 * matches case-insensitively), but is rejected at DDL install on postgres
 * (quoted identifiers are case-sensitive). An exact-spelling sentinel
 * binding proves events flow either way — silence is never vacuous.
 */
function caseSensitivityCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture table casing behaves per driver contract',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_case_${target.applies}`
      const exactFnId = `harness::native_case_exact_${target.applies}`
      const upperFnId = `harness::native_case_upper_${target.applies}`
      const exactEvents: RowChangedEvent[] = []
      const upperEvents: RowChangedEvent[] = []
      const exact = sink(exactEvents, 'exact-spelling subscriber')
      const upper = sink(upperEvents, 'uppercase subscriber')
      const ph = target.ph

      await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: target.nativeDb,
        sql: `CREATE TABLE ${table} (id ${target.idColumnDDL}, n INT NOT NULL)`,
      })
      const exactFn = iii.registerFunction(
        exactFnId,
        async (payload: RowChangedEvent) => {
          exactEvents.push(payload)
          return null
        },
        { description: 'Exact-spelling native binding (sentinel).' },
      )
      const exactTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: exactFnId,
        config: { db: target.nativeDb, table },
      })
      const upperFn = iii.registerFunction(
        upperFnId,
        async (payload: RowChangedEvent) => {
          upperEvents.push(payload)
          return null
        },
        { description: 'Uppercase-spelled native binding.' },
      )
      const upperTrigger = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: upperFnId,
        config: { db: target.nativeDb, table: table.toUpperCase() },
      })

      try {
        await waitForCaptureReady(call, iii, target, table)
        // Let the uppercase registration finish its install attempt (which
        // on pg fails, on sqlite reinstalls the same triggers).
        await sleep(500)

        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)})`,
          params: [1],
        })
        const heard = await exact.next()
        expectEqual(heard.op, 'insert', 'exact-spelling binding hears the write')

        if (target.uppercaseBindingCaptures) {
          const viaUpper = await upper.next()
          expectEqual(viaUpper.op, 'insert', 'uppercase binding hears the write')
        }
        // Exactly one event per subscriber, ever: a differently-cased
        // reinstall must converge on ONE trigger set — a second set would
        // double-log every write and fail here.
        await sleep(SILENCE_WINDOW_MS)
        exact.expectDrained()
        upper.expectDrained()
      } finally {
        upperTrigger.unregister()
        upperFn.unregister()
        exactTrigger.unregister()
        exactFn.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

/**
 * Two tables, two bindings, no cross-talk: per-table capture installation
 * and per-binding filtering keep each subscriber scoped to its own table.
 */
function twoTableIsolationCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture isolates bindings per table',
    applies: [target.applies],
    async run({ call, iii }) {
      const tableA = `e2e_native_iso_a_${target.applies}`
      const tableB = `e2e_native_iso_b_${target.applies}`
      const fnA = `harness::native_iso_a_${target.applies}`
      const fnB = `harness::native_iso_b_${target.applies}`
      const eventsA: RowChangedEvent[] = []
      const eventsB: RowChangedEvent[] = []
      const sinkA = sink(eventsA, 'table-A subscriber')
      const sinkB = sink(eventsB, 'table-B subscriber')
      const ph = target.ph

      for (const table of [tableA, tableB]) {
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
        await call('database::execute', {
          db: target.nativeDb,
          sql: `CREATE TABLE ${table} (id ${target.idColumnDDL}, n INT NOT NULL)`,
        })
      }
      const fnARef = iii.registerFunction(
        fnA,
        async (payload: RowChangedEvent) => {
          eventsA.push(payload)
          return null
        },
        { description: 'Table-A isolation E2E sink.' },
      )
      const triggerA = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: fnA,
        config: { db: target.nativeDb, table: tableA },
      })
      const fnBRef = iii.registerFunction(
        fnB,
        async (payload: RowChangedEvent) => {
          eventsB.push(payload)
          return null
        },
        { description: 'Table-B isolation E2E sink.' },
      )
      const triggerB = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: fnB,
        config: { db: target.nativeDb, table: tableB },
      })

      try {
        await waitForCaptureReady(call, iii, target, tableA)
        await waitForCaptureReady(call, iii, target, tableB)

        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${tableA} (n) VALUES (${ph(1)})`,
          params: [1],
        })
        expectEqual((await sinkA.next()).op, 'insert', 'A subscriber hears table A')
        await sleep(SILENCE_WINDOW_MS)
        sinkB.expectDrained()

        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${tableB} (n) VALUES (${ph(1)})`,
          params: [2],
        })
        expectEqual((await sinkB.next()).op, 'insert', 'B subscriber hears table B')
        await sleep(SILENCE_WINDOW_MS)
        sinkA.expectDrained()
      } finally {
        triggerA.unregister()
        fnARef.unregister()
        triggerB.unregister()
        fnBRef.unregister()
        for (const table of [tableA, tableB]) {
          await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
        }
      }
    },
  }
}

/**
 * Registering against a table that does not exist: pg/sqlite must reject
 * at DDL install — and stay silent even after the table is created,
 * proving the rejection was final, not deferred. mysql has nothing to
 * install, so the binding is accepted and starts capturing the moment the
 * table appears in the binlog. The asymmetry is contract; pin both sides.
 */
function missingTableRegistrationCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture registration against a missing table behaves per driver contract',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_missing_${target.applies}`
      const fnId = `harness::native_missing_${target.applies}`
      const events: RowChangedEvent[] = []
      const native = sink(events, 'missing-table subscriber')
      const ph = target.ph

      await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      const fnRef = iii.registerFunction(
        fnId,
        async (payload: RowChangedEvent) => {
          events.push(payload)
          return null
        },
        { description: 'Missing-table registration E2E sink.' },
      )
      const triggerRef = iii.registerTrigger({
        type: 'database::row-changed',
        function_id: fnId,
        config: { db: target.nativeDb, table },
      })

      try {
        // Let the registration (and, on pg/sqlite, its failing DDL install)
        // fully settle BEFORE the table exists — creating it too early
        // would let the install succeed and test nothing.
        await sleep(1_000)

        await call('database::execute', {
          db: target.nativeDb,
          sql: `CREATE TABLE ${table} (id ${target.idColumnDDL}, n INT NOT NULL)`,
        })
        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)})`,
          params: [1],
        })

        if (target.missingTableBindingAccepted) {
          const heard = await native.next()
          expectEqual(heard.op, 'insert', 'accepted binding captures once the table exists')
        } else {
          await sleep(SILENCE_WINDOW_MS)
          native.expectDrained()
        }
      } finally {
        triggerRef.unregister()
        fnRef.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

export const NATIVE_CAPTURE_CASES: TestCase[] = TARGETS.flatMap((target) => [
  crossClientCase(target),
  tablelessRejectionCase(target),
  txGatingCase(target),
  fanOutOpsCase(target),
  bulkCoalescingCase(target),
  caseSensitivityCase(target),
  twoTableIsolationCase(target),
  missingTableRegistrationCase(target),
])

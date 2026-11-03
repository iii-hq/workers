/**
 * Native change capture (`capture: native` — postgres, file-backed sqlite,
 * and mysql).
 *
 * Each native handle points at the same physical database as its
 * statements-path sibling (`pg_native_db` ↔ `pg_db`, `sqlite_native_db` ↔
 * `sqlite_db`, `mysql_native_db` ↔ `mysql_db`). A write that enters through
 * the sibling's pool is — from the native handle's perspective — an
 * external client: different pool, different connections, invisible to SQL
 * classification. If the native observer still hears it, the cross-client
 * claim holds end-to-end (postgres: database triggers → pg_notify →
 * dedicated LISTEN connection; sqlite: database triggers → changelog table →
 * fs-watch drain; mysql: the binlog replication stream, nothing installed).
 *
 * Cases carry `applies` for the sibling driver so they run once inside that
 * driver's loop, and address the native handle explicitly in payloads.
 */

import type { DriverKey } from './dialect.ts'
import type { TestCase } from './cases.ts'
import { expect, expectEqual, observeRowChanged, refusalCode } from './cases.ts'

const EVENT_TIMEOUT_MS = 5_000
const SILENCE_WINDOW_MS = 500

interface RowChangedEvent {
  db: string
  schema: string | null
  table: string | null
  op: 'insert' | 'update' | 'delete' | 'other'
  affected_rows: number
  returning?: Record<string, unknown>[]
  truncated?: boolean
  at: number
}

/** Native events carry at most this many keys; the rest is `truncated`. */
const KEY_CAP = 100

/** Everything that differs between the native targets. */
interface NativeTarget {
  /** Driver loop that hosts these cases (the statements-path sibling). */
  applies: DriverKey
  /** The `capture: native` handle. */
  nativeDb: string
  idColumnDDL: string
  ph: (i: number) => string
  /** The schema events name (pg reports one; the table itself is canonical). */
  eventSchema: string | null
  /**
   * How the id column's value appears in an event's `returning`: the same
   * encoding `database::query` uses for that column — pg's BIGSERIAL is a
   * bigint and travels as a string, sqlite's INTEGER and mysql's BIGINT as
   * numbers.
   */
  keyOf: (id: number) => unknown
  /**
   * Catalog probe returning the number of installed capture triggers.
   * Absent for binlog capture (mysql), which installs nothing — readiness
   * is proven by the warmup write loop instead.
   */
  triggerCountSql?: (table: string) => { sql: string; params: unknown[] }
  /**
   * Whether an observer of a table that does not exist is ACCEPTED.
   * pg/sqlite reject at DDL install; mysql has nothing to install, so the
   * observer is accepted and starts hearing when the table appears.
   */
  missingTableObserverAccepted: boolean
}

const TARGETS: NativeTarget[] = [
  {
    applies: 'pg_db',
    nativeDb: 'pg_native_db',
    idColumnDDL: 'BIGSERIAL PRIMARY KEY',
    ph: (i) => `$${i}`,
    eventSchema: 'public',
    keyOf: (id) => String(id),
    // `$1::text::regclass`, not `$1::regclass` — a bare regclass cast makes
    // the driver bind the parameter AS regclass (22P03); text binds cleanly
    // and the server does the regclass conversion.
    triggerCountSql: (table) => ({
      sql: `SELECT count(*) AS n FROM pg_trigger WHERE tgrelid = $1::text::regclass AND tgname LIKE 'iii_row_changed_%'`,
      params: [table],
    }),
    missingTableObserverAccepted: false,
  },
  {
    applies: 'sqlite_db',
    nativeDb: 'sqlite_native_db',
    idColumnDDL: 'INTEGER PRIMARY KEY AUTOINCREMENT',
    ph: () => '?',
    eventSchema: null,
    keyOf: (id) => id,
    // lower() on both sides: an install from a differently-cased spelling
    // stores tbl_name with THAT spelling, and sqlite's `=` is case-sensitive.
    triggerCountSql: (table) => ({
      sql: `SELECT count(*) AS n FROM sqlite_master WHERE type = 'trigger' AND lower(tbl_name) = lower(?1) AND name LIKE 'iii_row_changed_%'`,
      params: [table],
    }),
    missingTableObserverAccepted: false,
  },
  {
    applies: 'mysql_db',
    nativeDb: 'mysql_native_db',
    idColumnDDL: 'BIGINT AUTO_INCREMENT PRIMARY KEY',
    ph: () => '?',
    eventSchema: null,
    keyOf: (id) => id,
    // Binlog capture installs nothing to probe for.
    missingTableObserverAccepted: true,
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
 * Wait until native capture is actually delivering for this target, after
 * the observers are active. Delivery is at-most-once, so a write that races
 * the capture is silently unheard.
 *
 * Database-trigger targets (pg, sqlite): the added hook installed the DDL
 * before the observer became active; the catalog poll through the worker
 * confirms the three capture triggers exist. Binlog capture (mysql) installs
 * nothing to probe and its stream attaches on its own schedule, so prove it
 * empirically: a warmup table with its own observer is poked until an event
 * comes back.
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
  const observer = observeRowChanged(iii, { db: target.nativeDb, table: warmupTable }, fnId)
  try {
    await observer.ready()
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
    await observer.dispose()
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
      const native = sink(nativeEvents, 'native observer')
      const classified = sink(classifiedEvents, 'classified observer')
      const ph = target.ph

      // The watched table must exist before the observer is added — the
      // worker installs the capture triggers in its added hook.
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
      const nativeObserver = observeRowChanged(iii, { db: target.nativeDb, table }, nativeFnId)
      // Statements-path observer on the sibling handle, watching the SAME
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
      const classifiedObserver = observeRowChanged(iii, { db: target.applies, table }, classifiedFnId)

      const expectNative = (
        event: RowChangedEvent,
        op: RowChangedEvent['op'],
        affectedRows: number,
        ids: number[],
      ): void => {
        expectEqual(event.db, target.nativeDb, 'native event db')
        expectEqual(event.table, table, 'native event table (canonical)')
        expectEqual(event.schema, target.eventSchema, 'native event schema')
        expectEqual(event.op, op, 'native event op')
        expectEqual(event.affected_rows, affectedRows, 'native event affected_rows')
        // Native events name the changed rows by primary key — identity
        // without the writer's cooperation, encoded as database::query
        // would return the column. Row order within a statement is the
        // database's business, so compare as a set.
        const byKey = (rows: Record<string, unknown>[] | undefined) =>
          (rows ?? []).map((row) => JSON.stringify(row)).sort()
        expectEqual(
          byKey(event.returning),
          byKey(ids.map((id) => ({ id: target.keyOf(id) }))),
          "native event carries the changed rows' primary keys",
        )
        expect(event.truncated === undefined, 'under the cap nothing is truncated')
        expect(Number.isFinite(event.at) && event.at > 0, 'native event at is an epoch timestamp')
      }

      try {
        // Wait until BOTH observers are active before writing…
        await Promise.all([nativeObserver.ready(), classifiedObserver.ready()])
        // …and until the worker's DDL install has landed in the database.
        await waitForCaptureReady(call, iii, target, table)

        // 1. External write: enters through the sibling pool. The native
        // observer must hear it via the database; the classified
        // observer hears the same write attributed to the sibling.
        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)}), (${ph(2)})`,
          params: [10, 20],
        })
        expectNative(await native.next(), 'insert', 2, [1, 2])
        const viaSibling = await classified.next()
        expectEqual(viaSibling.db, target.applies, 'classified event db')
        expectEqual(viaSibling.op, 'insert', 'classified event op')

        // 2. Own write through the native handle: must fire exactly ONCE,
        // never twice — self-writes leave the classification path on a
        // native database. The writer still gets its RETURNING rows back;
        // the event carries the keys the database reported, not that
        // projection.
        const ownReturning = target.applies === 'mysql_db' ? '' : ' RETURNING id'
        const own = await call('database::execute', {
          db: target.nativeDb,
          sql: `UPDATE ${table} SET n = n + 1${ownReturning}`,
        })
        if (ownReturning) {
          expectEqual(own.returned_rows.length, 2, 'own write on a native db still returns its RETURNING rows')
        }
        expectNative(await native.next(), 'update', 2, [1, 2])

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
        expectNative(await native.next(), 'delete', 2, [1, 2])
        const deleted = await classified.next()
        expectEqual(deleted.op, 'delete', 'classified delete op')

        // The zero-row update (step 3) must not have queued anything.
        await sleep(SILENCE_WINDOW_MS)
        native.expectDrained()
        classified.expectDrained()
      } finally {
        await nativeObserver.dispose()
        nativeFn.unregister()
        await classifiedObserver.dispose()
        classifiedFn.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

function tablelessRejectionCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture rejects table-less observers',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_tableless_${target.applies}`
      const sentinelFnId = `harness::native_capture_sentinel_${target.applies}`
      const tablelessFnId = `harness::native_capture_tableless_${target.applies}`
      const sentinelEvents: RowChangedEvent[] = []
      const tablelessEvents: RowChangedEvent[] = []
      const sentinel = sink(sentinelEvents, 'sentinel observer')

      await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: target.nativeDb,
        sql: `CREATE TABLE ${table} (id ${target.idColumnDDL}, n INT NOT NULL)`,
      })

      // Valid table-scoped observer: installs the triggers and proves events
      // DO flow for this table — without it, the table-less observer's
      // silence below would be vacuous (no triggers, nothing to hear).
      const sentinelFn = iii.registerFunction(
        sentinelFnId,
        async (payload: RowChangedEvent) => {
          sentinelEvents.push(payload)
          return null
        },
        { description: 'Valid table-scoped sink proving events flow.' },
      )
      const sentinelObserver = observeRowChanged(iii, { db: target.nativeDb, table }, sentinelFnId)
      // A db-wide observer is invalid on a native database: per-table
      // database triggers are what make external writes visible, so the
      // worker must refuse it. If it were wrongly accepted, its args (db, no
      // table) would match the sentinel table's events.
      const tablelessFn = iii.registerFunction(
        tablelessFnId,
        async (payload: RowChangedEvent) => {
          tablelessEvents.push(payload)
          return null
        },
        { description: 'Sink that must never receive events (rejected observer).' },
      )
      const tablelessObserver = observeRowChanged(iii, { db: target.nativeDb }, tablelessFnId)

      try {
        await sentinelObserver.ready()
        expectEqual(
          await refusalCode(tablelessObserver),
          'NATIVE_CAPTURE_REQUIRES_TABLE',
          'the table-less observer is refused',
        )
        await waitForCaptureReady(call, iii, target, table)
        await call('database::execute', {
          db: target.nativeDb,
          sql: `INSERT INTO ${table} (n) VALUES (${target.ph(1)})`,
          params: [1],
        })
        const heard = await sentinel.next()
        expectEqual(heard.op, 'insert', 'sentinel hears the insert')
        await sleep(SILENCE_WINDOW_MS)
        expectEqual(tablelessEvents.length, 0, 'rejected table-less observer received an event')
      } finally {
        await tablelessObserver.dispose()
        tablelessFn.unregister()
        await sentinelObserver.dispose()
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
      const native = sink(events, 'native tx observer')
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
      const observer = observeRowChanged(iii, { db: target.nativeDb, table }, fnId)

      let activeTransaction: string | undefined
      try {
        await observer.ready()
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

        // Exactly two events, one per statement — the native listener sees
        // them as the database reports them, so assert the set, not the
        // sequence.
        const committed = [await native.next(), await native.next()]
        const ops = committed.map((e) => e.op).sort()
        expectEqual(ops, ['insert', 'update'], 'committed transaction delivers both events')
        // BIGSERIAL / AUTO_INCREMENT do not roll back, so the discarded
        // insert above burned an id and the committed row is NOT necessarily
        // id 1 (sqlite's sequence rides the transaction, the others' do
        // not). Read the row back and hold the events to what actually
        // exists — the claim under test is that the event names THE row, not
        // a particular id.
        const rows = await call('database::query', {
          db: target.nativeDb,
          sql: `SELECT id FROM ${table}`,
        })
        expectEqual(rows.rows.length, 1, 'exactly one row committed')
        for (const event of committed) {
          expectEqual(event.affected_rows, 1, `committed ${event.op} affected_rows`)
          // Both statements touched the one committed row, and the event
          // says which row that is — through the capture mechanism, not the
          // worker's classification (which is off for native databases).
          expectEqual(event.returning?.length, 1, `committed ${event.op} event names one row`)
          expectEqual(
            String(event.returning?.[0]?.id),
            String(rows.rows[0].id),
            `committed ${event.op} event names the committed row`,
          )
          expect(event.truncated === undefined, 'one key is under the cap')
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
        await observer.dispose()
        fnRef.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

/**
 * Fan-out and filtering on the native path: an all-ops observer and a
 * delete-only observer share one table, and one capture install (the
 * worker counts the observers holding it). After both are disposed,
 * external writes still succeed (orphaned database triggers are inert, not
 * broken) and deliver to no one.
 */
function fanOutOpsCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture fans out, filters ops, and outlives its observers',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_fanout_${target.applies}`
      const allFnId = `harness::native_fanout_all_${target.applies}`
      const deletesFnId = `harness::native_fanout_deletes_${target.applies}`
      const allEvents: RowChangedEvent[] = []
      const deleteEvents: RowChangedEvent[] = []
      const all = sink(allEvents, 'all-ops observer')
      const deletes = sink(deleteEvents, 'delete-only observer')
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
      const allObserver = observeRowChanged(iii, { db: target.nativeDb, table }, allFnId)
      // Second observer on the SAME table: it shares the first one's
      // capture install.
      const deletesFn = iii.registerFunction(
        deletesFnId,
        async (payload: RowChangedEvent) => {
          deleteEvents.push(payload)
          return null
        },
        { description: 'Native-capture fan-out E2E sink (deletes only).' },
      )
      const deletesObserver = observeRowChanged(iii, { db: target.nativeDb, table, ops: ['delete'] }, deletesFnId)

      let observersLive = true
      const disposeObservers = async () => {
        if (!observersLive) return
        observersLive = false
        await allObserver.dispose()
        await deletesObserver.dispose()
      }

      try {
        await Promise.all([allObserver.ready(), deletesObserver.ready()])
        await waitForCaptureReady(call, iii, target, table)

        // Insert (external client): all-ops hears, delete-only does not.
        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)})`,
          params: [10],
        })
        expectEqual((await all.next()).op, 'insert', 'all-ops observer hears insert')
        await sleep(SILENCE_WINDOW_MS)
        deletes.expectDrained()

        // Delete: both hear exactly one event.
        await call('database::execute', {
          db: target.applies,
          sql: `DELETE FROM ${table} WHERE n = ${ph(1)}`,
          params: [10],
        })
        expectEqual((await all.next()).op, 'delete', 'all-ops observer hears delete')
        expectEqual((await deletes.next()).op, 'delete', 'delete-only observer hears delete')

        // Dispose both; external writes still succeed and nobody hears.
        await disposeObservers()
        await sleep(SILENCE_WINDOW_MS)
        const r = await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)})`,
          params: [20],
        })
        expectEqual(r.affected_rows, 1, 'write succeeds after dispose (orphan capture is inert)')
        await sleep(SILENCE_WINDOW_MS)
        all.expectDrained()
        deletes.expectDrained()
      } finally {
        await disposeObservers()
        allFn.unregister()
        deletesFn.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

/**
 * Bulk statements stay single events with true counts: 150 rows inserted,
 * updated, deleted must arrive as exactly three events with
 * affected_rows=150 — never one event per row. Each driver earns this a
 * different way (pg statement-level triggers with transition tables,
 * sqlite run-length coalescing of changelog rows, mysql merging of chunked
 * binlog row events), so proving it end-to-end covers all three coalescers.
 * Past the key cap the event keeps the exact count, carries the first
 * KEY_CAP keys, and says so with `truncated`.
 */
function bulkCoalescingCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture coalesces bulk statements into single events',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_bulk_${target.applies}`
      const fnId = `harness::native_bulk_${target.applies}`
      const events: RowChangedEvent[] = []
      const native = sink(events, 'bulk observer')
      const ROWS = 150
      const expectCapped = (event: RowChangedEvent, label: string): void => {
        expectEqual(event.affected_rows, ROWS, `${label}: the count stays exact past the cap`)
        expectEqual(event.returning?.length, KEY_CAP, `${label}: the first ${KEY_CAP} keys ride the event`)
        expectEqual(event.truncated, true, `${label}: the dropped keys are flagged`)
      }

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
      const observer = observeRowChanged(iii, { db: target.nativeDb, table }, fnId)

      try {
        await observer.ready()
        await waitForCaptureReady(call, iii, target, table)

        const values = Array.from({ length: ROWS }, (_, i) => `(${i})`).join(', ')
        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES ${values}`,
        })
        const inserted = await native.next()
        expectEqual(inserted.op, 'insert', 'bulk insert op')
        expectEqual(inserted.affected_rows, ROWS, 'bulk insert arrives as ONE event')
        expectCapped(inserted, 'bulk insert')

        await call('database::execute', {
          db: target.applies,
          sql: `UPDATE ${table} SET n = n + 1`,
        })
        const updated = await native.next()
        expectEqual(updated.op, 'update', 'bulk update op')
        expectEqual(updated.affected_rows, ROWS, 'bulk update arrives as ONE event')
        expectCapped(updated, 'bulk update')

        await call('database::execute', { db: target.applies, sql: `DELETE FROM ${table}` })
        const deleted = await native.next()
        expectEqual(deleted.op, 'delete', 'bulk delete op')
        expectEqual(deleted.affected_rows, ROWS, 'bulk delete arrives as ONE event')
        expectCapped(deleted, 'bulk delete')

        // Exactly three events total — a per-row implementation would have
        // flooded 450.
        await sleep(SILENCE_WINDOW_MS)
        native.expectDrained()
      } finally {
        await observer.dispose()
        fnRef.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

/**
 * An observer names its table canonically (bare, unquoted, lowercase) on
 * every driver: an uppercase spelling is refused when it is created
 * (`NON_CANONICAL_TABLE`) and installs nothing, so it can never add a second
 * capture set. An exact-spelling sentinel observer proves events flow, so the
 * single event below is never vacuous.
 */
function caseSensitivityCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture refuses a non-canonical table spelling',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_case_${target.applies}`
      const exactFnId = `harness::native_case_exact_${target.applies}`
      const upperFnId = `harness::native_case_upper_${target.applies}`
      const exactEvents: RowChangedEvent[] = []
      const upperEvents: RowChangedEvent[] = []
      const exact = sink(exactEvents, 'exact-spelling observer')
      const upper = sink(upperEvents, 'uppercase observer')
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
        { description: 'Exact-spelling native observer (sentinel).' },
      )
      const exactObserver = observeRowChanged(iii, { db: target.nativeDb, table }, exactFnId)
      const upperFn = iii.registerFunction(
        upperFnId,
        async (payload: RowChangedEvent) => {
          upperEvents.push(payload)
          return null
        },
        { description: 'Uppercase-spelled native observer.' },
      )
      const upperObserver = observeRowChanged(iii, { db: target.nativeDb, table: table.toUpperCase() }, upperFnId)

      try {
        await exactObserver.ready()
        expectEqual(await refusalCode(upperObserver), 'NON_CANONICAL_TABLE', 'the uppercase spelling is refused')
        await waitForCaptureReady(call, iii, target, table)

        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)})`,
          params: [1],
        })
        const heard = await exact.next()
        expectEqual(heard.op, 'insert', 'exact-spelling observer hears the write')
        // Exactly one event, ever: one capture set per table.
        await sleep(SILENCE_WINDOW_MS)
        exact.expectDrained()
        upper.expectDrained()
      } finally {
        await upperObserver.dispose().catch(() => {})
        upperFn.unregister()
        await exactObserver.dispose()
        exactFn.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

/**
 * Two tables, two observers, no cross-talk: per-table capture installation
 * and per-observer keys keep each observer scoped to its own table.
 */
function twoTableIsolationCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture isolates observers per table',
    applies: [target.applies],
    async run({ call, iii }) {
      const tableA = `e2e_native_iso_a_${target.applies}`
      const tableB = `e2e_native_iso_b_${target.applies}`
      const fnA = `harness::native_iso_a_${target.applies}`
      const fnB = `harness::native_iso_b_${target.applies}`
      const eventsA: RowChangedEvent[] = []
      const eventsB: RowChangedEvent[] = []
      const sinkA = sink(eventsA, 'table-A observer')
      const sinkB = sink(eventsB, 'table-B observer')
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
      const observerA = observeRowChanged(iii, { db: target.nativeDb, table: tableA }, fnA)
      const fnBRef = iii.registerFunction(
        fnB,
        async (payload: RowChangedEvent) => {
          eventsB.push(payload)
          return null
        },
        { description: 'Table-B isolation E2E sink.' },
      )
      const observerB = observeRowChanged(iii, { db: target.nativeDb, table: tableB }, fnB)

      try {
        await Promise.all([observerA.ready(), observerB.ready()])
        await waitForCaptureReady(call, iii, target, tableA)
        await waitForCaptureReady(call, iii, target, tableB)

        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${tableA} (n) VALUES (${ph(1)})`,
          params: [1],
        })
        expectEqual((await sinkA.next()).op, 'insert', 'A observer hears table A')
        await sleep(SILENCE_WINDOW_MS)
        sinkB.expectDrained()

        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${tableB} (n) VALUES (${ph(1)})`,
          params: [2],
        })
        expectEqual((await sinkB.next()).op, 'insert', 'B observer hears table B')
        await sleep(SILENCE_WINDOW_MS)
        sinkA.expectDrained()
      } finally {
        await observerA.dispose()
        fnARef.unregister()
        await observerB.dispose()
        fnBRef.unregister()
        for (const table of [tableA, tableB]) {
          await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
        }
      }
    },
  }
}

/**
 * Observing a table that does not exist: pg/sqlite refuse the observer at
 * DDL install (`CAPTURE_INSTALL_FAILED`) — and stay silent even after the
 * table is created, proving the rejection was final, not deferred. mysql has
 * nothing to install, so the observer is accepted and starts hearing the
 * moment the table appears in the binlog. The asymmetry is contract; pin
 * both sides.
 */
function missingTableRegistrationCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture observer of a missing table behaves per driver contract',
    applies: [target.applies],
    async run({ call, iii }) {
      const table = `e2e_native_missing_${target.applies}`
      const fnId = `harness::native_missing_${target.applies}`
      const events: RowChangedEvent[] = []
      const native = sink(events, 'missing-table observer')
      const ph = target.ph

      await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      const fnRef = iii.registerFunction(
        fnId,
        async (payload: RowChangedEvent) => {
          events.push(payload)
          return null
        },
        { description: 'Missing-table observer E2E sink.' },
      )
      const observer = observeRowChanged(iii, { db: target.nativeDb, table }, fnId)

      try {
        // The observer settles (its DDL install fails on pg/sqlite) BEFORE
        // the table exists — creating it too early would let the install
        // succeed and test nothing.
        const refusal = await refusalCode(observer)
        if (target.missingTableObserverAccepted) {
          expectEqual(refusal, undefined, 'the observer of a missing table is accepted')
        } else {
          expectEqual(refusal, 'CAPTURE_INSTALL_FAILED', 'the observer of a missing table is refused')
        }

        await call('database::execute', {
          db: target.nativeDb,
          sql: `CREATE TABLE ${table} (id ${target.idColumnDDL}, n INT NOT NULL)`,
        })
        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (n) VALUES (${ph(1)})`,
          params: [1],
        })

        if (target.missingTableObserverAccepted) {
          const heard = await native.next()
          expectEqual(heard.op, 'insert', 'accepted observer hears once the table exists')
        } else {
          await sleep(SILENCE_WINDOW_MS)
          native.expectDrained()
        }
      } finally {
        await observer.dispose()
        fnRef.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

/**
 * Key shapes beyond the auto-increment id: a composite primary key arrives
 * with every key column, and a table without a primary key arrives as the
 * count-only event it always was — no rowid, no `truncated`, nothing
 * invented. Uniform across the three capture mechanisms.
 */
function keyShapesCase(target: NativeTarget): TestCase {
  return {
    name: 'native capture reports composite keys and nothing for keyless tables',
    applies: [target.applies],
    async run({ call, iii }) {
      const composite = `e2e_native_keys_${target.applies}`
      const keyless = `e2e_native_keyless_${target.applies}`
      const compositeFnId = `harness::native_keys_${target.applies}`
      const keylessFnId = `harness::native_keyless_${target.applies}`
      const compositeEvents: RowChangedEvent[] = []
      const keylessEvents: RowChangedEvent[] = []
      const compositeSink = sink(compositeEvents, 'composite-key observer')
      const keylessSink = sink(keylessEvents, 'keyless observer')
      const ph = target.ph

      for (const t of [composite, keyless]) {
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${t}` })
      }
      // VARCHAR, not TEXT: mysql refuses a TEXT column in a key without a length.
      await call('database::execute', {
        db: target.nativeDb,
        sql: `CREATE TABLE ${composite} (a INT NOT NULL, b VARCHAR(32) NOT NULL, n INT, PRIMARY KEY (a, b))`,
      })
      await call('database::execute', {
        db: target.nativeDb,
        sql: `CREATE TABLE ${keyless} (n INT NOT NULL)`,
      })
      const compositeFn = iii.registerFunction(
        compositeFnId,
        async (payload: RowChangedEvent) => {
          compositeEvents.push(payload)
          return null
        },
        { description: 'Composite-key native capture E2E sink.' },
      )
      const compositeObserver = observeRowChanged(iii, { db: target.nativeDb, table: composite }, compositeFnId)
      const keylessFn = iii.registerFunction(
        keylessFnId,
        async (payload: RowChangedEvent) => {
          keylessEvents.push(payload)
          return null
        },
        { description: 'Keyless-table native capture E2E sink.' },
      )
      const keylessObserver = observeRowChanged(iii, { db: target.nativeDb, table: keyless }, keylessFnId)

      try {
        await Promise.all([compositeObserver.ready(), keylessObserver.ready()])
        await waitForCaptureReady(call, iii, target, composite)
        await waitForCaptureReady(call, iii, target, keyless)

        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${composite} (a, b, n) VALUES (${ph(1)}, ${ph(2)}, ${ph(3)})`,
          params: [1, 'x', 0],
        })
        const keyed = await compositeSink.next()
        expectEqual(keyed.op, 'insert', 'composite insert op')
        expectEqual(keyed.affected_rows, 1, 'composite insert count')
        // Both key columns, nothing else (n stays out); INT is a number on
        // every driver, so no keyOf is needed here.
        expectEqual(keyed.returning, [{ a: 1, b: 'x' }], 'composite key carries every key column')
        expect(keyed.truncated === undefined, 'one row is under the cap')

        await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${keyless} (n) VALUES (${ph(1)})`,
          params: [7],
        })
        const unkeyed = await keylessSink.next()
        expectEqual(unkeyed.op, 'insert', 'keyless insert op')
        expectEqual(unkeyed.affected_rows, 1, 'keyless insert count')
        expect(unkeyed.returning === undefined, 'no primary key, no identity — not a rowid either')
        expect(unkeyed.truncated === undefined, 'no keys is not truncation')

        await sleep(SILENCE_WINDOW_MS)
        compositeSink.expectDrained()
        keylessSink.expectDrained()
      } finally {
        await compositeObserver.dispose()
        compositeFn.unregister()
        await keylessObserver.dispose()
        keylessFn.unregister()
        for (const t of [composite, keyless]) {
          await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${t}` })
        }
      }
    },
  }
}

/**
 * The postgres trigger builds its NOTIFY payload with a byte budget: an
 * 8000-byte payload is refused by `pg_notify`, and inside an AFTER trigger
 * that error would abort the WRITER's statement. So the trigger halves the
 * key count until the payload fits. Twenty ~1KB text keys cannot all fit —
 * the write must succeed, the count must stay exact, and the event must say
 * it dropped keys rather than pretend the list is complete.
 */
function pgByteCapCase(): TestCase {
  return {
    name: 'native capture trims postgres keys to the NOTIFY payload budget',
    applies: ['pg_db'],
    async run({ call, iii }) {
      const target = TARGETS.find((t) => t.applies === 'pg_db')!
      const table = 'e2e_native_pg_bytes'
      const fnId = 'harness::native_pg_bytes'
      const events: RowChangedEvent[] = []
      const native = sink(events, 'pg byte-cap observer')

      await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      await call('database::execute', {
        db: target.nativeDb,
        sql: `CREATE TABLE ${table} (k TEXT PRIMARY KEY, n INT)`,
      })
      const fnRef = iii.registerFunction(
        fnId,
        async (payload: RowChangedEvent) => {
          events.push(payload)
          return null
        },
        { description: 'Postgres NOTIFY byte-budget E2E sink.' },
      )
      const observer = observeRowChanged(iii, { db: target.nativeDb, table }, fnId)

      try {
        await observer.ready()
        await waitForCaptureReady(call, iii, target, table)
        const rows = Array.from({ length: 20 }, (_, i) => `('${'x'.repeat(1000)}${i}', ${i})`)
        const written = await call('database::execute', {
          db: target.applies,
          sql: `INSERT INTO ${table} (k, n) VALUES ${rows.join(', ')}`,
        })
        expectEqual(written.affected_rows, 20, 'the write itself is never aborted by the cap')

        const event = await native.next()
        expectEqual(event.op, 'insert', 'pg byte-cap op')
        expectEqual(event.affected_rows, 20, 'the count stays exact')
        const keys = event.returning ?? []
        expect(
          keys.length > 0 && keys.length < 20,
          `some keys fit and not all (got ${keys.length} of 20)`,
        )
        expectEqual(event.truncated, true, 'the dropped keys are flagged')
      } finally {
        await observer.dispose()
        fnRef.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

/**
 * Upgrade path: a changelog written by the pre-keys worker has three
 * columns, and the new triggers write a fourth. The install must migrate the
 * table in place — a trigger writing `key` into a three-column changelog
 * would fail the WRITER's statement, not ours.
 *
 * The AUTOINCREMENT sequence is pushed past the watcher's cursor on purpose:
 * the watcher resumes at `MAX(id)` and a recreated table restarts ids at 1,
 * so without the jump the new rows would land below the cursor and never be
 * drained — a test that could pass for the wrong reason.
 */
function sqliteLegacyChangelogCase(): TestCase {
  return {
    name: 'native capture migrates a pre-keys sqlite changelog in place',
    applies: ['sqlite_db'],
    async run({ call, iii }) {
      const target = TARGETS.find((t) => t.applies === 'sqlite_db')!
      const table = 'e2e_native_legacy'
      const fnId = 'harness::native_legacy'
      const events: RowChangedEvent[] = []
      const native = sink(events, 'legacy-changelog observer')

      await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      // Simulate the changelog as the previous worker left it: no `key`.
      await call('database::execute', {
        db: target.nativeDb,
        sql: 'DROP TABLE IF EXISTS _iii_row_changes',
      })
      await call('database::execute', {
        db: target.nativeDb,
        sql: 'CREATE TABLE _iii_row_changes (id INTEGER PRIMARY KEY AUTOINCREMENT, tbl TEXT NOT NULL, op TEXT NOT NULL)',
      })
      await call('database::execute', {
        db: target.nativeDb,
        sql: "INSERT INTO _iii_row_changes (tbl, op) VALUES ('legacy-seed', 'insert')",
      })
      await call('database::execute', {
        db: target.nativeDb,
        sql: "UPDATE sqlite_sequence SET seq = 10000000 WHERE name = '_iii_row_changes'",
      })
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
        { description: 'Legacy-changelog migration E2E sink.' },
      )
      const observer = observeRowChanged(iii, { db: target.nativeDb, table }, fnId)

      try {
        await observer.ready()
        // The install is what migrates the changelog (ALTER … ADD COLUMN).
        // If it had not, this INSERT would fail in the trigger and the
        // capture would be silently absent — the event below is the proof.
        await waitForCaptureReady(call, iii, target, table)
        await call('database::execute', {
          db: target.nativeDb,
          sql: `INSERT INTO ${table} (n) VALUES (${target.ph(1)})`,
          params: [1],
        })
        const event = await native.next()
        expectEqual(event.op, 'insert', 'legacy-changelog op')
        expectEqual(event.affected_rows, 1, 'legacy-changelog count')
        expectEqual(
          event.returning,
          [{ id: target.keyOf(1) }],
          'keys flow through a migrated changelog',
        )
      } finally {
        await observer.dispose()
        fnRef.unregister()
        await call('database::execute', { db: target.nativeDb, sql: `DROP TABLE IF EXISTS ${table}` })
      }
    },
  }
}

export const NATIVE_CAPTURE_CASES: TestCase[] = [
  ...TARGETS.flatMap((target) => [
    crossClientCase(target),
    tablelessRejectionCase(target),
    txGatingCase(target),
    fanOutOpsCase(target),
    bulkCoalescingCase(target),
    caseSensitivityCase(target),
    twoTableIsolationCase(target),
    missingTableRegistrationCase(target),
    keyShapesCase(target),
  ]),
  pgByteCapCase(),
  sqliteLegacyChangelogCase(),
]

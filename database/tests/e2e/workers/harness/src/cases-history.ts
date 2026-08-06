import type { ISdk } from 'iii-sdk'
import type { TestCase, CaseContext } from './cases.ts'
import { expect, expectEqual } from './cases.ts'

/**
 * Query-history cap cases (MOT-4372). The worker stores console history on
 * the `state` worker; uncapped, one value grew to ~8MB — large enough that
 * serving it reset the state worker's engine connection and unregistered
 * `state::*` for the whole stack.
 *
 * No real state worker runs in this stack, and that is the point: the
 * harness registers `state::get` / `state::set` / `state::update` itself,
 * which both observes every write the worker makes and scripts the failure
 * modes a real state worker cannot safely reproduce (a value too large to
 * serve kills the very connection that would deliver it).
 *
 * Caps under test come from the seeded e2e config (database-config.ts):
 * `history_max_entries: 5`, `history_max_bytes: 8192`. The write path is
 * driver-agnostic, so the group runs on sqlite only.
 */

const MAX_ENTRIES = 5
const MAX_BYTES = 8192
const SCOPE = 'database'
const READY_TIMEOUT_MS = 10_000
const WRITE_TIMEOUT_MS = 5_000

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))
const jsonBytes = (v: unknown) => Buffer.byteLength(JSON.stringify(v))
const historyKey = (driver: string) => `${SCOPE}/history:${driver}`

interface StoredEntry {
  sql: string
  verb: string
  duration_ms?: number
  row_count?: number
  at: string
}

interface KeyRef {
  scope: string
  key: string
}

interface MockState {
  store: Map<string, unknown>
  /** `state::get` attempts per key, counted before any scripted throw. */
  gets: Map<string, number>
  /** Every `state::set` attempt in arrival order, including scripted failures. */
  setAttempts: Array<{ key: string; failed: boolean }>
  /** `state::update` invocations — must stay 0; history's old uncapped append lived there. */
  updates: number
  /** Keys whose `state::get` throws, standing in for a value too large to serve. */
  getThrows: Set<string>
  /** Upcoming `state::set` calls to fail before succeeding again. */
  setFailuresRemaining: number
  unregister(): void
}

function registerMockState(iii: ISdk): MockState {
  const mock: MockState = {
    store: new Map(),
    gets: new Map(),
    setAttempts: [],
    updates: 0,
    getThrows: new Set(),
    setFailuresRemaining: 0,
    unregister: () => {},
  }
  const k = (p: KeyRef) => `${p.scope}/${p.key}`

  const getRef = iii.registerFunction(
    'state::get',
    async (p: KeyRef) => {
      const key = k(p)
      mock.gets.set(key, (mock.gets.get(key) ?? 0) + 1)
      if (mock.getThrows.has(key)) {
        throw new Error('MOCK_UNSERVABLE: value too large to serve')
      }
      return mock.store.get(key) ?? null
    },
    { description: 'Harness mock state::get (history-cap e2e).' },
  )
  const setRef = iii.registerFunction(
    'state::set',
    async (p: KeyRef & { value: unknown }) => {
      const key = k(p)
      const failed = mock.setFailuresRemaining > 0
      mock.setAttempts.push({ key, failed })
      if (failed) {
        mock.setFailuresRemaining -= 1
        throw new Error('MOCK_SET_DOWN: scripted state::set failure')
      }
      mock.store.set(key, p.value)
      return null
    },
    { description: 'Harness mock state::set (history-cap e2e).' },
  )
  const updateRef = iii.registerFunction(
    'state::update',
    async () => {
      mock.updates += 1
      throw new Error('MOCK_NO_UPDATE: history must not use state::update')
    },
    { description: 'Harness mock state::update — history writes must never land here.' },
  )
  mock.unregister = () => {
    getRef.unregister()
    setRef.unregister()
    updateRef.unregister()
  }
  return mock
}

/** Wait until the engine routes `state::*` to the mock, then zero the counters. */
async function mockReady(ctx: CaseContext, mock: MockState): Promise<void> {
  const deadline = Date.now() + READY_TIMEOUT_MS
  const probe = { scope: SCOPE, key: 'harness:probe' }
  for (;;) {
    try {
      await ctx.call('state::get', probe)
      await ctx.call('state::set', { ...probe, value: [] })
      break
    } catch (e) {
      if (Date.now() > deadline) throw new Error(`mock state::get/set not reachable: ${e}`)
      await sleep(50)
    }
  }
  // `state::update` always throws its own marker; routed once we see it.
  for (;;) {
    try {
      await ctx.call('state::update', { ...probe, ops: [] })
      throw new Error('mock state::update resolved — it must throw')
    } catch (e: any) {
      const msg = String(e?.message ?? e)
      if (msg.includes('must throw')) throw e
      if (msg.includes('MOCK_NO_UPDATE')) break
      if (Date.now() > deadline) throw new Error(`mock state::update not reachable: ${msg}`)
      await sleep(50)
    }
  }
  mock.store.clear()
  mock.gets.clear()
  mock.setAttempts.length = 0
  mock.updates = 0
}

/**
 * Run one recorded query and wait for its (fire-and-forget) history write
 * attempt to reach the mock — success or scripted failure — so consecutive
 * queries never interleave their read-modify-write cycles.
 */
async function recordedQuery(ctx: CaseContext, mock: MockState, sql: string): Promise<void> {
  const before = mock.setAttempts.length
  await ctx.call('database::query', { db: ctx.driver, sql })
  const deadline = Date.now() + WRITE_TIMEOUT_MS
  while (mock.setAttempts.length === before) {
    // Name the regression rather than waiting out the timeout: the pre-fix
    // worker appended through state::update, which is both uncapped and
    // exempt from the state worker's size guard.
    if (mock.updates > 0) {
      throw new Error('history wrote via state::update (uncapped append) instead of state::set')
    }
    if (Date.now() > deadline) {
      throw new Error(`history write for ${JSON.stringify(sql)} never reached state::set`)
    }
    await sleep(20)
  }
}

/**
 * One recorded query whose write lands successfully. Earlier suites run
 * their queries with no `state::*` registered at all, which latches the
 * worker's replace-without-reading recovery; a successful write clears the
 * latch and leaves the store holding exactly this query's entry.
 */
async function flushHistory(ctx: CaseContext, mock: MockState): Promise<void> {
  await recordedQuery(ctx, mock, 'SELECT 0')
  const last = mock.setAttempts[mock.setAttempts.length - 1]
  expect(last !== undefined && !last.failed, 'flush write should succeed')
}

function storedHistory(mock: MockState, driver: string): StoredEntry[] {
  const v = mock.store.get(historyKey(driver))
  expect(Array.isArray(v), `stored history is an array, got: ${JSON.stringify(v)?.slice(0, 200)}`)
  return v as StoredEntry[]
}

export const HISTORY_CASES: TestCase[] = [
  {
    name: 'history entry cap rotates oldest entries out',
    applies: ['sqlite_db'],
    async run(ctx) {
      const mock = registerMockState(ctx.iii)
      try {
        await mockReady(ctx, mock)
        await flushHistory(ctx, mock)
        for (let i = 1; i <= 8; i++) {
          await recordedQuery(ctx, mock, `SELECT ${100 + i}`)
        }
        const stored = storedHistory(mock, ctx.driver)
        expectEqual(stored.length, MAX_ENTRIES, 'stored entry count == history_max_entries')
        expectEqual(
          stored.map((e) => e.sql),
          ['SELECT 104', 'SELECT 105', 'SELECT 106', 'SELECT 107', 'SELECT 108'],
          'newest entries kept, oldest rotated out',
        )
        expect(jsonBytes(stored) <= MAX_BYTES, 'stored value within history_max_bytes')
        // Read path through the real engine round-trips the same tail.
        const h = await ctx.call('database::history', { db: ctx.driver })
        expectEqual(h.count, MAX_ENTRIES, 'database::history count')
        expectEqual(h.entries[0].sql, 'SELECT 108', 'database::history newest first')
        expectEqual(mock.updates, 0, 'state::update never used')
      } finally {
        mock.unregister()
      }
    },
  },
  {
    name: 'history byte cap trims fat entries to fit',
    applies: ['sqlite_db'],
    async run(ctx) {
      const mock = registerMockState(ctx.iii)
      try {
        await mockReady(ctx, mock)
        await flushHistory(ctx, mock)
        // ~3KB per entry: two fit under the 8KB byte cap, three never do —
        // while the 5-entry cap alone would happily keep them all.
        const pad = `/* ${'x'.repeat(3000)} */`
        for (let i = 1; i <= 4; i++) {
          await recordedQuery(ctx, mock, `SELECT ${200 + i} ${pad}`)
          expect(
            jsonBytes(storedHistory(mock, ctx.driver)) <= MAX_BYTES,
            `write ${i}: stored value within history_max_bytes`,
          )
        }
        const stored = storedHistory(mock, ctx.driver)
        expectEqual(stored.length, 2, 'byte cap binds before the entry cap')
        expectEqual(
          stored.map((e) => e.sql.slice(0, 10)),
          ['SELECT 203', 'SELECT 204'],
          'newest fat entries kept',
        )
        expectEqual(mock.updates, 0, 'state::update never used')
      } finally {
        mock.unregister()
      }
    },
  },
  {
    name: 'oversized stored history is trimmed to caps on the next write',
    applies: ['sqlite_db'],
    async run(ctx) {
      const mock = registerMockState(ctx.iii)
      try {
        await mockReady(ctx, mock)
        await flushHistory(ctx, mock)
        // ~500KB of readable pre-cap backlog — what a stack upgraded from
        // the uncapped worker wakes up with (short of transport-fatal).
        const backlog = Array.from({ length: 500 }, (_, i) => ({
          sql: `SELECT ${i} /* ${'y'.repeat(950)} */`,
          verb: 'select',
          at: '2026-01-01T00:00:00+00:00',
        }))
        mock.store.set(historyKey(ctx.driver), backlog)
        await recordedQuery(ctx, mock, 'SELECT 999')
        const stored = storedHistory(mock, ctx.driver)
        expectEqual(stored.length, MAX_ENTRIES, 'backlog trimmed to the entry cap')
        expect(jsonBytes(stored) <= MAX_BYTES, 'backlog trimmed to the byte cap')
        expectEqual(stored[stored.length - 1]?.sql, 'SELECT 999', 'new entry survives the trim')
        expectEqual(mock.updates, 0, 'state::update never used')
      } finally {
        mock.unregister()
      }
    },
  },
  {
    name: 'unreadable stored history is replaced blind without re-reading',
    applies: ['sqlite_db'],
    async run(ctx) {
      const mock = registerMockState(ctx.iii)
      try {
        await mockReady(ctx, mock)
        await flushHistory(ctx, mock)
        const hk = historyKey(ctx.driver)
        // Script the outage: reads of this key die (in production the ~8MB
        // value resets the connection that would serve it), and the first
        // write after the failed read dies with it.
        mock.getThrows.add(hk)
        mock.setFailuresRemaining = 1
        const getsBefore = mock.gets.get(hk) ?? 0

        // First write: read fails, worker latches replace-without-reading,
        // and the blind write is scripted to fail so the latch survives.
        await recordedQuery(ctx, mock, 'SELECT 301')
        const firstAttempt = mock.setAttempts[mock.setAttempts.length - 1]
        expect(firstAttempt !== undefined && firstAttempt.failed, 'first write hit the scripted set failure')
        expectEqual((mock.gets.get(hk) ?? 0) - getsBefore, 1, 'exactly one read attempt so far')

        // Second write: no re-read of the poisoned key — straight to a small
        // replacement value. This is the self-heal: the value that broke the
        // connection is never round-tripped again.
        await recordedQuery(ctx, mock, 'SELECT 302')
        expectEqual((mock.gets.get(hk) ?? 0) - getsBefore, 1, 'poisoned key not re-read')
        const stored = storedHistory(mock, ctx.driver)
        expectEqual(
          stored.map((e) => e.sql),
          ['SELECT 302'],
          'replacement value holds only the new entry',
        )
        expect(jsonBytes(stored) <= MAX_BYTES, 'replacement value within caps')
        expectEqual(mock.updates, 0, 'state::update never used')

        // Back to normal reads; leave the worker un-latched for whatever runs next.
        mock.getThrows.delete(hk)
        await recordedQuery(ctx, mock, 'SELECT 303')
      } finally {
        mock.unregister()
      }
    },
  },
]

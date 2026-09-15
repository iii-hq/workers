/**
 * Catalog-surface cases: every `database::*` function the rest of the suite
 * never calls gets a minimal real call here, asserting the declared envelope
 * rather than exact values. The failure this guards against is a function
 * that breaks wholesale on one driver — the postgres `"char"` decode bug
 * took `listTables`/`describeTable` down on every server for weeks (MOT-4361)
 * with nothing in this suite noticing.
 *
 * The last case is the guard: the `database::*` ids the engine reports must
 * equal DATABASE_FUNCTIONS below, so adding a function to the worker without
 * covering it here fails the suite.
 */

import type { ISdk } from 'iii-sdk'
import type { TestCase, CaseContext } from './cases.ts'
import { expect, expectEqual } from './cases.ts'

/** Every `database::*` id the worker registers. Keep in step with the guard case. */
const DATABASE_FUNCTIONS = [
  'database::beginTransaction',
  'database::browseTable',
  'database::columnStats',
  'database::commitTransaction',
  'database::deleteSavedQuery',
  'database::describeSchema',
  'database::describeTable',
  'database::execute',
  'database::executeBatch',
  'database::explain',
  'database::getTableView',
  'database::health',
  'database::history',
  'database::listDatabases',
  'database::listSavedQueries',
  'database::listTables',
  'database::prepareStatement',
  'database::query',
  'database::rollbackTransaction',
  'database::runStatement',
  'database::saveQuery',
  'database::saveTableView',
  'database::schemaDiagram',
  'database::terminateQuery',
  'database::testConnection',
  'database::transaction',
  'database::transactionExecute',
  'database::transactionQuery',
] as const

const TABLE = 'e2e_catalog'
const VIEW = 'e2e_catalog_v'
const READY_TIMEOUT_MS = 10_000

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

/**
 * A minimal `state::*` KV — saved queries and table views live on the state
 * worker, which does not run in this stack. Same approach as cases-history.ts:
 * register the two functions the worker calls, then wait until the engine
 * routes to them before exercising the handlers that depend on them.
 */
function registerMockState(iii: ISdk): { unregister: () => void } {
  const store = new Map<string, unknown>()
  const k = (p: { scope: string; key: string }) => `${p.scope}/${p.key}`
  const getRef = iii.registerFunction(
    'state::get',
    async (p: { scope: string; key: string }) => store.get(k(p)) ?? null,
    { description: 'Harness mock state::get (catalog e2e).' },
  )
  const setRef = iii.registerFunction(
    'state::set',
    async (p: { scope: string; key: string; value: unknown }) => {
      store.set(k(p), p.value)
      return null
    },
    { description: 'Harness mock state::set (catalog e2e).' },
  )
  return {
    unregister: () => {
      getRef.unregister()
      setRef.unregister()
    },
  }
}

/** Wait until `state::*` routes to the mock, or fail with why. */
async function mockStateReady(ctx: CaseContext): Promise<void> {
  const probe = { scope: 'database', key: 'harness:probe', value: 1 }
  const deadline = Date.now() + READY_TIMEOUT_MS
  for (;;) {
    try {
      await ctx.call('state::set', probe)
      await ctx.call('state::get', { scope: probe.scope, key: probe.key })
      return
    } catch (e) {
      if (Date.now() > deadline) throw new Error(`mock state::get/set not reachable: ${e}`)
      await sleep(50)
    }
  }
}

export const CATALOG_CASES: TestCase[] = [
  {
    name: 'catalog lists tables and describes their shape',
    async run({ driver, dialect, call }) {
      await call('database::execute', { db: driver, sql: `DROP VIEW IF EXISTS ${VIEW}` })
      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${TABLE} (id ${dialect.idColumnDDL()}, label VARCHAR(50), qty INT)`,
      })
      await call('database::execute', {
        db: driver,
        sql: `CREATE VIEW ${VIEW} AS SELECT id, label FROM ${TABLE}`,
      })
      try {
        const tables = await call('database::listTables', { db: driver })
        expectEqual(tables.count, tables.tables.length, 'listTables count matches the rows')
        const names = tables.tables.map((t: any) => String(t.name).toLowerCase())
        expect(names.includes(TABLE), `listTables finds ${TABLE}: ${JSON.stringify(names)}`)
        expect(names.includes(VIEW), 'listTables sees the view')
        const view = tables.tables.find((t: any) => String(t.name).toLowerCase() === VIEW)
        expectEqual(String(view.kind), 'view', 'view rows carry kind=view')

        const desc = await call('database::describeTable', { db: driver, table: TABLE })
        expectEqual(String(desc.table).toLowerCase(), TABLE, 'describeTable echoes the table')
        expectEqual(desc.columns.length, 3, 'three columns')
        const id = desc.columns.find((c: any) => c.name === 'id')
        expect(id !== undefined && id.primary_key === true, 'id is the primary key')
        expect(
          desc.columns.every((c: any) => typeof c.type === 'string' && c.type.length > 0),
          `every column reports a type: ${JSON.stringify(desc.columns)}`,
        )
        expect(Array.isArray(desc.indexes), 'indexes array present')

        const schema = await call('database::describeSchema', { db: driver, tables: [TABLE] })
        expectEqual(schema.count, 1, 'describeSchema filtered to one table')
        expectEqual(schema.tables[0].columns.length, 3, 'describeSchema carries the columns')

        const diagram = await call('database::schemaDiagram', { db: driver })
        const node = diagram.nodes.find(
          (n: any) => String(n.table).toLowerCase() === TABLE,
        )
        expect(node !== undefined, 'schemaDiagram places the table')
        expectEqual(node.columns.length, 3, 'the diagram node carries the columns')
        expect(Array.isArray(diagram.edges), 'edges array')
        expect(Array.isArray(diagram.isolated), 'isolated array')
      } finally {
        await call('database::execute', { db: driver, sql: `DROP VIEW IF EXISTS ${VIEW}` })
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      }
    },
  },
  {
    name: 'catalog browses and profiles rows without SQL',
    async run({ driver, dialect, call }) {
      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${TABLE} (id ${dialect.idColumnDDL()}, label VARCHAR(50), qty INT)`,
      })
      const p1 = dialect.placeholder(1)
      const p2 = dialect.placeholder(2)
      await call('database::execute', {
        db: driver,
        sql: `INSERT INTO ${TABLE} (label, qty) VALUES (${p1}, ${p2})`,
        params: ['a', 10],
      })
      await call('database::execute', {
        db: driver,
        sql: `INSERT INTO ${TABLE} (label, qty) VALUES (${p1}, ${p2})`,
        params: ['b', 20],
      })
      await call('database::execute', {
        db: driver,
        sql: `INSERT INTO ${TABLE} (label, qty) VALUES (${p1}, ${p2})`,
        params: ['c', 30],
      })
      try {
        const first = await call('database::browseTable', { db: driver, table: TABLE, page_size: 2 })
        expectEqual(first.rows.length, 2, 'page holds page_size rows')
        expectEqual(first.page_size, 2, 'page_size echoed')
        expect(first.has_more === true, 'has_more on a partial page')
        expectEqual(first.total, 3, 'total honours no filters')

        const second = await call('database::browseTable', {
          db: driver,
          table: TABLE,
          page: 1,
          page_size: 2,
        })
        expectEqual(second.rows.length, 1, 'second page holds the rest')
        expect(second.has_more === false, 'no more pages')

        const filtered = await call('database::browseTable', {
          db: driver,
          table: TABLE,
          filters: [{ column: 'qty', op: 'equals', value: 20 }],
        })
        expectEqual(filtered.total, 1, 'total honours filters')
        expectEqual(Number(filtered.rows[0].qty), 20, 'the filtered row')

        const stats = await call('database::columnStats', {
          db: driver,
          table: TABLE,
          columns: ['qty'],
        })
        expectEqual(String(stats.table).toLowerCase(), TABLE, 'columnStats echoes the table')
        expectEqual(stats.columns.length, 1, 'one profiled column')
        expectEqual(stats.columns[0].name, 'qty', 'the profiled column is the one asked for')
        expect(typeof stats.approximate === 'boolean', 'approximate flag present')
      } finally {
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      }
    },
  },
  {
    name: 'catalog explains a read without running it',
    async run({ driver, dialect, call }) {
      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${TABLE} (id ${dialect.idColumnDDL()}, qty INT)`,
      })
      try {
        const plan = await call('database::explain', {
          db: driver,
          sql: `SELECT * FROM ${TABLE} WHERE id = 1`,
        })
        expect(typeof plan.format === 'string' && plan.format.length > 0, 'a plan format is named')
        expectEqual(plan.analyzed, false, 'analyze defaults off')
        expect(
          (plan.root ?? null) !== null || (plan.raw ?? null) !== null,
          'a parsed tree or the raw driver output is present',
        )
        expect(Array.isArray(plan.warnings), 'warnings array')
      } finally {
        await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      }
    },
  },
  {
    name: 'catalog reports health per section and refuses impossible terminate ids',
    async run({ driver, call, expectError }) {
      const health = await call('database::health', { db: driver })
      expectEqual(health.db, driver, 'health echoes the handle')
      expect(typeof health.driver === 'string' && health.driver.length > 0, 'driver named')
      expect(health.pool.max > 0, 'pool section carries the ceiling')
      for (const section of ['active_queries', 'table_sizes', 'locks', 'cache']) {
        const status = String(health[section]?.status)
        expect(
          ['available', 'unsupported', 'denied'].includes(status),
          `${section} labels its status (got ${JSON.stringify(health[section])})`,
        )
      }

      // The backend id lands in SQL, so anything but a bare integer is
      // refused before it reaches the driver.
      await expectError(
        () => call('database::terminateQuery', { db: driver, id: 'not-a-number' }),
        'INVALID_PARAM',
      )
      const after = await call('database::query', { db: driver, sql: 'SELECT 1 AS ok' })
      expectEqual(Number(after.rows[0].ok), 1, 'worker still serving after a refused terminate')
    },
  },
  {
    name: 'catalog lists configured databases with credentials redacted',
    async run({ driver, call }) {
      const listed = await call('database::listDatabases', {})
      expectEqual(listed.count, listed.databases.length, 'count matches rows')
      const entry = listed.databases.find((d: any) => d.name === driver)
      expect(entry !== undefined, `${driver} is listed`)
      expect(typeof entry.url === 'string' && entry.url.length > 0, 'redacted url present')
      expect(!entry.url.includes('iii:iii'), `no credential in the url: ${entry.url}`)
      expect(entry.pool.max > 0, 'pool settings ride along')
      expect(typeof entry.tls.mode === 'string', 'tls mode rides along')
    },
  },
  {
    name: 'catalog round-trips saved queries through the state worker',
    applies: ['sqlite_db'],
    async run(ctx) {
      const mock = registerMockState(ctx.iii)
      try {
        await mockStateReady(ctx)
        const saved = await ctx.call('database::saveQuery', {
          db: ctx.driver,
          name: 'catalog_roundtrip',
          sql: 'SELECT 1',
        })
        expect(typeof saved.id === 'string' && saved.id.length > 0, 'save returns an id')
        expectEqual(saved.replaced, false, 'first save is not a replacement')

        const again = await ctx.call('database::saveQuery', {
          db: ctx.driver,
          name: 'catalog_roundtrip',
          sql: 'SELECT 2',
        })
        expectEqual(again.replaced, true, 'same name replaces in place')

        const list = await ctx.call('database::listSavedQueries', { db: ctx.driver })
        const entry = list.queries.find((q: any) => q.name === 'catalog_roundtrip')
        expect(entry !== undefined, 'the saved query is listed')
        expectEqual(entry.sql, 'SELECT 2', 'the listed sql is the replacement')

        const del = await ctx.call('database::deleteSavedQuery', { db: ctx.driver, id: entry.id })
        expectEqual(del.deleted, true, 'delete reports the removal')
        const after = await ctx.call('database::listSavedQueries', { db: ctx.driver })
        expectEqual(after.queries.length, 0, 'the deleted query is gone')
      } finally {
        mock.unregister()
      }
    },
  },
  {
    name: 'catalog round-trips a table view through the state worker',
    applies: ['sqlite_db'],
    async run(ctx) {
      const mock = registerMockState(ctx.iii)
      try {
        await mockStateReady(ctx)
        const other = `${TABLE}_unset`
        const saved = await ctx.call('database::saveTableView', {
          db: ctx.driver,
          table: TABLE,
          widths: { id: 120 },
          hidden: ['qty'],
          order: ['qty', 'id'],
        })
        expectEqual(saved.saved, true, 'save reports success')

        const view = await ctx.call('database::getTableView', { db: ctx.driver, table: TABLE })
        expectEqual(view.widths.id, 120, 'column width round-trips')
        expectEqual(view.hidden, ['qty'], 'hidden columns round-trip')
        expectEqual(view.order, ['qty', 'id'], 'column order round-trips')

        const unset = await ctx.call('database::getTableView', { db: ctx.driver, table: other })
        expectEqual(unset.hidden, [], 'an unsaved table has no hidden columns')
        expectEqual(unset.order, [], 'an unsaved table has no custom order')
      } finally {
        mock.unregister()
      }
    },
  },
  {
    name: 'catalog tests a connection without touching a handle',
    applies: ['sqlite_db'],
    async run({ call }) {
      const ok = await call('database::testConnection', { url: 'sqlite::memory:' })
      expectEqual(ok.ok, true, 'an in-memory sqlite url probes ok')
      expectEqual(ok.driver, 'sqlite', 'driver inferred from the url')
      expect(typeof ok.server_version === 'string' && ok.server_version.length > 0, 'version reported')
      expect(typeof ok.latency_ms === 'number', 'latency reported')

      // `ok: false` is a normal response for a URL that parses but cannot
      // work — not a handler error.
      const bad = await call('database::testConnection', { url: 'nonsense://x' })
      expectEqual(bad.ok, false, 'an unknown scheme answers ok:false')
      expectEqual(bad.driver, 'unknown', 'and names the driver unknown')
    },
  },
  {
    // The enforcement half: registering a new database::* function without a
    // case above (and an entry in DATABASE_FUNCTIONS) fails here, so the
    // surface can never grow silently past the suite again.
    name: 'catalog guard keeps every registered database function covered',
    applies: ['sqlite_db'],
    async run({ iii }) {
      const listed = await iii.trigger<Record<string, never>, { functions?: Array<{ function_id?: string }> }>({
        function_id: 'engine::functions::list',
        payload: {},
      })
      const ids = (listed.functions ?? [])
        .map((f) => f.function_id)
        .filter((id): id is string => typeof id === 'string' && id.startsWith('database::'))
        // The config-reload handler every worker registers; not a capability.
        .filter((id) => !id.endsWith('::on-config-change'))
        .sort()
      expectEqual(
        ids,
        [...DATABASE_FUNCTIONS].sort(),
        'database::* registry matches the covered list — add a case above when the surface changes',
      )
    },
  },
]

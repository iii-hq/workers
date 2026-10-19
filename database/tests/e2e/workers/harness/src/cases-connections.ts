/**
 * The connection editor and app surface added with structured connection
 * entries: max_rows, raw browse filters, distinctValues, insertRow,
 * formatSql, getConnection / saveConnection / deleteConnection, the
 * structured testConnection and the @database mention provider. Minimal real
 * calls asserting the declared envelopes, like the rest of the catalog suite.
 */

import type { TestCase } from './cases.ts'
import { expect, expectEqual } from './cases.ts'

const TABLE = 'e2e_conn_rows'

export const CONNECTION_CASES: TestCase[] = [
  {
    name: 'query caps rows with max_rows and reports truncated and elapsed_ms',
    async run({ driver, dialect, call }) {
      await call('database::execute', { db: driver, sql: `DROP TABLE IF EXISTS ${TABLE}` })
      await call('database::execute', {
        db: driver,
        sql: `CREATE TABLE ${TABLE} (id ${dialect.idColumnDDL()}, label VARCHAR(50), qty INT)`,
      })
      await call('database::execute', {
        db: driver,
        sql: `INSERT INTO ${TABLE} (label, qty) VALUES ('a', 1), ('b', 2), ('b', 3), ('c', 4)`,
      })
      const cut = await call('database::query', { db: driver, sql: `SELECT * FROM ${TABLE}`, max_rows: 2 })
      expectEqual(cut.rows.length, 2, 'max_rows caps the rows')
      expectEqual(cut.truncated, true, 'truncated says more existed')
      expect(typeof cut.elapsed_ms === 'number', 'elapsed_ms is a number')
      const all = await call('database::query', { db: driver, sql: `SELECT * FROM ${TABLE}`, max_rows: 10 })
      expectEqual(all.truncated, false, 'not truncated under the cap')
      const exec = await call('database::execute', { db: driver, sql: `UPDATE ${TABLE} SET qty = qty WHERE id < 0` })
      expect(typeof exec.elapsed_ms === 'number', 'execute reports elapsed_ms')
    },
  },
  {
    name: 'browseTable takes a raw filter and refuses one that could write',
    async run({ driver, call, expectError }) {
      const page = await call('database::browseTable', {
        db: driver,
        table: TABLE,
        filters: [{ raw: "qty >= 2 AND label = 'b'" }],
      })
      expectEqual(page.rows.length, 2, 'raw filter applied')
      expectEqual(page.total, 2, 'total honours the raw filter')
      expect(typeof page.elapsed_ms === 'number', 'browse reports elapsed_ms')
      // The shape W-ui sends: an ignored column/op next to raw.
      const same = await call('database::browseTable', {
        db: driver,
        table: TABLE,
        filters: [{ column: '', op: 'equals', raw: 'qty = 4' }],
      })
      expectEqual(same.rows.length, 1, 'column/op ignored with raw')
      await expectError(
        () =>
          call('database::browseTable', {
            db: driver,
            table: TABLE,
            filters: [{ raw: `1=1; DELETE FROM ${TABLE}` }],
          }),
        'INVALID_PARAM',
      )
      const left = await call('database::query', { db: driver, sql: `SELECT COUNT(*) AS c FROM ${TABLE}` })
      expectEqual(String(left.rows[0].c), '4', 'nothing was deleted')
    },
  },
  {
    name: 'distinctValues lists a column by frequency',
    async run({ driver, call }) {
      const r = await call('database::distinctValues', { db: driver, table: TABLE, column: 'label' })
      expectEqual(r.values[0].value, 'b', 'most frequent first')
      expectEqual(Number(r.values[0].count), 2, 'with its count')
      expectEqual(r.truncated, false, 'not truncated')
      const narrowed = await call('database::distinctValues', { db: driver, table: TABLE, column: 'label', search: 'C' })
      expectEqual(narrowed.values.length, 1, 'search narrows case-insensitively')
    },
  },
  {
    name: 'insertRow casts values and returns the inserted row',
    async run({ driver, call }) {
      const r = await call('database::insertRow', { db: driver, table: TABLE, values: { label: 'new', qty: '7' } })
      expectEqual(r.affected_rows, 1, 'one row inserted')
      expect(r.row !== null && r.row !== undefined, `the row comes back: ${JSON.stringify(r)}`)
      expectEqual(r.row.label, 'new', 'label stored')
      expectEqual(Number(r.row.qty), 7, 'qty cast from a string')
      await call('database::execute', { db: driver, sql: `DROP TABLE ${TABLE}` })
    },
  },
  {
    name: 'formatSql pretty-prints for the connection dialect and keeps identifiers',
    async run({ driver, call }) {
      const r = await call('database::formatSql', { db: driver, sql: 'select status, n from t where n > 1 -- why' })
      expect(r.sql.includes('SELECT') && r.sql.includes('WHERE'), `keywords uppercased: ${r.sql}`)
      expect(r.sql.includes('status'), `a non-reserved keyword column keeps its case: ${r.sql}`)
      expect(r.sql.includes('-- why'), 'comment kept')
    },
  },
  {
    name: 'listDatabases reports config_id and the structured fields',
    applies: ['sqlite_db'],
    async run({ call }) {
      const listed = await call('database::listDatabases', {})
      expect(typeof listed.config_id === 'string' && listed.config_id.length > 0, 'config_id present')
      for (const d of listed.databases) {
        expect(typeof d.uses_secrets === 'boolean', `${d.name}: uses_secrets`)
        expect(typeof d.tls.client_cert_present === 'boolean', `${d.name}: client_cert_present`)
      }
    },
  },
  {
    name: 'connections are saved, read redacted, renamed and deleted',
    applies: ['sqlite_db'],
    async run({ call, expectError }) {
      const entry = {
        driver: 'sqlite',
        database: './data/e2e_saved.db',
        display: { icon: 'database', color: '#22C55E' },
        folder: '/tmp/e2e-queries',
      }
      const saved = await call('database::saveConnection', { name: 'e2e_saved', connection: entry })
      expectEqual(saved.saved, true, 'saved')
      expect(saved.error === undefined, `no build error: ${JSON.stringify(saved)}`)
      await expectError(() => call('database::saveConnection', { name: 'e2e_saved', connection: entry }), 'CONFIG_ERROR')
      const listed = await call('database::listDatabases', {})
      const row = listed.databases.find((d: any) => d.name === 'e2e_saved')
      expect(row !== undefined, 'the saved connection is live')
      expectEqual(row.folder, '/tmp/e2e-queries', 'folder rides along')
      expectEqual(row.display.color, '#22C55E', 'display rides along')
      const q = await call('database::query', { db: 'e2e_saved', sql: 'SELECT 1 AS one' })
      expectEqual(q.rows[0].one, 1, 'the saved connection answers')

      // A literal password is shown redacted and survives an edit.
      const got = await call('database::getConnection', { name: 'pg_db' }).catch(() => null)
      if (got) {
        expect(!JSON.stringify(got).includes(':iii@'), `no literal password: ${JSON.stringify(got)}`)
      }

      const renamed = await call('database::saveConnection', {
        name: 'e2e_renamed',
        previous_name: 'e2e_saved',
        connection: entry,
      })
      expectEqual(renamed.name, 'e2e_renamed', 'renamed')
      await expectError(() => call('database::getConnection', { name: 'e2e_saved' }), 'UNKNOWN_DB')
      const read = await call('database::getConnection', { name: 'e2e_renamed' })
      expectEqual(read.connection.driver, 'sqlite', 'stored entry read back')
      const deleted = await call('database::deleteConnection', { name: 'e2e_renamed' })
      expectEqual(deleted.deleted, true, 'deleted')
      const again = await call('database::deleteConnection', { name: 'e2e_renamed' })
      expectEqual(again.deleted, false, 'a second delete finds nothing')
      await expectError(
        () => call('database::saveConnection', { name: 'bad name!', connection: entry }),
        'CONFIG_ERROR',
      )
    },
  },
  {
    name: 'testConnection probes a structured entry and reports the failing stage',
    applies: ['sqlite_db'],
    async run({ call }) {
      const ok = await call('database::testConnection', { connection: { driver: 'sqlite', database: ':memory:' } })
      expectEqual(ok.ok, true, 'in-memory sqlite answers')
      const bad = await call('database::testConnection', {
        connection: { driver: 'postgres', host: '127.0.0.1', port: 1, user: 'u', password: 'p', tls: { mode: 'disable' } },
        timeout_ms: 3000,
      })
      expectEqual(bad.ok, false, 'a refused port fails')
      expectEqual(bad.stage, 'tcp', 'at the tcp stage')
      const secret = await call('database::testConnection', {
        connection: { driver: 'postgres', host: 'h', user: 'u', password: 'secret://E2E_NOPE' },
        timeout_ms: 3000,
      })
      expectEqual(secret.stage, 'secrets', 'an unresolvable reference fails at the secrets stage')
      expect(String(secret.message).includes('secret://E2E_NOPE'), 'naming the reference')
    },
  },
  {
    name: 'the @database mention provider searches and resolves connections',
    applies: ['sqlite_db'],
    async run({ call }) {
      const found = await call('database::mention::search', { query: 'sqlite' })
      expect(found.items.some((i: any) => i.id === 'sqlite_db'), `search finds sqlite_db: ${JSON.stringify(found)}`)
      const view = await call('database::mention::get', { id: 'sqlite_db' })
      expect(String(view.summary).includes('db: "sqlite_db"'), `summary names the db: ${view.summary}`)
      const missing = await call('database::mention::get', { id: 'nope' })
      expectEqual(missing, null, 'an unknown connection resolves to null')
    },
  },
]

import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import {
  browseTable,
  dbEndpoint,
  distinctValues,
  formatSql,
  insertRow,
  isComplete,
  isRawFilter,
  listDbs,
  rawFilter,
  testConnection,
  testConnectionSchema,
  toWireFilter,
} from './rpc'

function fakeHost(respond: (fn: string, payload: Record<string, unknown>) => unknown) {
  const calls: Array<{ fn: string; payload: Record<string, unknown> }> = []
  const host = {
    iii: {
      trigger: async (fn: string, payload: Record<string, unknown>) => {
        calls.push({ fn, payload })
        return respond(fn, payload)
      },
    },
  } as unknown as Host
  return { host, calls }
}

describe('raw filter chips', () => {
  it('is a draft until it holds an expression', () => {
    expect(isRawFilter(rawFilter())).toBe(true)
    expect(isRawFilter({ column: 'a', op: 'equals', value: 1 })).toBe(false)
    expect(isComplete(rawFilter())).toBe(false)
    expect(isComplete(rawFilter('   '))).toBe(false)
    expect(isComplete(rawFilter("status = 'open'"))).toBe(true)
  })

  it('serialises a raw chip as its expression and a column chip without `raw`', () => {
    expect(toWireFilter(rawFilter("  status = 'open' "))).toEqual({ column: '', op: 'equals', raw: "status = 'open'" })
    expect(toWireFilter({ ...rawFilter('a > 1'), disabled: true })).toEqual({
      column: '',
      op: 'equals',
      raw: 'a > 1',
      disabled: true,
    })
    expect(toWireFilter({ column: 'a', op: 'gt', value: 1, raw: undefined })).toEqual({ column: 'a', op: 'gt', value: 1 })
  })

  it('sends complete chips only, raw ones in wire form', async () => {
    const { host, calls } = fakeHost(() => ({ rows: [], columns: [], page: 0, page_size: 50, has_more: false }))
    await browseTable(host, 'pg', 'orders', 'public', {
      filters: [
        rawFilter('total > 100'),
        rawFilter(''),
        { column: 'status', op: 'equals', value: 'open' },
        { column: 'status', op: 'equals' },
      ],
    })
    expect(calls[0].payload.filters).toEqual([
      { column: '', op: 'equals', raw: 'total > 100' },
      { column: 'status', op: 'equals', value: 'open' },
    ])
  })
})

describe('listDatabases', () => {
  it('reads the extended entry: display, endpoint, error', async () => {
    const { host } = fakeHost(() => ({
      databases: [
        {
          name: 'shop',
          driver: 'postgres',
          url: 'postgres://***@db:5432/shop',
          host: 'db',
          port: 5432,
          user: 'app',
          database: 'shop',
          display: { icon: 'server', color: '#16A34A' },
          folder: '/srv/q',
          ssh: { host: 'bastion', port: 22, user: 'ubuntu' },
          tls: { mode: 'require', ca_cert_present: false, client_cert_present: false, trust_native: true },
          uses_secrets: true,
          pool: { max: 10 },
          error: 'secret://DATABASE_SHOP_PASSWORD could not be resolved',
        },
        { name: 'bad-colour', driver: 'sqlite', database: './a.db', display: { color: 'green' } },
      ],
      count: 2,
    }))
    const [shop, sqlite] = await listDbs(host)
    expect(shop).toMatchObject({
      name: 'shop',
      driver: 'postgres',
      icon: 'server',
      color: '#16A34A',
      folder: '/srv/q',
      ssh: { host: 'bastion', port: 22, user: 'ubuntu' },
      usesSecrets: true,
      poolMax: 10,
      error: 'secret://DATABASE_SHOP_PASSWORD could not be resolved',
    })
    expect(dbEndpoint(shop)).toBe('db:5432/shop')
    expect(sqlite.color).toBeUndefined()
    expect(dbEndpoint(sqlite)).toBe('./a.db')
  })

  it('still reads an older worker’s minimal entry', async () => {
    const { host } = fakeHost(() => ({
      databases: [{ name: 'primary', driver: 'sqlite', url: 'sqlite:./data/iii.db', pool: { max: 4 } }],
      count: 1,
    }))
    const [primary] = await listDbs(host)
    expect(primary).toMatchObject({ name: 'primary', driver: 'sqlite', url: 'sqlite:./data/iii.db', poolMax: 4 })
    expect(primary.error).toBeUndefined()
    expect(dbEndpoint(primary)).toBe('sqlite:./data/iii.db')
  })
})

describe('new functions', () => {
  it('asks for distinct values with the schema split out', async () => {
    const { host, calls } = fakeHost(() => ({ values: [{ value: 'open', count: 3 }], truncated: false }))
    const res = await distinctValues(host, 'pg', 'orders', 'public', 'status', { search: 'op', limit: 12 })
    expect(calls[0]).toEqual({
      fn: 'database::distinctValues',
      payload: { db: 'pg', table: 'orders', schema: 'public', column: 'status', search: 'op', limit: 12 },
    })
    expect(res.values).toEqual([{ value: 'open', count: 3 }])
  })

  it('inserts a row and reads back what was stored', async () => {
    const { host, calls } = fakeHost(() => ({ row: { id: 9, email: 'a@x' }, affected_rows: 1, elapsed_ms: 2 }))
    const res = await insertRow(host, 'sq', 'users', null, { email: 'a@x' })
    expect(calls[0]).toEqual({ fn: 'database::insertRow', payload: { db: 'sq', table: 'users', values: { email: 'a@x' } } })
    expect(res.row).toEqual({ id: 9, email: 'a@x' })
  })

  it('formats through the worker', async () => {
    const { host, calls } = fakeHost(() => ({ sql: 'SELECT\n  1' }))
    expect(await formatSql(host, 'pg', 'select 1')).toBe('SELECT\n  1')
    expect(calls[0].payload).toEqual({ db: 'pg', sql: 'select 1' })
  })

  it('parses a probe result, including where it stopped and the host key', async () => {
    const { host } = fakeHost(() => ({
      ok: false,
      driver: 'postgres',
      latency_ms: 31,
      message: 'host key mismatch',
      stage: 'ssh',
      ssh_host_key: { algorithm: 'ssh-ed25519', fingerprint: 'SHA256:abc' },
    }))
    const res = await testConnection(host, { connection: { driver: 'postgres' } })
    expect(res.stage).toBe('ssh')
    expect(res.ssh_host_key).toEqual({ algorithm: 'ssh-ed25519', fingerprint: 'SHA256:abc' })
    // An older worker's answer still parses.
    expect(testConnectionSchema.safeParse({ ok: true, driver: 'sqlite', latency_ms: 1 }).success).toBe(true)
    expect(testConnectionSchema.safeParse({ driver: 'sqlite' }).success).toBe(false)
  })
})

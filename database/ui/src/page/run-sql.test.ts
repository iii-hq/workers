import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import { commonSchema, ddlInfo, resultSummary, runAdhocSql } from './db-data'

/** Host stub that records the one call the dispatcher makes. */
function fakeHost(respond: (fn: string) => unknown) {
  const calls: Array<{ fn: string; payload: Record<string, unknown> }> = []
  const host = {
    iii: {
      trigger: async (fn: string, payload: Record<string, unknown>) => {
        calls.push({ fn, payload })
        return respond(fn)
      },
    },
  } as unknown as Host
  return { host, calls }
}

const EMPTY_QUERY = { rows: [], row_count: 0, columns: [] }

describe('runAdhocSql', () => {
  it('routes reads through database::query', async () => {
    const { host, calls } = fakeHost(() => EMPTY_QUERY)
    const out = await runAdhocSql(host, 'mysql', 'SELECT * FROM shipments')
    expect(calls.map((c) => c.fn)).toEqual(['database::query'])
    expect(out.write).toBeUndefined()
  })

  it('routes writes through database::execute and strips the trailing semicolon', async () => {
    const { host, calls } = fakeHost(() => ({
      affected_rows: 1,
      last_insert_id: null,
      returned_rows: [],
    }))
    // The exact statement from the screenshot that used to be rejected.
    const out = await runAdhocSql(host, 'mysql', 'drop table completion_log;')
    expect(calls.map((c) => c.fn)).toEqual(['database::execute'])
    expect(calls[0].payload.sql).toBe('drop table completion_log')
    expect(out.write).toEqual({ affectedRows: 1, lastInsertId: null, echo: 'dropped table completion_log' })
    expect(out.result.rows).toEqual([])
  })

  it('caps reads at the row limit and reports the server time and the cut', async () => {
    const { host, calls } = fakeHost(() => ({ ...EMPTY_QUERY, row_count: 100, truncated: true, elapsed_ms: 7 }))
    const out = await runAdhocSql(host, 'pg', 'select * from orders', { maxRows: 100 })
    expect(calls[0].payload).toEqual({ db: 'pg', sql: 'select * from orders', max_rows: 100 })
    expect(out.serverMs).toBe(7)
    expect(out.truncatedAt).toBe(100)
  })

  it('sends no cap without a limit, and never caps a write', async () => {
    const { host, calls } = fakeHost((fn) =>
      fn === 'database::query' ? EMPTY_QUERY : { affected_rows: 2, last_insert_id: null, elapsed_ms: 3 },
    )
    await runAdhocSql(host, 'pg', 'select 1', { maxRows: null })
    const write = await runAdhocSql(host, 'pg', 'update t set a = 1', { maxRows: 100 })
    expect(calls[0].payload).toEqual({ db: 'pg', sql: 'select 1' })
    expect(calls[1].payload).toEqual({ db: 'pg', sql: 'update t set a = 1' })
    expect(write.serverMs).toBe(3)
    expect(write.truncatedAt).toBeUndefined()
  })

  it('turns RETURNING rows into a drawable grid', async () => {
    const { host } = fakeHost(() => ({
      affected_rows: 1,
      last_insert_id: '7',
      returned_rows: [{ id: 7, email: 'a@x' }],
    }))
    const out = await runAdhocSql(host, 'pg', "INSERT INTO users (email) VALUES ('a@x') RETURNING id, email")
    expect(out.write).toEqual({ affectedRows: 1, lastInsertId: '7', echo: null })
    expect(out.result.rows).toEqual([{ id: 7, email: 'a@x' }])
    expect(out.result.columns.map((c) => c.name)).toEqual(['id', 'email'])
  })
})

describe('ddlInfo', () => {
  it('names the schema change in both tenses', () => {
    expect(ddlInfo('drop table completion_log')).toEqual({
      present: 'drops table completion_log',
      past: 'dropped table completion_log',
    })
    expect(ddlInfo('DROP TABLE IF EXISTS `completion_log`')).toEqual({
      present: 'drops table completion_log',
      past: 'dropped table completion_log',
    })
    expect(ddlInfo('truncate shipments')).toEqual({
      present: 'truncates shipments',
      past: 'truncated shipments',
    })
    expect(ddlInfo('alter table orders add column note text')).toEqual({
      present: 'alters table orders',
      past: 'altered table orders',
    })
  })

  it('leaves row writes and reads alone', () => {
    expect(ddlInfo('update t set a = 1')).toBeNull()
    expect(ddlInfo('insert into t values (1)')).toBeNull()
    expect(ddlInfo("select * from t where note like '%drop table%'")).toBeNull()
  })
})

describe('commonSchema', () => {
  it('finds the one schema qualifying every table', () => {
    expect(commonSchema(['public.a', 'public.b', 'public.c'])).toBe('public')
  })

  it('returns nothing when schemas are mixed or absent', () => {
    expect(commonSchema(['public.a', 'audit.b'])).toBe('')
    expect(commonSchema(['public.a', 'plain'])).toBe('')
    expect(commonSchema(['shipments', 'ledger'])).toBe('')
    expect(commonSchema([])).toBe('')
  })
})

describe('resultSummary', () => {
  const read = (n: number, extra: Record<string, unknown> = {}) => ({
    result: { rows: [], row_count: n, columns: [] },
    durationMs: 40,
    ...extra,
  })

  it('prefers the worker’s own timing and says when the limit cut the result', () => {
    expect(resultSummary(read(1))).toBe('1 row in 40 ms')
    expect(resultSummary(read(12, { serverMs: 4 }))).toBe('12 rows in 4 ms')
    expect(resultSummary(read(100, { serverMs: 4, truncatedAt: 100 }))).toBe('100 rows in 4 ms · truncated at 100')
  })

  it('describes writes by what they did', () => {
    expect(
      resultSummary({
        ...read(0, { serverMs: 2 }),
        write: { affectedRows: 3, lastInsertId: null, echo: null },
      }),
    ).toBe('3 affected in 2 ms')
    expect(
      resultSummary({ ...read(0), write: { affectedRows: 0, lastInsertId: null, echo: 'dropped table x' } }),
    ).toBe('dropped table x in 40 ms')
  })
})

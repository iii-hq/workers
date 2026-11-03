import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import { commonSchema, ddlInfo, describeWrite, resultSummary, runAdhocSql, writeHeadline } from './db-data'

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
    expect(out.write).toEqual({
      affectedRows: 1,
      lastInsertId: null,
      echo: 'dropped table completion_log',
      info: { kind: 'drop', object: 'table', target: 'completion_log', returning: false },
    })
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
    expect(out.write).toEqual({
      affectedRows: 1,
      lastInsertId: '7',
      echo: null,
      info: { kind: 'insert', object: null, target: 'users', returning: true },
    })
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
    ).toBe('3 rows affected in 2 ms')
    expect(resultSummary({ ...read(0), write: { affectedRows: 0, lastInsertId: null, echo: 'dropped table x' } })).toBe(
      'Dropped table x in 40 ms',
    )
    expect(
      resultSummary({
        ...read(0, { serverMs: 3 }),
        write: { affectedRows: 1, lastInsertId: '7', echo: null, info: describeWrite("insert into todos (title) values ('a')") },
      }),
    ).toBe('Inserted 1 row into todos in 3 ms')
  })
})

describe('describeWrite', () => {
  it('reads the verb and the table off row writes', () => {
    expect(describeWrite("INSERT INTO public.orders (id) VALUES (1)")).toEqual({
      kind: 'insert',
      object: null,
      target: 'public.orders',
      returning: false,
    })
    expect(describeWrite('insert or replace into "todos" values (1)')?.target).toBe('todos')
    expect(describeWrite('update `todos` set done = 1 where id = 2')).toMatchObject({ kind: 'update', target: 'todos' })
    expect(describeWrite('delete from todos where done = 1 returning *')).toMatchObject({
      kind: 'delete',
      target: 'todos',
      returning: true,
    })
    expect(describeWrite('with old as (select 1) delete from logs')).toMatchObject({ kind: 'delete', target: 'logs' })
    expect(describeWrite('-- tidy up\nupdate t set a = 1')).toMatchObject({ kind: 'update', target: 't' })
  })

  it('reads schema statements, naming the object', () => {
    expect(describeWrite('create table if not exists orders (id int)')).toMatchObject({
      kind: 'create',
      object: 'table',
      target: 'orders',
    })
    expect(describeWrite('CREATE UNIQUE INDEX idx_orders_id ON orders (id)')).toMatchObject({
      kind: 'create',
      object: 'index',
      target: 'idx_orders_id',
    })
    expect(describeWrite('drop table completion_log')).toMatchObject({ kind: 'drop', object: 'table', target: 'completion_log' })
    expect(describeWrite('truncate shipments')).toMatchObject({ kind: 'truncate', object: 'table', target: 'shipments' })
  })

  it('stays quiet about shapes it does not know', () => {
    expect(describeWrite('vacuum')).toBeNull()
    expect(describeWrite('select 1')).toBeNull()
  })
})

describe('writeHeadline', () => {
  const w = (sql: string, affectedRows: number, echo: string | null = null) => ({
    affectedRows,
    lastInsertId: null,
    echo,
    info: describeWrite(sql),
  })

  it('says what happened, in the past tense, with the count', () => {
    expect(writeHeadline(w('insert into todos values (1), (2)', 2))).toBe('Inserted 2 rows into todos')
    expect(writeHeadline(w('update todos set done = 1', 0))).toBe('Updated 0 rows in todos')
    expect(writeHeadline(w('delete from todos where id = 1', 1))).toBe('Deleted 1 row from todos')
    expect(writeHeadline(w('create view open_todos as select 1', 0))).toBe('Created view open_todos')
    expect(writeHeadline(w('drop table x', 0, 'dropped table x'))).toBe('Dropped table x')
    expect(writeHeadline(w('vacuum', 0))).toBe('0 rows affected')
  })
})

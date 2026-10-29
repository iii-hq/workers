import { describe, expect, it } from 'vitest'
import {
  buildInsertValues,
  fieldValue,
  fkTable,
  hasServerDefault,
  type InsertColumn,
  initialField,
  inputKindFor,
  modesFor,
} from './new-row'

const col = (over: Partial<InsertColumn> & { name: string; type: string }): InsertColumn => ({
  nullable: false,
  pk: false,
  defaultValue: null,
  ...over,
})

describe('inputKindFor', () => {
  it('maps each driver’s type names to a control', () => {
    const cases: Array<[string, string]> = [
      ['integer', 'integer'],
      ['bigint', 'integer'],
      ['int(11)', 'integer'],
      ['serial', 'integer'],
      ['smallint unsigned', 'integer'],
      ['year', 'integer'],
      ['numeric(10,2)', 'decimal'],
      ['decimal', 'decimal'],
      ['money', 'decimal'],
      ['double precision', 'number'],
      ['real', 'number'],
      ['float', 'number'],
      ['boolean', 'bool'],
      ['tinyint(1)', 'integer'],
      ['date', 'date'],
      ['timestamp with time zone', 'datetime'],
      ['datetime', 'datetime'],
      ['time without time zone', 'time'],
      ['jsonb', 'json'],
      ['json', 'json'],
      ['bytea', 'binary'],
      ['blob', 'binary'],
      ['text', 'longtext'],
      ['varchar(255)', 'text'],
      ['character varying', 'text'],
      ['uuid', 'text'],
      ["enum('a','b')", 'text'],
      ['interval', 'text'],
      ['point', 'text'],
      ['', 'text'],
    ]
    for (const [type, kind] of cases) expect(inputKindFor(type), type).toBe(kind)
  })
})

describe('defaults and modes', () => {
  it('leaves generated and defaulted columns to the database', () => {
    expect(hasServerDefault(col({ name: 'id', type: 'integer', pk: true }))).toBe(true)
    expect(hasServerDefault(col({ name: 'id', type: 'uuid', pk: true }))).toBe(false)
    expect(hasServerDefault(col({ name: 'id', type: 'bigserial' }))).toBe(true)
    expect(hasServerDefault(col({ name: 'at', type: 'timestamp', defaultValue: 'now()' }))).toBe(true)
    expect(hasServerDefault(col({ name: 'note', type: 'text', defaultValue: '' }))).toBe(false)
  })

  it('starts required columns on a value and the rest on Default', () => {
    expect(initialField(col({ name: 'email', type: 'text' }))).toEqual({ mode: 'value', text: '' })
    expect(initialField(col({ name: 'note', type: 'text', nullable: true }))).toEqual({ mode: 'default', text: '' })
    expect(initialField(col({ name: 'id', type: 'integer', pk: true }))).toEqual({ mode: 'default', text: '' })
  })

  it('offers NULL only for nullable columns', () => {
    expect(modesFor(col({ name: 'email', type: 'text' }))).toEqual(['value'])
    expect(modesFor(col({ name: 'note', type: 'text', nullable: true }))).toEqual(['default', 'value', 'null'])
    expect(modesFor(col({ name: 'status', type: 'text', defaultValue: "'open'" }))).toEqual(['default', 'value'])
  })
})

describe('fieldValue', () => {
  const value = (type: string, text: string, over: Partial<InsertColumn> = {}) =>
    fieldValue(col({ name: 'c', type, ...over }), { mode: 'value', text })

  it('turns typed text into the JSON the worker casts', () => {
    expect(value('integer', ' 42 ')).toEqual({ omit: false, value: 42 })
    expect(value('bigint', '9007199254740993')).toEqual({ omit: false, value: '9007199254740993' })
    expect(value('real', '1.5e3')).toEqual({ omit: false, value: 1500 })
    expect(value('numeric(10,2)', '0.10')).toEqual({ omit: false, value: '0.10' })
    expect(value('boolean', 'false')).toEqual({ omit: false, value: false })
    expect(value('jsonb', '{"a": [1, 2]}')).toEqual({ omit: false, value: { a: [1, 2] } })
    expect(value('timestamp', '2026-10-18T14:30')).toEqual({ omit: false, value: '2026-10-18 14:30' })
    expect(value('date', '2026-10-18')).toEqual({ omit: false, value: '2026-10-18' })
    expect(value('varchar', '  padded ')).toEqual({ omit: false, value: '  padded ' })
    expect(value('text', '')).toEqual({ omit: false, value: '' })
  })

  it('catches text that cannot be the column’s type', () => {
    expect(value('integer', '4.2')).toEqual({ error: 'not a whole number' })
    expect(value('real', 'abc')).toEqual({ error: 'not a number' })
    expect(value('numeric', '1,5')).toEqual({ error: 'not a number' })
    expect(value('boolean', 'yes')).toEqual({ error: 'true or false' })
    expect(value('json', '{oops')).toEqual({ error: 'not valid JSON' })
    expect(value('integer', '')).toEqual({ error: 'enter a value' })
  })

  it('omits Default and sends NULL only where allowed', () => {
    expect(fieldValue(col({ name: 'c', type: 'text' }), { mode: 'default', text: 'x' })).toEqual({ omit: true })
    expect(fieldValue(col({ name: 'c', type: 'text', nullable: true }), { mode: 'null', text: '' })).toEqual({
      omit: false,
      value: null,
    })
    expect(fieldValue(col({ name: 'c', type: 'text' }), { mode: 'null', text: '' })).toEqual({ error: 'not nullable' })
  })
})

describe('buildInsertValues', () => {
  const columns = [
    col({ name: 'id', type: 'integer', pk: true }),
    col({ name: 'email', type: 'varchar(255)' }),
    col({ name: 'customer_id', type: 'integer', fk: { schema: 'public', table: 'customers', column: 'id' } }),
    col({ name: 'note', type: 'text', nullable: true }),
    col({ name: 'total', type: 'numeric(10,2)', defaultValue: '0' }),
  ]

  it('sends only what the user set, leaving defaults out', () => {
    expect(
      buildInsertValues(columns, {
        email: { mode: 'value', text: 'a@x' },
        customer_id: { mode: 'value', text: '7' },
        note: { mode: 'null', text: '' },
      }),
    ).toEqual({ values: { email: 'a@x', customer_id: 7, note: null }, errors: {} })
  })

  it('reports every field that cannot be sent', () => {
    expect(buildInsertValues(columns, { customer_id: { mode: 'value', text: 'seven' } })).toEqual({
      values: { email: '' },
      errors: { customer_id: 'not a whole number' },
    })
  })

  it('names the referenced table the way the page does', () => {
    expect(fkTable({ schema: 'public', table: 'customers', column: 'id' })).toBe('public.customers')
    expect(fkTable({ schema: null, table: 'customers', column: 'id' })).toBe('customers')
  })
})

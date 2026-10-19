/**
 * The New row form's model: which control a column gets, what it starts on,
 * and how a typed field becomes the JSON `database::insertRow` takes.
 *
 * The worker casts every value to its column's type, so the page sends plain
 * JSON and never builds SQL. What the page decides is only what the user
 * meant: "leave it to the default", "NULL", or this value — and whether the
 * text typed can be that value at all, so a typo is caught by the field
 * rather than by a failed insert.
 */

import type { ForeignKeyRef } from '../lib/rpc'
import { typeCategory } from '../lib/rpc'

export type InputKind = 'integer' | 'number' | 'decimal' | 'bool' | 'date' | 'datetime' | 'time' | 'json' | 'text' | 'longtext' | 'binary'

export interface InsertColumn {
  name: string
  type: string
  nullable: boolean
  pk: boolean
  /** The column's default as the catalog spells it (`nextval(…)`, `now()`, `0`). */
  defaultValue?: string | null
  fk?: ForeignKeyRef
}

/** The control for a column, by its declared type. */
export function inputKindFor(type: string): InputKind {
  const t = type.toLowerCase()
  // `interval` contains "int" and `point` contains it too: neither is a number.
  if (/interval|point/.test(t)) return 'text'
  const category = typeCategory(type)
  switch (category) {
    case 'bool':
      return 'bool'
    case 'numeric':
      if (/decimal|numeric|money/.test(t)) return 'decimal'
      if (/int|serial/.test(t)) return 'integer'
      return 'number'
    case 'date':
      if (/timestamp|datetime/.test(t)) return 'datetime'
      if (/^year/.test(t)) return 'integer'
      if (/^date\b/.test(t) && !/time/.test(t)) return 'date'
      if (/time/.test(t)) return 'time'
      return 'date'
    case 'json':
      return 'json'
    case 'binary':
      return 'binary'
    case 'text':
      return /^(?:text|mediumtext|longtext|clob)\b/.test(t) ? 'longtext' : 'text'
    default:
      return 'text'
  }
}

/** The database fills it in: a declared default, or an auto-generated key. */
export function hasServerDefault(col: InsertColumn): boolean {
  if (col.defaultValue != null && col.defaultValue.trim() !== '') return true
  const t = col.type.toLowerCase()
  if (/serial|identity|auto_increment/.test(t)) return true
  // An integer primary key is generated on all three drivers in practice
  // (SQLite's rowid alias, MySQL AUTO_INCREMENT, Postgres identity) — and
  // when it is not, the insert says so and the field takes a value.
  return col.pk && /^(?:integer|int|bigint|smallint|int4|int8|int2)\b/.test(t)
}

export type FieldMode = 'default' | 'null' | 'value'

export interface FieldState {
  mode: FieldMode
  text: string
}

/** Where a field starts: left to the database when it can fill it in. */
export function initialField(col: InsertColumn): FieldState {
  if (hasServerDefault(col) || col.nullable) return { mode: 'default', text: '' }
  return { mode: 'value', text: '' }
}

/** The modes a column offers: NULL only when it is nullable. */
export function modesFor(col: InsertColumn): FieldMode[] {
  const modes: FieldMode[] = ['value']
  if (hasServerDefault(col) || col.nullable) modes.unshift('default')
  if (col.nullable) modes.push('null')
  return modes
}

export type FieldValue = { omit: true } | { omit: false; value: unknown } | { error: string }

const INTEGER_RE = /^[-+]?\d+$/
const NUMBER_RE = /^[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?$/

/** One field as JSON for the insert, or why its text cannot be that. */
export function fieldValue(col: InsertColumn, field: FieldState): FieldValue {
  if (field.mode === 'default') return { omit: true }
  if (field.mode === 'null') {
    return col.nullable ? { omit: false, value: null } : { error: 'not nullable' }
  }
  const kind = inputKindFor(col.type)
  const text = field.text
  const trimmed = text.trim()
  if (trimmed === '' && kind !== 'text' && kind !== 'longtext') return { error: 'enter a value' }
  switch (kind) {
    case 'integer': {
      if (!INTEGER_RE.test(trimmed)) return { error: 'not a whole number' }
      const n = Number(trimmed)
      // Beyond 2^53 a JS number rounds; the worker casts the text exactly.
      return { omit: false, value: Number.isSafeInteger(n) ? n : trimmed }
    }
    case 'number': {
      if (!NUMBER_RE.test(trimmed)) return { error: 'not a number' }
      return { omit: false, value: Number(trimmed) }
    }
    case 'decimal':
      // Kept as text: a decimal is exact, a float is not.
      return NUMBER_RE.test(trimmed) ? { omit: false, value: trimmed } : { error: 'not a number' }
    case 'bool':
      if (trimmed === 'true') return { omit: false, value: true }
      if (trimmed === 'false') return { omit: false, value: false }
      return { error: 'true or false' }
    case 'json':
      try {
        return { omit: false, value: JSON.parse(trimmed) }
      } catch {
        return { error: 'not valid JSON' }
      }
    case 'datetime':
      // `<input type="datetime-local">` writes `2026-10-18T14:30`; a space
      // is the separator every driver accepts.
      return { omit: false, value: trimmed.replace('T', ' ') }
    case 'date':
    case 'time':
      return { omit: false, value: trimmed }
    default:
      return { omit: false, value: text }
  }
}

/** The insert's `values`, or the fields that cannot be sent as typed. */
export function buildInsertValues(
  columns: readonly InsertColumn[],
  fields: Readonly<Record<string, FieldState>>,
): { values: Record<string, unknown>; errors: Record<string, string> } {
  const values: Record<string, unknown> = {}
  const errors: Record<string, string> = {}
  for (const col of columns) {
    const out = fieldValue(col, fields[col.name] ?? initialField(col))
    if ('error' in out) errors[col.name] = out.error
    else if (!out.omit) values[col.name] = out.value
  }
  return { values, errors }
}

/** The page's identifier for the table a foreign key points at. */
export function fkTable(fk: ForeignKeyRef): string {
  return fk.schema ? `${fk.schema}.${fk.table}` : fk.table
}

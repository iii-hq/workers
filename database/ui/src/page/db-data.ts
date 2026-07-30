/**
 * The page's data layer — now a thin adapter over the worker's own functions.
 *
 * This file used to carry ~520 lines of per-driver catalog SQL: three
 * spellings of a table list, three of a column list, `PRAGMA table_info`,
 * `information_schema` correlated subqueries, `pg_indexes`, `GROUP_CONCAT`
 * over `STATISTICS`. All of it now lives in the worker as
 * `database::listTables` / `describeTable` / `describeSchema` / `browseTable`,
 * where it is written once, tested against all three drivers in CI, and
 * callable by any agent rather than only by this page.
 *
 * What remains is shape translation for the existing components, plus the two
 * client-side SQL affordances (`isReadOnlySql`, `explainPrefix`) that exist to
 * grey out a button before a round trip. The worker enforces both for real;
 * neither is trusted.
 */

import type { Host } from '@iii-dev/console-ui'
import type { DbDriver, ForeignKeyRef, QueryResponse } from '../lib/rpc'
import * as rpc from '../lib/rpc'

export type {
  ColumnMeta,
  DbDriver,
  DbInfo,
  FilterOp,
  FilterSpec,
  ForeignKeyRef,
  QueryResponse,
  SortSpec,
  TypeCategory,
} from '../lib/rpc'
export { isComplete, listDbs, opsFor, PAGE_SIZE, typeCategory } from '../lib/rpc'

export interface TableSort {
  column: string
  dir: 'asc' | 'desc'
}

export interface DbTable {
  /** Display + query identifier; schema-qualified for postgres. */
  name: string
  kind: 'table' | 'view'
}

export interface ColumnInfo {
  name: string
  type: string
  nullable: boolean
  pk: boolean
  /**
   * Structured reference. Prefer this over `fkTarget` — a joined
   * `"table.column"` string cannot express a schema qualifier unambiguously,
   * and breaks outright on a table whose name contains a dot.
   */
  fk?: ForeignKeyRef
  /** Display-only rendering of `fk`. */
  fkTarget?: string
}

export interface IndexInfo {
  name: string
  unique: boolean
  /** Column list, joined for display. */
  detail: string
}

export interface TablePage {
  result: QueryResponse
  page: number
  pageSize: number
  /** A further page exists. Derived from a sentinel row by the worker, so it
      is correct even when the caller skipped the count. */
  hasMore: boolean
  /** Rows matching the same filters, when the caller asked for it. */
  total?: number | null
}

export interface AdhocResult {
  result: QueryResponse
  durationMs: number
}

/** Split a schema-qualified name for the worker, which takes them apart. */
function splitRef(table: string): { schema: string | null; name: string } {
  if (!table.includes('.')) return { schema: null, name: table }
  const [schema, ...rest] = table.split('.')
  return { schema, name: rest.join('.') }
}

/** Rebuild the qualified display name the page uses as a table identifier. */
function qualify(schema: string | null | undefined, name: string): string {
  return schema ? `${schema}.${name}` : name
}

export async function listTables(host: Host, db: string, _driver: DbDriver): Promise<DbTable[]> {
  const tables = await rpc.listTables(host, db)
  return tables.map((t) => ({ name: qualify(t.schema, t.name), kind: t.kind }))
}

/**
 * Browse a table by its page identifier.
 *
 * The identifier is schema-qualified (`public.users` on postgres), and the
 * worker takes schema and name apart. Calling `rpc.browseTable` directly with
 * the qualified string and a null schema looks up a table literally named
 * `public.users` — invisible on sqlite, which has no schemas, and broken on
 * postgres. Every other read in this file splits first; so does this.
 */
export async function browseTableRef(
  host: Host,
  db: string,
  table: string,
  opts: rpc.BrowseOptions,
): Promise<rpc.BrowseResult> {
  const { schema, name } = splitRef(table)
  return rpc.browseTable(host, db, name, schema, opts)
}

export async function tableColumns(host: Host, db: string, _driver: DbDriver, table: string): Promise<ColumnInfo[]> {
  const { schema, name } = splitRef(table)
  const d = await rpc.describeTable(host, db, name, schema)
  return d.columns.map((c) => ({
    name: c.name,
    type: c.type,
    nullable: c.nullable,
    pk: c.primary_key,
    fk: c.foreign_key ?? undefined,
    fkTarget: c.foreign_key
      ? `${qualify(c.foreign_key.schema, c.foreign_key.table)}.${c.foreign_key.column}`
      : undefined,
  }))
}

export async function tableIndexes(host: Host, db: string, _driver: DbDriver, table: string): Promise<IndexInfo[]> {
  const { schema, name } = splitRef(table)
  const d = await rpc.describeTable(host, db, name, schema)
  return d.indexes.map((i) => ({
    name: i.name,
    unique: i.unique,
    detail: i.columns.join(', '),
  }))
}

export async function fetchTablePage(
  host: Host,
  db: string,
  _driver: DbDriver,
  table: string,
  page: number,
  pageSize: number = rpc.PAGE_SIZE,
  sort?: TableSort | null,
  filters?: rpc.FilterSpec[],
): Promise<TablePage> {
  const { schema, name } = splitRef(table)
  const res = await rpc.browseTable(host, db, name, schema, {
    page,
    pageSize,
    sort: sort ? [{ column: sort.column, direction: sort.dir }] : [],
    filters,
  })
  return {
    result: { rows: res.rows, row_count: res.rows.length, columns: res.columns },
    page: res.page,
    pageSize: res.page_size,
    hasMore: res.has_more,
    total: res.total,
  }
}

/**
 * Row count matching the current filters.
 *
 * `browseTable` already returns this alongside the page, so prefer
 * `TablePage.total`. Kept for callers that ask on their own.
 */
export async function countTableRows(
  host: Host,
  db: string,
  _driver: DbDriver,
  table: string,
  filters?: rpc.FilterSpec[],
): Promise<number | null> {
  const { schema, name } = splitRef(table)
  const res = await rpc.browseTable(host, db, name, schema, { pageSize: 1, filters })
  return res.total ?? null
}

/* ---- ad-hoc SQL ---- */

const READ_ONLY_LEAD = /^(select|with|explain|pragma|show|describe|desc|values|table)\b/i
const WRITE_ANYWHERE =
  /\b(insert|update|delete|replace|merge|upsert|drop|create|alter|truncate|grant|revoke|attach|detach|reindex|vacuum)\b/i

/**
 * Best-effort read check, used only to grey out the run button before a round
 * trip. The worker re-checks every statement and is the actual authority —
 * this is a UX affordance, not a security boundary.
 */
export function isReadOnlySql(sql: string): boolean {
  const stripped = sql
    .replace(/--[^\n]*/g, '')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .trim()
  if (!stripped) return false
  if (stripped.replace(/;+\s*$/, '').includes(';')) return false
  if (!READ_ONLY_LEAD.test(stripped)) return false
  if (/^pragma\b/i.test(stripped) && stripped.includes('=')) return false
  return !WRITE_ANYWHERE.test(stripped)
}

export async function runReadOnlySql(host: Host, db: string, sql: string): Promise<AdhocResult> {
  if (!isReadOnlySql(sql)) {
    throw new Error('only read-only statements can be run from this panel')
  }
  const started = performance.now()
  const result = await rpc.runSql(host, db, sql)
  return { result, durationMs: Math.round(performance.now() - started) }
}

/**
 * Kept so the SQL panel can prefix a statement itself. Prefer
 * `database::explain`, which returns a plan tree rather than a grid of text.
 */
export function explainPrefix(driver: DbDriver): string {
  return driver === 'sqlite' ? 'EXPLAIN QUERY PLAN ' : 'EXPLAIN '
}

/* ---- identifier quoting, for display only ---- */

export function quoteIdent(driver: DbDriver, ident: string): string {
  return driver === 'mysql' ? `\`${ident.replace(/`/g, '``')}\`` : `"${ident.replace(/"/g, '""')}"`
}

export function quoteTableRef(driver: DbDriver, table: string): string {
  const { schema, name } = splitRef(table)
  return driver === 'postgres' && schema
    ? `${quoteIdent(driver, schema)}.${quoteIdent(driver, name)}`
    : quoteIdent(driver, name)
}

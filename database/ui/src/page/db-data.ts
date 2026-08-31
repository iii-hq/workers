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
 * client-side SQL affordances (`isReadOnlySql` picks the query vs execute
 * surface for a statement, `explainPrefix`). The worker enforces read-only on
 * `query` for real; nothing here is trusted.
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
  /** Grid to draw. For writes, synthesized from the RETURNING rows. */
  result: QueryResponse
  durationMs: number
  /**
   * Present when the statement ran through `database::execute`. `echo` is the
   * human account of a schema change ("dropped table x") — the row count is
   * the wrong summary for DDL, which MySQL reports as `0 affected`.
   */
  write?: { affectedRows: number; lastInsertId: string | null; echo: string | null }
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
 * Best-effort read check. It routes: reads go to `database::query`, anything
 * else to `database::execute`. Misclassifying a read as a write is harmless —
 * `execute` runs SELECTs too, it just reports them as `0 affected` — and the
 * worker's own read-only enforcement on `query` catches the other direction.
 * A routing hint, not a security boundary.
 */
function stripSqlComments(sql: string): string {
  return sql.replace(/--[^\n]*/g, '').replace(/\/\*[\s\S]*?\*\//g, '')
}

export function isReadOnlySql(sql: string): boolean {
  const stripped = stripSqlComments(sql).trim()
  if (!stripped) return false
  if (stripped.replace(/;+\s*$/, '').includes(';')) return false
  if (!READ_ONLY_LEAD.test(stripped)) return false
  if (/^pragma\b/i.test(stripped) && stripped.includes('=')) return false
  return !WRITE_ANYWHERE.test(stripped)
}

/**
 * Run a statement from the editor: reads through `database::query`, writes
 * through `database::execute`. Both are the worker's own functions, so an
 * engine policy that narrows an agent to reads narrows this panel the same
 * way, and writes land in the `database::row-changed` feed like any other.
 */
export async function runAdhocSql(host: Host, db: string, sql: string): Promise<AdhocResult> {
  // People type SQL with a trailing `;`; the drivers' prepared paths don't
  // all accept one. Strip it here instead of teaching that per driver.
  const statement = sql.trim().replace(/;+\s*$/, '')
  const started = performance.now()
  if (isReadOnlySql(statement)) {
    const result = await rpc.runSql(host, db, statement)
    return { result, durationMs: Math.round(performance.now() - started) }
  }
  const res = await rpc.execSql(host, db, statement)
  const rows = res.returned_rows ?? []
  const ddl = ddlInfo(statement)
  return {
    result: {
      rows,
      row_count: rows.length,
      columns: Object.keys(rows[0] ?? {}).map((name) => ({ name })),
    },
    durationMs: Math.round(performance.now() - started),
    write: {
      affectedRows: res.affected_rows,
      lastInsertId: res.last_insert_id ?? null,
      echo: ddl ? ddl.past : null,
    },
  }
}

/** Verb pairs for the statements that reshape the schema itself. */
const DDL_VERBS: Record<string, [string, string]> = {
  drop: ['drops', 'dropped'],
  truncate: ['truncates', 'truncated'],
  alter: ['alters', 'altered'],
}

const DDL_RE =
  /^(drop|truncate|alter)\b(?:\s+(table|view|index|database|schema|trigger))?(?:\s+if\s+(?:not\s+)?exists)?(?:\s+([`"']?[\w.$]+[`"']?))?/i

/**
 * The statements that reshape the schema, named in both tenses: the armed
 * note says what is about to happen ("drops table x"), the result line says
 * what did ("dropped table x"). Display-only — routing does not depend on it.
 */
export function ddlInfo(sql: string): { present: string; past: string } | null {
  const m = DDL_RE.exec(stripSqlComments(sql).trim())
  if (!m) return null
  const [present, past] = DDL_VERBS[m[1].toLowerCase()]
  const object = [m[2]?.toLowerCase(), m[3]?.replace(/[`"']/g, '')].filter(Boolean).join(' ')
  return {
    present: object ? `${present} ${object}` : present,
    past: object ? `${past} ${object}` : past,
  }
}

/**
 * Kept so the SQL panel can prefix a statement itself. Prefer
 * `database::explain`, which returns a plan tree rather than a grid of text.
 */
export function explainPrefix(driver: DbDriver): string {
  return driver === 'sqlite' ? 'EXPLAIN QUERY PLAN ' : 'EXPLAIN '
}

/**
 * The schema qualifying every listed table, or '' when there isn't exactly
 * one. A universal prefix (postgres's `public.` on every row) is repetition,
 * not information — display strips it; identity keeps the qualified name.
 */
export function commonSchema(names: readonly string[]): string {
  const schemas = new Set(names.map((n) => (n.includes('.') ? n.slice(0, n.indexOf('.')) : '')))
  return schemas.size === 1 ? [...schemas][0] : ''
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

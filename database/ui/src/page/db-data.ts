/**
 * Read-only RPC surface for the injected database page, built entirely on
 * `database::query` + `database::listDatabases` — the worker has no
 * dedicated introspection functions, so table discovery issues the
 * driver-native catalog query (sqlite_master / information_schema).
 * Mutations stay in agent flows behind the approval gate.
 *
 * Ported from the console's `lib/database.ts`: the catalog SQL and zod
 * parsing are verbatim; only the transport changed — every call now takes
 * the tab's `host` and routes through `host.iii.trigger(...)` instead of a
 * console-internal iii client.
 */

import type { Host } from '@iii-dev/console-ui'
import { z } from 'zod'

/* ---------------------------------------------------------------------- *
 * Response schemas — inlined from the console's `database::*` parsers so
 * this page carries no dependency on console-internal modules.
 * ---------------------------------------------------------------------- */

export const columnMetaSchema = z.object({
  name: z.string(),
  type: z.string().optional(),
})
export type ColumnMeta = z.infer<typeof columnMetaSchema>

export const queryResponseSchema = z.object({
  rows: z.array(z.record(z.string(), z.unknown())),
  row_count: z.number(),
  columns: z.array(columnMetaSchema).optional(),
})
export type QueryResponse = z.infer<typeof queryResponseSchema>

export const listDatabasesResponseSchema = z.object({
  databases: z.array(
    z.object({
      name: z.string(),
      driver: z.string().optional(),
      url: z.string().optional(),
      pool: z
        .object({
          max: z.number().optional(),
          idle_timeout_ms: z.number().optional(),
          acquire_timeout_ms: z.number().optional(),
        })
        .optional(),
      tls: z.object({ mode: z.string().optional() }).optional(),
    }),
  ),
  count: z.number().optional(),
})
export type ListDatabasesResponse = z.infer<typeof listDatabasesResponseSchema>

export interface TableSort {
  column: string
  dir: 'asc' | 'desc'
}

export type TypeCategory =
  | 'numeric'
  | 'text'
  | 'bool'
  | 'date'
  | 'json'
  | 'binary'
  | 'other'

/** Coarse category for a driver-reported column type. */
export function typeCategory(type: string | undefined): TypeCategory {
  if (!type) return 'other'
  const t = type.toLowerCase()
  if (/bool/.test(t)) return 'bool'
  if (/int|real|float|double|decimal|numeric|serial|money/.test(t)) {
    return 'numeric'
  }
  if (/date|time|year/.test(t)) return 'date'
  if (/json/.test(t)) return 'json'
  if (/blob|bytea|binary/.test(t)) return 'binary'
  if (/char|text|clob|uuid|enum/.test(t)) return 'text'
  return 'other'
}

/* ---------------------------------------------------------------------- */

const DATABASE_QUERY_FN = 'database::query'
const DATABASE_LIST_FN = 'database::listDatabases'

export const PAGE_SIZE = 50

export type DbDriver = 'sqlite' | 'postgres' | 'mysql' | 'unknown'

export interface DbInfo {
  name: string
  driver: DbDriver
  url?: string
  poolMax?: number
}

export interface DbTable {
  /** Display + query identifier; schema-qualified for postgres. */
  name: string
  kind: 'table' | 'view'
}

function parseDriver(raw: string | undefined): DbDriver {
  if (raw === 'sqlite' || raw === 'postgres' || raw === 'mysql') return raw
  return 'unknown'
}

async function triggerRaw(
  host: Host,
  fnId: string,
  payload: Record<string, unknown>,
): Promise<unknown> {
  return host.iii.trigger<unknown>(fnId, payload)
}

export async function listDbs(host: Host): Promise<DbInfo[]> {
  const res = listDatabasesResponseSchema.safeParse(
    await triggerRaw(host, DATABASE_LIST_FN, {}),
  )
  if (!res.success) return []
  return res.data.databases.map((db) => ({
    name: db.name,
    driver: parseDriver(db.driver),
    url: db.url,
    poolMax: db.pool?.max,
  }))
}

async function runQuery(
  host: Host,
  db: string,
  sql: string,
  params: unknown[] = [],
): Promise<QueryResponse> {
  const res = queryResponseSchema.safeParse(
    await triggerRaw(host, DATABASE_QUERY_FN, { db, sql, params }),
  )
  if (!res.success) {
    throw new Error('unexpected response shape from database::query')
  }
  return res.data
}

/* ---- identifier quoting ---- */

/**
 * Quote one identifier for the driver. Table names come from the driver's
 * own catalog, but quoting is still mandatory — names may contain
 * uppercase, spaces, or reserved words.
 */
export function quoteIdent(driver: DbDriver, ident: string): string {
  if (driver === 'mysql') {
    return `\`${ident.replaceAll('`', '``')}\``
  }
  return `"${ident.replaceAll('"', '""')}"`
}

/** Quote a possibly schema-qualified table reference (`schema.table`). */
function quoteTableRef(driver: DbDriver, table: string): string {
  if (driver === 'postgres' && table.includes('.')) {
    const [schema, ...rest] = table.split('.')
    return `${quoteIdent(driver, schema)}.${quoteIdent(driver, rest.join('.'))}`
  }
  return quoteIdent(driver, table)
}

/* ---- catalog queries ---- */

const TABLE_LIST_SQL: Record<Exclude<DbDriver, 'unknown'>, string> = {
  sqlite:
    "SELECT name, CASE type WHEN 'view' THEN 'view' ELSE 'table' END AS kind FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY kind, name",
  postgres:
    "SELECT CASE WHEN table_schema = 'public' THEN table_name ELSE table_schema || '.' || table_name END AS name, CASE table_type WHEN 'VIEW' THEN 'view' ELSE 'table' END AS kind FROM information_schema.tables WHERE table_type IN ('BASE TABLE', 'VIEW') AND table_schema NOT IN ('pg_catalog', 'information_schema') ORDER BY 2, 1",
  mysql:
    "SELECT table_name AS name, CASE TABLE_TYPE WHEN 'VIEW' THEN 'view' ELSE 'table' END AS kind FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY 2, 1",
}

const tableRowSchema = z.object({
  name: z.string(),
  kind: z.string().optional(),
})

export async function listTables(
  host: Host,
  db: string,
  driver: DbDriver,
): Promise<DbTable[]> {
  if (driver === 'unknown') {
    throw new Error(`cannot introspect tables for an unknown driver`)
  }
  const res = await runQuery(host, db, TABLE_LIST_SQL[driver])
  return res.rows
    .map((row) => tableRowSchema.safeParse(row))
    .filter((p) => p.success)
    .map((p) => ({
      name: p.data.name,
      kind: p.data.kind === 'view' ? ('view' as const) : ('table' as const),
    }))
}

/* ---- indexes ---- */

export interface IndexInfo {
  name: string
  unique: boolean
  /** Column list or full definition, driver-dependent. */
  detail: string
}

const sqliteIndexSchema = z.object({
  name: z.string(),
  unique: z.number(),
  origin: z.string().optional(),
})

const pgIndexSchema = z.object({
  name: z.string(),
  definition: z.string(),
})

const mysqlIndexSchema = z.object({
  name: z.string(),
  unique: z.number(),
  columns: z.string(),
})

export async function tableIndexes(
  host: Host,
  db: string,
  driver: DbDriver,
  table: string,
): Promise<IndexInfo[]> {
  if (driver === 'sqlite') {
    const res = await runQuery(
      host,
      db,
      `PRAGMA index_list(${quoteIdent(driver, table)})`,
    )
    return res.rows
      .map((row) => sqliteIndexSchema.safeParse(row))
      .filter((p) => p.success)
      .map((p) => ({
        name: p.data.name,
        unique: p.data.unique === 1,
        detail: p.data.origin === 'pk' ? 'primary key' : '',
      }))
  }
  if (driver === 'postgres') {
    const { schema, name } = splitTableRef(table)
    const res = await runQuery(
      host,
      db,
      `SELECT indexname AS name, indexdef AS definition FROM pg_indexes WHERE schemaname = $1 AND tablename = $2 ORDER BY indexname`,
      [schema ?? 'public', name],
    )
    return res.rows
      .map((row) => pgIndexSchema.safeParse(row))
      .filter((p) => p.success)
      .map((p) => ({
        name: p.data.name,
        unique: /\bUNIQUE\b/i.test(p.data.definition),
        detail: p.data.definition.replace(/^.*USING /i, ''),
      }))
  }
  if (driver === 'mysql') {
    const res = await runQuery(
      host,
      db,
      `SELECT INDEX_NAME AS name, MIN(NON_UNIQUE) = 0 AS \`unique\`, GROUP_CONCAT(COLUMN_NAME ORDER BY SEQ_IN_INDEX) AS columns FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? GROUP BY INDEX_NAME ORDER BY INDEX_NAME`,
      [splitTableRef(table).name],
    )
    return res.rows
      .map((row) => mysqlIndexSchema.safeParse(row))
      .filter((p) => p.success)
      .map((p) => ({
        name: p.data.name,
        unique: p.data.unique === 1,
        detail: `(${p.data.columns})`,
      }))
  }
  return []
}

/* ---- ad-hoc read-only queries ---- */

/** Leading keyword of a statement (comments stripped). */
function leadingKeyword(sql: string): string {
  const cleaned = sql
    .replace(/\/\*[\s\S]*?\*\//g, ' ')
    .replace(/--[^\n]*/g, ' ')
    .trim()
  return cleaned.split(/[^a-zA-Z_]+/, 1)[0]?.toUpperCase() ?? ''
}

const READ_ONLY_KEYWORDS = new Set([
  'SELECT',
  'WITH',
  'EXPLAIN',
  'PRAGMA',
  'SHOW',
  'DESCRIBE',
  'VALUES',
])

/** Console-side gate: the page only issues read statements. */
export function isReadOnlySql(sql: string): boolean {
  return READ_ONLY_KEYWORDS.has(leadingKeyword(sql))
}

export interface AdhocResult {
  result: QueryResponse
  durationMs: number
}

/**
 * Run one read-only statement from the SQL panel. Write verbs are rejected
 * before they reach the worker — mutations stay in agent flows behind the
 * approval gate.
 */
export async function runReadOnlySql(
  host: Host,
  db: string,
  sql: string,
): Promise<AdhocResult> {
  if (!isReadOnlySql(sql)) {
    throw new Error(
      'read-only console: only SELECT / WITH / EXPLAIN / PRAGMA / SHOW run here. write statements go through agent flows and the approval gate.',
    )
  }
  const started = performance.now()
  const result = await runQuery(host, db, sql)
  return { result, durationMs: Math.round(performance.now() - started) }
}

/** Driver-native EXPLAIN prefix for the SQL panel's explain toggle. */
export function explainPrefix(driver: DbDriver): string {
  if (driver === 'sqlite') return 'EXPLAIN QUERY PLAN '
  return 'EXPLAIN '
}

export async function countTableRows(
  host: Host,
  db: string,
  driver: DbDriver,
  table: string,
): Promise<number | null> {
  const ref = quoteTableRef(driver, table)
  const res = await runQuery(host, db, `SELECT COUNT(*) AS n FROM ${ref}`)
  const n = res.rows[0]?.n
  if (typeof n === 'number') return n
  if (typeof n === 'string' && /^\d+$/.test(n)) return Number(n)
  return null
}

/* ---- column introspection ---- */

export interface ColumnInfo {
  name: string
  type: string
  nullable: boolean
  pk: boolean
  /** Referenced `table.column` when this column is a foreign key. */
  fkTarget?: string
}

/** Split a possibly schema-qualified table into schema + bare name. */
function splitTableRef(table: string): { schema: string | null; name: string } {
  if (!table.includes('.')) return { schema: null, name: table }
  const [schema, ...rest] = table.split('.')
  return { schema, name: rest.join('.') }
}

const sqlitePragmaColumnSchema = z.object({
  name: z.string(),
  type: z.string(),
  notnull: z.number(),
  pk: z.number(),
})

const sqlitePragmaFkSchema = z.object({
  table: z.string(),
  from: z.string(),
  to: z.string().nullable(),
})

async function sqliteColumns(
  host: Host,
  db: string,
  table: string,
): Promise<ColumnInfo[]> {
  const ref = quoteIdent('sqlite', table)
  const cols = await runQuery(host, db, `PRAGMA table_info(${ref})`)
  let fks: Map<string, string> = new Map()
  try {
    const fkRes = await runQuery(host, db, `PRAGMA foreign_key_list(${ref})`)
    fks = new Map(
      fkRes.rows
        .map((row) => sqlitePragmaFkSchema.safeParse(row))
        .filter((p) => p.success)
        .map((p) => [p.data.from, `${p.data.table}.${p.data.to ?? 'rowid'}`]),
    )
  } catch {
    // foreign_key_list errors on tables without FKs in some builds; treat
    // as "no foreign keys" rather than failing the whole column read.
  }
  return cols.rows
    .map((row) => sqlitePragmaColumnSchema.safeParse(row))
    .filter((p) => p.success)
    .map((p) => ({
      name: p.data.name,
      type: p.data.type || 'any',
      nullable: p.data.notnull === 0,
      pk: p.data.pk > 0,
      fkTarget: fks.get(p.data.name),
    }))
}

const infoSchemaColumnSchema = z.object({
  name: z.string(),
  type: z.string(),
  nullable: z.string(),
  key_kind: z.string().nullable().optional(),
  fk_target: z.string().nullable().optional(),
})

async function postgresColumns(
  host: Host,
  db: string,
  table: string,
): Promise<ColumnInfo[]> {
  const { schema, name } = splitTableRef(table)
  const res = await runQuery(
    host,
    db,
    `SELECT c.column_name AS name, c.data_type AS type, c.is_nullable AS nullable,
       (SELECT tc.constraint_type FROM information_schema.table_constraints tc
          JOIN information_schema.key_column_usage kcu
            ON kcu.constraint_name = tc.constraint_name
           AND kcu.table_schema = tc.table_schema
         WHERE tc.table_schema = c.table_schema AND tc.table_name = c.table_name
           AND kcu.column_name = c.column_name AND tc.constraint_type = 'PRIMARY KEY'
         LIMIT 1) AS key_kind,
       (SELECT ccu.table_name || '.' || ccu.column_name
          FROM information_schema.table_constraints tc
          JOIN information_schema.key_column_usage kcu
            ON kcu.constraint_name = tc.constraint_name
           AND kcu.table_schema = tc.table_schema
          JOIN information_schema.constraint_column_usage ccu
            ON ccu.constraint_name = tc.constraint_name
         WHERE tc.table_schema = c.table_schema AND tc.table_name = c.table_name
           AND kcu.column_name = c.column_name AND tc.constraint_type = 'FOREIGN KEY'
         LIMIT 1) AS fk_target
     FROM information_schema.columns c
     WHERE c.table_schema = $1 AND c.table_name = $2
     ORDER BY c.ordinal_position`,
    [schema ?? 'public', name],
  )
  return res.rows
    .map((row) => infoSchemaColumnSchema.safeParse(row))
    .filter((p) => p.success)
    .map((p) => ({
      name: p.data.name,
      type: p.data.type,
      nullable: p.data.nullable === 'YES',
      pk: p.data.key_kind === 'PRIMARY KEY',
      fkTarget: p.data.fk_target ?? undefined,
    }))
}

async function mysqlColumns(
  host: Host,
  db: string,
  table: string,
): Promise<ColumnInfo[]> {
  const { name } = splitTableRef(table)
  const res = await runQuery(
    host,
    db,
    `SELECT c.COLUMN_NAME AS name, c.COLUMN_TYPE AS type, c.IS_NULLABLE AS nullable,
       c.COLUMN_KEY AS key_kind,
       (SELECT CONCAT(kcu.REFERENCED_TABLE_NAME, '.', kcu.REFERENCED_COLUMN_NAME)
          FROM information_schema.KEY_COLUMN_USAGE kcu
         WHERE kcu.TABLE_SCHEMA = c.TABLE_SCHEMA AND kcu.TABLE_NAME = c.TABLE_NAME
           AND kcu.COLUMN_NAME = c.COLUMN_NAME
           AND kcu.REFERENCED_TABLE_NAME IS NOT NULL
         LIMIT 1) AS fk_target
     FROM information_schema.COLUMNS c
     WHERE c.TABLE_SCHEMA = DATABASE() AND c.TABLE_NAME = ?
     ORDER BY c.ORDINAL_POSITION`,
    [name],
  )
  return res.rows
    .map((row) => infoSchemaColumnSchema.safeParse(row))
    .filter((p) => p.success)
    .map((p) => ({
      name: p.data.name,
      type: p.data.type,
      nullable: p.data.nullable === 'YES',
      pk: p.data.key_kind === 'PRI',
      fkTarget: p.data.fk_target ?? undefined,
    }))
}

export async function tableColumns(
  host: Host,
  db: string,
  driver: DbDriver,
  table: string,
): Promise<ColumnInfo[]> {
  if (driver === 'sqlite') return sqliteColumns(host, db, table)
  if (driver === 'postgres') return postgresColumns(host, db, table)
  if (driver === 'mysql') return mysqlColumns(host, db, table)
  return []
}

/* ---- paged reads ---- */

export interface TablePage {
  result: QueryResponse
  page: number
  pageSize: number
}

export async function fetchTablePage(
  host: Host,
  db: string,
  driver: DbDriver,
  table: string,
  page: number,
  pageSize: number = PAGE_SIZE,
  sort?: TableSort | null,
): Promise<TablePage> {
  const ref = quoteTableRef(driver, table)
  const offset = page * pageSize
  const orderBy = sort
    ? ` ORDER BY ${quoteIdent(driver, sort.column)} ${sort.dir === 'desc' ? 'DESC' : 'ASC'}`
    : ''
  const result = await runQuery(
    host,
    db,
    `SELECT * FROM ${ref}${orderBy} LIMIT ${pageSize} OFFSET ${offset}`,
  )
  return { result, page, pageSize }
}

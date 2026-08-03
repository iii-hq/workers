/**
 * Typed calls into the worker's own functions.
 *
 * The page used to build catalog SQL in the browser — around 520 lines of
 * `sqlite_master` / `information_schema` / `PRAGMA` per driver. That logic now
 * lives in the worker, where it is written once, tested against all three
 * drivers in CI, and callable by any agent on the bus. This module is the
 * whole of what replaced it: one wrapper per function, no SQL.
 *
 * Everything is parsed with zod at the boundary, because a worker one version
 * behind will happily return a shape this page does not expect, and a missing
 * field should surface as a readable error rather than `undefined` three
 * components deep.
 */

import type { Host } from '@iii-dev/console-ui'
import { z } from 'zod'

export const DB = {
  listDatabases: 'database::listDatabases',
  listTables: 'database::listTables',
  describeTable: 'database::describeTable',
  describeSchema: 'database::describeSchema',
  browseTable: 'database::browseTable',
  query: 'database::query',
  explain: 'database::explain',
  columnStats: 'database::columnStats',
  health: 'database::health',
  schemaDiagram: 'database::schemaDiagram',
  saveQuery: 'database::saveQuery',
  listSavedQueries: 'database::listSavedQueries',
  deleteSavedQuery: 'database::deleteSavedQuery',
  history: 'database::history',
  getTableView: 'database::getTableView',
  saveTableView: 'database::saveTableView',
} as const

export type DbFunction = (typeof DB)[keyof typeof DB]

/** Functions older installed workers will not have. */
export const OPTIONAL_FNS: readonly DbFunction[] = [
  DB.listTables,
  DB.describeTable,
  DB.describeSchema,
  DB.browseTable,
  DB.explain,
  DB.columnStats,
  DB.health,
  DB.schemaDiagram,
  DB.saveQuery,
  DB.listSavedQueries,
  DB.deleteSavedQuery,
  DB.history,
  DB.getTableView,
  DB.saveTableView,
]

async function call<T>(host: Host, fn: DbFunction, payload: Record<string, unknown>, schema: z.ZodType<T>): Promise<T> {
  const raw = await host.iii.trigger<unknown>(fn, payload)
  const parsed = schema.safeParse(raw)
  if (!parsed.success) {
    throw new Error(`unexpected response shape from ${fn}`)
  }
  return parsed.data
}

/* ---------------- shared shapes ---------------- */

export const columnMetaSchema = z.object({
  name: z.string(),
  type: z.string().optional(),
})
export type ColumnMeta = z.infer<typeof columnMetaSchema>

export const queryResponseSchema = z.object({
  rows: z.array(z.record(z.string(), z.unknown())),
  row_count: z.number(),
  columns: z.array(columnMetaSchema),
})
export type QueryResponse = z.infer<typeof queryResponseSchema>

export const foreignKeySchema = z.object({
  schema: z.string().nullish(),
  table: z.string(),
  column: z.string(),
})
export type ForeignKeyRef = z.infer<typeof foreignKeySchema>

export const columnDescSchema = z.object({
  name: z.string(),
  type: z.string(),
  nullable: z.boolean(),
  default_value: z.string().nullish(),
  primary_key: z.boolean(),
  position: z.number(),
  foreign_key: foreignKeySchema.nullish(),
})
export type ColumnDesc = z.infer<typeof columnDescSchema>

export const indexDescSchema = z.object({
  name: z.string(),
  unique: z.boolean(),
  primary: z.boolean(),
  columns: z.array(z.string()),
})
export type IndexDesc = z.infer<typeof indexDescSchema>

export const tableRefSchema = z.object({
  name: z.string(),
  schema: z.string().nullish(),
  kind: z.enum(['table', 'view']),
})
export type TableRef = z.infer<typeof tableRefSchema>

export const tableDescriptionSchema = z.object({
  table: z.string(),
  schema: z.string().nullish(),
  kind: z.enum(['table', 'view']),
  columns: z.array(columnDescSchema),
  indexes: z.array(indexDescSchema),
  row_count_estimate: z.number().nullish(),
})
export type TableDescription = z.infer<typeof tableDescriptionSchema>

/* ---------------- filters and sorts ---------------- */

export type FilterOp =
  | 'contains'
  | 'not_contains'
  | 'equals'
  | 'not_equals'
  | 'starts_with'
  | 'ends_with'
  | 'gt'
  | 'gte'
  | 'lt'
  | 'lte'
  | 'between'
  | 'is_true'
  | 'is_false'
  | 'is_null'
  | 'is_not_null'
  | 'is_empty'
  | 'in'
  | 'not_in'

export interface FilterSpec {
  column: string
  op: FilterOp
  value?: unknown
  value2?: unknown
  /** Operands for `in` / `not_in`. */
  values?: unknown[]
  case_sensitive?: boolean
  /** Kept in the bar but not applied. */
  disabled?: boolean
}

/** Human labels, grouped the way a picker should present them. */
export const OP_LABEL: Record<FilterOp, string> = {
  equals: 'is',
  not_equals: 'is not',
  contains: 'contains',
  not_contains: 'does not contain',
  starts_with: 'starts with',
  ends_with: 'ends with',
  gt: 'greater than',
  gte: 'at least',
  lt: 'less than',
  lte: 'at most',
  between: 'between',
  in: 'is one of',
  not_in: 'is none of',
  is_true: 'is true',
  is_false: 'is false',
  is_null: 'is null',
  is_not_null: 'is not null',
  is_empty: 'is empty',
}

export type SortMode = 'default' | 'natural' | 'length' | 'absolute_value' | 'random'

export interface SortSpec {
  column: string
  direction: 'asc' | 'desc'
  nulls?: 'first' | 'last'
  mode?: SortMode
}

/**
 * Operators that make sense for a column, by its declared type. Offering
 * `contains` on a boolean is how a filter bar starts feeling untrustworthy.
 */
export function opsFor(category: TypeCategory): FilterOp[] {
  switch (category) {
    case 'numeric':
      return ['equals', 'not_equals', 'gt', 'gte', 'lt', 'lte', 'between', 'in', 'not_in', 'is_null', 'is_not_null']
    case 'bool':
      return ['is_true', 'is_false', 'is_null', 'is_not_null']
    case 'date':
      return ['equals', 'gt', 'lt', 'between', 'is_null', 'is_not_null']
    default:
      return [
        'contains',
        'not_contains',
        'equals',
        'not_equals',
        'starts_with',
        'ends_with',
        'in',
        'not_in',
        'is_empty',
        'is_null',
        'is_not_null',
      ]
  }
}

/** Operators taking a list rather than a single operand. */
export const SET_OPS: ReadonlySet<FilterOp> = new Set(['in', 'not_in'])

/** Operators taking no operand — a chip using one is complete immediately. */
const NULLARY: ReadonlySet<FilterOp> = new Set(['is_true', 'is_false', 'is_null', 'is_not_null', 'is_empty'])

/**
 * Whether a chip is ready to send. An incomplete chip stays in the bar as a
 * draft and is never sent — the worker would reject it, and a round trip to
 * learn that would make typing feel broken.
 */
export function isComplete(f: FilterSpec): boolean {
  if (!f.column) return false
  if (NULLARY.has(f.op)) return true
  if (f.op === 'between') return hasValue(f.value) && hasValue(f.value2)
  // `IN ()` is a syntax error everywhere, so an empty list is a draft.
  if (SET_OPS.has(f.op)) return (f.values ?? []).some(hasValue)
  return hasValue(f.value)
}

function hasValue(v: unknown): boolean {
  return v !== undefined && v !== null && v !== ''
}

export type TypeCategory = 'numeric' | 'text' | 'bool' | 'date' | 'json' | 'binary' | 'other'

/** Coarse category for a driver-reported column type. */
export function typeCategory(type: string | undefined): TypeCategory {
  if (!type) return 'other'
  const t = type.toLowerCase()
  if (/bool/.test(t)) return 'bool'
  if (/int|real|float|double|decimal|numeric|serial|money/.test(t)) return 'numeric'
  if (/date|time|year/.test(t)) return 'date'
  if (/json/.test(t)) return 'json'
  if (/blob|bytea|binary/.test(t)) return 'binary'
  if (/char|text|clob|uuid|enum/.test(t)) return 'text'
  return 'other'
}

/* ---------------- calls ---------------- */

export const PAGE_SIZE = 50
export type DbDriver = 'sqlite' | 'postgres' | 'mysql' | 'unknown'

const listDatabasesSchema = z.object({
  databases: z.array(
    z.object({
      name: z.string(),
      driver: z.string(),
      url: z.string().optional(),
      pool: z.object({ max: z.number().optional() }).optional(),
    }),
  ),
  count: z.number(),
})

export interface DbInfo {
  name: string
  driver: DbDriver
  url?: string
  poolMax?: number
}

function parseDriver(raw: string | undefined): DbDriver {
  return raw === 'sqlite' || raw === 'postgres' || raw === 'mysql' ? raw : 'unknown'
}

export async function listDbs(host: Host): Promise<DbInfo[]> {
  const res = await call(host, DB.listDatabases, {}, listDatabasesSchema)
  return res.databases.map((d) => ({
    name: d.name,
    driver: parseDriver(d.driver),
    url: d.url,
    poolMax: d.pool?.max,
  }))
}

export async function listTables(host: Host, db: string): Promise<TableRef[]> {
  const res = await call(host, DB.listTables, { db }, z.object({ tables: z.array(tableRefSchema), count: z.number() }))
  return res.tables
}

export async function describeTable(
  host: Host,
  db: string,
  table: string,
  schema?: string | null,
): Promise<TableDescription> {
  return call(host, DB.describeTable, { db, table, schema }, tableDescriptionSchema)
}

export async function describeSchema(host: Host, db: string, includeIndexes = false): Promise<TableDescription[]> {
  const res = await call(
    host,
    DB.describeSchema,
    { db, include_indexes: includeIndexes },
    z.object({
      tables: z.array(tableDescriptionSchema),
      count: z.number(),
      truncated: z.boolean(),
    }),
  )
  return res.tables
}

const browseSchema = z.object({
  rows: z.array(z.record(z.string(), z.unknown())),
  columns: z.array(columnMetaSchema),
  page: z.number(),
  page_size: z.number(),
  has_more: z.boolean(),
  total: z.number().nullish(),
})
export type BrowseResult = z.infer<typeof browseSchema>

export interface BrowseOptions {
  page?: number
  pageSize?: number
  sort?: SortSpec[]
  filters?: FilterSpec[]
  /** Skip the filtered COUNT(*) while the user is still editing a chip. */
  includeTotal?: boolean
}

export async function browseTable(
  host: Host,
  db: string,
  table: string,
  schema: string | null | undefined,
  opts: BrowseOptions = {},
): Promise<BrowseResult> {
  return call(
    host,
    DB.browseTable,
    {
      db,
      table,
      schema,
      page: Math.max(0, Math.trunc(opts.page ?? 0)),
      page_size: Math.max(1, Math.trunc(opts.pageSize ?? PAGE_SIZE)),
      sort: opts.sort ?? [],
      // Draft chips never leave the browser.
      filters: (opts.filters ?? []).filter(isComplete),
      include_total: opts.includeTotal ?? true,
    },
    browseSchema,
  )
}

/** Ad-hoc SQL from the editor. Still goes through `database::query`. */
export async function runSql(host: Host, db: string, sql: string): Promise<QueryResponse> {
  return call(host, DB.query, { db, sql }, queryResponseSchema)
}

export const planNodeSchema: z.ZodType<PlanNode> = z.lazy(() =>
  z.object({
    id: z.number(),
    parent: z.number().nullish(),
    label: z.string(),
    node_class: z.string(),
    relation: z.string().nullish(),
    cost_startup: z.number().nullish(),
    cost_total: z.number().nullish(),
    rows_estimated: z.number().nullish(),
    rows_actual: z.number().nullish(),
    width: z.number().nullish(),
    time_ms: z.number().nullish(),
    loops: z.number().nullish(),
    detail: z.string(),
    children: z.array(planNodeSchema),
  }),
)

export interface PlanNode {
  id: number
  parent?: number | null
  label: string
  node_class: string
  relation?: string | null
  cost_startup?: number | null
  cost_total?: number | null
  rows_estimated?: number | null
  rows_actual?: number | null
  width?: number | null
  time_ms?: number | null
  loops?: number | null
  detail: string
  children: PlanNode[]
}

const explainSchema = z.object({
  format: z.string(),
  analyzed: z.boolean(),
  root: planNodeSchema.nullish(),
  warnings: z.array(
    z.object({
      node_id: z.number(),
      kind: z.string(),
      message: z.string(),
      severity: z.string(),
    }),
  ),
  raw: z.unknown().nullish(),
})
export type ExplainResult = z.infer<typeof explainSchema>

export async function explain(host: Host, db: string, sql: string, analyze = false): Promise<ExplainResult> {
  return call(host, DB.explain, { db, sql, analyze }, explainSchema)
}

const columnStatSchema = z.object({
  name: z.string(),
  row_count: z.number().nullish(),
  distinct_count: z.number().nullish(),
  null_count: z.number().nullish(),
  null_fraction: z.number().nullish(),
  min: z.unknown().nullish(),
  max: z.unknown().nullish(),
  mean: z.number().nullish(),
  top_values: z.array(z.object({ value: z.unknown(), count: z.number() })),
  source: z.enum(['planner', 'computed']),
})
export type ColumnStat = z.infer<typeof columnStatSchema>

export async function columnStats(
  host: Host,
  db: string,
  table: string,
  columns: string[],
  exact = false,
): Promise<{ columns: ColumnStat[]; approximate: boolean }> {
  return call(
    host,
    DB.columnStats,
    { db, table, columns, exact },
    z.object({
      table: z.string(),
      schema: z.string().nullish(),
      columns: z.array(columnStatSchema),
      approximate: z.boolean(),
    }),
  )
}

const probeSchema = <T extends z.ZodTypeAny>(data: T) =>
  z.discriminatedUnion('status', [
    z.object({ status: z.literal('available'), data }),
    z.object({ status: z.literal('unsupported'), reason: z.string() }),
    z.object({ status: z.literal('denied'), reason: z.string() }),
  ])

const healthSchema = z.object({
  db: z.string(),
  driver: z.string(),
  worker_version: z.string(),
  pool: z.object({
    max: z.number(),
    size: z.number().nullish(),
    idle: z.number().nullish(),
    waiting: z.number().nullish(),
  }),
  active_queries: probeSchema(
    z.array(
      z.object({
        id: z.string(),
        sql: z.string(),
        state: z.string().nullish(),
        duration_ms: z.number().nullish(),
        user: z.string().nullish(),
      }),
    ),
  ),
  table_sizes: probeSchema(
    z.array(
      z.object({
        table: z.string(),
        schema: z.string().nullish(),
        total_bytes: z.number().nullish(),
        index_bytes: z.number().nullish(),
        row_estimate: z.number().nullish(),
      }),
    ),
  ),
  locks: probeSchema(
    z.array(
      z.object({
        blocked_id: z.string(),
        blocked_sql: z.string(),
        blocking_id: z.string(),
        blocking_sql: z.string().nullish(),
        relation: z.string().nullish(),
      }),
    ),
  ),
  cache: probeSchema(
    z.object({
      hit_ratio: z.number(),
      blocks_hit: z.number(),
      blocks_read: z.number(),
    }),
  ),
})
export type HealthReport = z.infer<typeof healthSchema>

export async function health(host: Host, db: string): Promise<HealthReport> {
  return call(host, DB.health, { db }, healthSchema)
}

const diagramSchema = z.object({
  nodes: z.array(
    z.object({
      table: z.string(),
      schema: z.string().nullish(),
      x: z.number(),
      y: z.number(),
      w: z.number(),
      h: z.number(),
      rank: z.number(),
      degree: z.number(),
      hidden_columns: z.number(),
      columns: z.array(
        z.object({
          name: z.string(),
          type: z.string(),
          primary_key: z.boolean(),
          foreign_key: z.boolean(),
          nullable: z.boolean(),
        }),
      ),
    }),
  ),
  edges: z.array(
    z.object({
      from: z.string(),
      from_column: z.string(),
      to: z.string(),
      to_column: z.string(),
      points: z.array(z.object({ x: z.number(), y: z.number() })),
      self_loop: z.boolean(),
    }),
  ),
  width: z.number(),
  height: z.number(),
  isolated: z.array(z.string()),
  // Optional so an older worker still renders — it just draws no boundaries.
  components: z
    .array(
      z.object({
        index: z.number(),
        tables: z.array(z.string()),
        hub: z.string().nullish(),
        x: z.number(),
        y: z.number(),
        w: z.number(),
        h: z.number(),
      }),
    )
    .nullish()
    .transform((v) => v ?? []),
  crossings: z.number(),
  truncated: z.boolean(),
  focus: z.string().nullish(),
  frontier: z
    .array(z.string())
    .nullish()
    .transform((v) => v ?? []),
})
export type SchemaDiagram = z.infer<typeof diagramSchema>

export interface DiagramOptions {
  /** Draw only this table's neighbourhood. */
  focus?: string | null
  /** Foreign-key hops out from `focus`. */
  depth?: number
}

export async function schemaDiagram(host: Host, db: string, opts: DiagramOptions = {}): Promise<SchemaDiagram> {
  return call(
    host,
    DB.schemaDiagram,
    // `focus` is omitted rather than sent as null: the worker treats absence
    // as "the whole schema", and an explicit null would have to mean the same
    // thing in two places.
    opts.focus ? { db, focus: opts.focus, depth: opts.depth ?? 1 } : { db },
    diagramSchema,
  )
}

const savedQuerySchema = z.object({
  id: z.string(),
  name: z.string(),
  sql: z.string(),
  description: z.string().nullish(),
  saved_at: z.string(),
})
export type SavedQuery = z.infer<typeof savedQuerySchema>

export async function listSavedQueries(host: Host, db: string): Promise<SavedQuery[]> {
  const res = await call(
    host,
    DB.listSavedQueries,
    { db },
    z.object({ queries: z.array(savedQuerySchema), count: z.number() }),
  )
  return res.queries
}

export async function saveQuery(
  host: Host,
  db: string,
  name: string,
  sql: string,
): Promise<{ id: string; replaced: boolean }> {
  return call(host, DB.saveQuery, { db, name, sql }, z.object({ id: z.string(), replaced: z.boolean() }))
}

export async function deleteSavedQuery(host: Host, db: string, id: string): Promise<boolean> {
  const res = await call(host, DB.deleteSavedQuery, { db, id }, z.object({ deleted: z.boolean() }))
  return res.deleted
}

const historySchema = z.object({
  entries: z.array(
    z.object({
      sql: z.string(),
      verb: z.string(),
      duration_ms: z.number().nullish(),
      row_count: z.number().nullish(),
      at: z.string(),
    }),
  ),
  count: z.number(),
})
export type HistoryEntry = z.infer<typeof historySchema>['entries'][number]

const tableViewSchema = z.object({
  widths: z
    .record(z.string(), z.number())
    .nullish()
    .transform((v) => v ?? {}),
  hidden: z
    .array(z.string())
    .nullish()
    .transform((v) => v ?? []),
  order: z
    .array(z.string())
    .nullish()
    .transform((v) => v ?? []),
})
export type TableView = z.infer<typeof tableViewSchema>

export async function getTableView(host: Host, db: string, table: string): Promise<TableView> {
  return call(host, DB.getTableView, { db, table }, tableViewSchema)
}

export async function saveTableView(host: Host, db: string, table: string, view: TableView): Promise<boolean> {
  const r = await call(host, DB.saveTableView, { db, table, ...view }, z.object({ saved: z.boolean() }))
  return r.saved
}

export async function history(host: Host, db: string, limit = 25): Promise<HistoryEntry[]> {
  const res = await call(host, DB.history, { db, limit }, historySchema)
  return res.entries
}

/**
 * Defensive parsing for database::* trigger messages. Every accessor
 * tolerates absent/odd shapes and returns undefined rather than throwing —
 * the console fences injected renderers, but a throw degrades the card to
 * an error chip, and a raw-JSON fallback is strictly better.
 */

export const DB_PREFIX = 'database::'

/** `{ content: [...], details }` harness result envelope → details. */
export function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) return obj.details
  return value
}

export function isErrorOutput(value: unknown): boolean {
  return (
    !!value &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    'error' in (value as Record<string, unknown>)
  )
}

export function asRecord(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {}
  return value as Record<string, unknown>
}

export function asString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

export function asNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

export function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : []
}

export interface DbRequest {
  db?: string
  sql?: string
  params?: unknown[]
  transactionId?: string
  handleId?: string
  isolation?: string
  /** executeBatch: `string | {sql, params}` items; transaction: `{sql, params}`. */
  statements?: { sql?: string; params?: unknown[] }[]
}

export function parseRequest(input: unknown): DbRequest {
  const obj = asRecord(input)
  const statements = asArray(obj.statements).map((s) => {
    if (typeof s === 'string') return { sql: s }
    const rec = asRecord(s)
    return { sql: asString(rec.sql), params: Array.isArray(rec.params) ? rec.params : undefined }
  })
  return {
    db: asString(obj.db),
    // `query` is an accepted alias for `sql` on query/execute.
    sql: asString(obj.sql) ?? asString(obj.query),
    params: Array.isArray(obj.params) && obj.params.length > 0 ? obj.params : undefined,
    transactionId: asString(obj.transaction_id),
    handleId: asString(obj.handle_id),
    isolation: asString(obj.isolation),
    statements: statements.length > 0 ? statements : undefined,
  }
}

/** QueryResp — rows are row-of-objects keyed by column name. */
export interface QueryLike {
  rows: Record<string, unknown>[]
  rowCount?: number
  columns: string[]
}

export function parseQueryResp(details: unknown): QueryLike | undefined {
  const obj = asRecord(details)
  if (!Array.isArray(obj.rows)) return undefined
  const rows = obj.rows.map(asRecord)
  const columns = asArray(obj.columns)
    .map((c) => asString(asRecord(c).name))
    .filter((n): n is string => !!n)
  // Fall back to the first row's keys when columns metadata is absent.
  const cols = columns.length > 0 ? columns : Object.keys(rows[0] ?? {})
  return { rows, rowCount: asNumber(obj.row_count), columns: cols }
}

/** ExecuteResp / TxExecuteResp. */
export interface ExecuteLike {
  affectedRows?: number
  lastInsertId?: string
  returnedRows: Record<string, unknown>[]
}

export function parseExecuteResp(details: unknown): ExecuteLike {
  const obj = asRecord(details)
  return {
    affectedRows: asNumber(obj.affected_rows),
    lastInsertId: asString(obj.last_insert_id),
    returnedRows: asArray(obj.returned_rows).map(asRecord),
  }
}

/** TxResp — executeBatch and transaction share it. */
export interface TxLike {
  committed?: boolean
  failedIndex?: number
  steps: { affectedRows?: number }[]
}

export function parseTxResp(details: unknown): TxLike {
  const obj = asRecord(details)
  return {
    committed: typeof obj.committed === 'boolean' ? obj.committed : undefined,
    failedIndex: asNumber(obj.failed_index),
    steps: asArray(obj.results).map((r) => ({
      affectedRows: asNumber(asRecord(r).affected_rows),
    })),
  }
}

export interface DatabaseInfoLike {
  name?: string
  driver?: string
  url?: string
  poolMax?: number
}

export function parseListDatabases(details: unknown): DatabaseInfoLike[] {
  const obj = asRecord(details)
  return asArray(obj.databases).map((d) => {
    const rec = asRecord(d)
    return {
      name: asString(rec.name),
      driver: asString(rec.driver),
      url: asString(rec.url),
      poolMax: asNumber(asRecord(rec.pool).max),
    }
  })
}

/** Shorten opaque ids (uuids, tx ids) for chip display. */
export function shortId(id: string): string {
  return id.length > 13 ? `${id.slice(0, 13)}…` : id
}

/** Compact single-line rendering of a table cell value. */
export function cellText(v: unknown): { text: string; isNull: boolean } {
  if (v === null || v === undefined) return { text: 'null', isNull: true }
  if (typeof v === 'string') {
    return { text: v.length > 200 ? `${v.slice(0, 200)}…` : v, isNull: false }
  }
  if (typeof v === 'object') {
    const json = JSON.stringify(v)
    return { text: json.length > 200 ? `${json.slice(0, 200)}…` : json, isNull: false }
  }
  return { text: String(v), isNull: false }
}

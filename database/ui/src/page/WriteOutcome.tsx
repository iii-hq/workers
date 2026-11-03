/**
 * What a write did, when it returned no rows to draw.
 *
 * "Statement ran" said neither what changed nor whether anything was saved.
 * This reads the answer off what the worker reports (rows affected, the last
 * insert id, its timing) plus the statement itself (verb and table, via
 * `describeWrite`): a headline in plain words, whether it was saved and
 * where, the numbers, the statement, and the way to go and look at the rows.
 * Nothing here asks the worker for more than it already returned.
 */

import { Button } from '@iii-dev/console-ui'
import { CircleCheck, CircleDashed, Table2 } from 'lucide-react'
import { type AdhocResult, type DbDriver, writeHeadline } from './db-data'

/** Which drivers take `RETURNING` on INSERT/UPDATE/DELETE. */
const RETURNING_DRIVERS: ReadonlySet<DbDriver> = new Set(['postgres', 'sqlite'])

const PAST: Record<string, string> = { insert: 'inserted', update: 'updated', delete: 'deleted' }

/** The listed table a statement's target names, schema-qualified or not. */
function findTable(target: string, tables: readonly string[]): string | undefined {
  const t = target.toLowerCase()
  return tables.find((name) => {
    const n = name.toLowerCase()
    return n === t || n.endsWith(`.${t}`) || t.endsWith(`.${n}`)
  })
}

export function WriteOutcome({
  outcome,
  sql,
  db,
  driver,
  at,
  tables,
  onOpenTable,
}: {
  outcome: AdhocResult
  /** The statement that ran. */
  sql: string
  db: string
  driver?: DbDriver
  /** When it committed (ms since epoch). */
  at?: number
  /** Listed tables, to offer opening the one the statement changed. */
  tables?: readonly string[]
  onOpenTable?: (table: string) => void
}) {
  const write = outcome.write
  if (!write) return null
  const info = write.info ?? null
  const n = write.affectedRows
  const rowWrite = info?.kind === 'insert' || info?.kind === 'update' || info?.kind === 'delete'
  const schemaWrite = !!info && !rowWrite
  // A row write that matched nothing changed nothing — the one outcome that
  // most needs saying, because "ran" reads as success.
  const nothing = rowWrite && n === 0
  const ms = outcome.serverMs ?? outcome.durationMs
  const time = at ? new Date(at).toLocaleTimeString() : null
  const openable = info?.target && info.kind !== 'drop' ? findTable(info.target, tables ?? []) : undefined
  const offerReturning = rowWrite && !info?.returning && !!driver && RETURNING_DRIVERS.has(driver)

  const saved = nothing
    ? `No rows matched, so nothing was saved to ${db}`
    : `${schemaWrite ? 'Schema change saved' : 'Saved'} to ${db}${time ? ` at ${time}` : ''}`

  const operation = info
    ? `${info.kind.toUpperCase()}${info.object ? ` ${info.object.toUpperCase()}` : ''}`
    : 'Statement'

  return (
    <section className="db-write" aria-label="write result">
      <div className={`db-write-head${nothing ? ' nothing' : ''}`}>
        {nothing ? (
          <CircleDashed size={16} className="db-write-icon" aria-hidden />
        ) : (
          <CircleCheck size={16} className="db-write-icon" aria-hidden />
        )}
        <div className="db-write-titles">
          <p className="title">{writeHeadline(write)}</p>
          <p className="sub">{saved}</p>
        </div>
      </div>

      <dl className="db-write-facts">
        <div>
          <dt>{rowWrite || !info ? 'Rows affected' : 'Object'}</dt>
          <dd>{rowWrite || !info ? n.toLocaleString() : (info.target ?? '—')}</dd>
        </div>
        {info?.kind === 'insert' && write.lastInsertId !== null ? (
          <div>
            <dt>Last insert id</dt>
            <dd>{write.lastInsertId}</dd>
          </div>
        ) : null}
        <div>
          <dt>Operation</dt>
          <dd className="mono">{operation}</dd>
        </div>
        <div>
          <dt>Time</dt>
          <dd>{ms.toLocaleString()} ms</dd>
        </div>
      </dl>

      <div className="db-write-stmt">
        <span className="label">Statement</span>
        <pre>{sql}</pre>
      </div>

      {openable || offerReturning ? (
        <div className="db-write-next">
          {openable && onOpenTable ? (
            <Button variant="secondary" size="sm" onClick={() => onOpenTable(openable)}>
              <Table2 size={16} aria-hidden />
              Open {info?.target}
            </Button>
          ) : null}
          {offerReturning ? (
            <p className="db-write-tip">
              Add <code>returning *</code> to see the {PAST[info?.kind ?? ''] ?? 'changed'} rows here.
            </p>
          ) : null}
        </div>
      ) : null}
    </section>
  )
}

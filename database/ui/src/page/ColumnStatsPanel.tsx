/**
 * What is actually in a column.
 *
 * `source: planner` means the numbers come from statistics the database keeps
 * for the query planner — cheap, and possibly stale or absent. `computed`
 * means real aggregates ran. That distinction is rendered as a chip on every
 * result, because a stale `n_distinct` presented as fact is worse than no
 * number: it reads exactly like a measurement.
 */

import { Badge, Button, type Host, StatusPanel } from '@iii-dev/console-ui'
import { useCallback, useState } from 'react'
import { type ColumnStat, columnStats } from '../lib/rpc'
import { AlertCircle } from './icons'
import { useDatabaseRead } from './useDatabaseRead'

export function ColumnStatsPanel({
  host,
  db,
  table,
  column,
}: {
  host: Host
  db: string
  table: string
  column: string
}) {
  const [exact, setExact] = useState(false)
  const fetcher = useCallback(() => columnStats(host, db, table, [column], exact), [host, db, table, column, exact])
  const read = useDatabaseRead(true, fetcher)

  if (read.error) {
    return (
      <StatusPanel
        variant="alert"
        icon={<AlertCircle size={18} />}
        headline="could not profile this column"
        detail={read.error}
      />
    )
  }
  if (!read.data) {
    return <div className="db-msg db-pulse">· reading statistics…</div>
  }

  const stat: ColumnStat | undefined = read.data.columns[0]
  if (!stat) {
    return <p className="db-msg">no statistics for this column.</p>
  }

  const total = stat.row_count ?? 0
  const nullFrac = stat.null_fraction ?? (total > 0 && stat.null_count != null ? stat.null_count / total : null)
  const topMax = Math.max(...stat.top_values.map((v) => v.count), 1)

  return (
    <div className="db-stats">
      <div className="db-stats-head">
        <span className="db-stats-col">{stat.name}</span>
        {stat.source === 'planner' ? (
          <Badge variant="default" title="from statistics the database keeps for the planner; may be stale">
            estimated · from planner statistics
          </Badge>
        ) : (
          <Badge variant="accent">measured</Badge>
        )}
      </div>

      {stat.source === 'planner' ? (
        <Button variant="ghost" size="sm" onClick={() => setExact(true)} disabled={read.loading}>
          count exactly
        </Button>
      ) : null}

      <dl className="db-kv">
        <Row label="rows" value={fmt(stat.row_count)} />
        <Row label="distinct" value={fmt(stat.distinct_count)} />
        <Row label="nulls" value={fmt(stat.null_count)} />
        <Row label="min" value={render(stat.min)} />
        <Row label="max" value={render(stat.max)} />
        <Row label="mean" value={stat.mean == null ? '—' : stat.mean.toFixed(3)} />
      </dl>

      {nullFrac != null ? (
        <div className="db-bar-row">
          <span className="db-bar-label">null share</span>
          <span className="db-track">
            <span className="db-track-fill" style={{ width: `${nullFrac * 100}%` }} />
          </span>
          <span className="db-num">{(nullFrac * 100).toFixed(1)}%</span>
        </div>
      ) : null}

      {stat.top_values.length > 0 ? (
        <>
          <h4 className="db-stats-sub">most common</h4>
          <ul className="db-stats-top">
            {stat.top_values.map((v, i) => (
              <li key={`${i}-${String(v.value)}`}>
                <span className="db-stats-val" title={render(v.value)}>
                  {render(v.value)}
                </span>
                <span className="db-track">
                  <span className="db-track-fill" style={{ width: `${(v.count / topMax) * 100}%` }} />
                </span>
                <span className="db-num">{v.count.toLocaleString()}</span>
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </div>
  )
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <>
      <dt>{label}</dt>
      <dd className="db-num">{value}</dd>
    </>
  )
}

function fmt(n: number | null | undefined): string {
  return n == null ? '—' : n.toLocaleString()
}

/** Null and empty string must not look alike here either. */
function render(v: unknown): string {
  if (v == null) return 'NULL'
  if (v === '') return "''"
  if (typeof v === 'object') return JSON.stringify(v)
  return String(v)
}

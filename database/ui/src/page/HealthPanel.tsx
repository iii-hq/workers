/**
 * Connection health.
 *
 * Every section is a `ProbeResult`, and rendering that honestly is the entire
 * point of the panel. "sqlite has no equivalent of pg_stat_activity",
 * "permission denied on pg_stat_activity" and "no queries are running" are
 * three different facts, and a panel that shows an empty list for all three
 * teaches you to distrust it.
 */

import { Button, type Host, Select, StatusPanel } from '@iii-dev/console-ui'
import { useCallback, useEffect, useState } from 'react'
import { type HealthReport, health } from '../lib/rpc'
import { AlertCircle, RefreshCw } from './icons'
import { useDatabaseRead } from './useDatabaseRead'

type Probe<T> =
  | { status: 'available'; data: T }
  | { status: 'unsupported'; reason: string }
  | { status: 'denied'; reason: string }

const INTERVALS = [
  { value: '0', label: 'manual' },
  { value: '5', label: 'every 5s' },
  { value: '15', label: 'every 15s' },
  { value: '60', label: 'every 60s' },
]

export function HealthPanel({ host, db }: { host: Host; db: string }) {
  const [nonce, setNonce] = useState(0)
  const [every, setEvery] = useState('0')
  const fetcher = useCallback(() => {
    void nonce
    return health(host, db)
  }, [host, db, nonce])
  const read = useDatabaseRead(true, fetcher)

  const refresh = useCallback(() => setNonce((n) => n + 1), [])

  useEffect(() => {
    const seconds = Number(every)
    if (!seconds) return
    const id = setInterval(refresh, seconds * 1000)
    return () => clearInterval(id)
  }, [every, refresh])

  if (read.error) {
    return (
      <StatusPanel
        variant="alert"
        icon={<AlertCircle size={18} />}
        headline="could not read connection health"
        detail={read.error}
      />
    )
  }
  if (!read.data) {
    return <div className="db-msg db-pulse">· probing the connection…</div>
  }

  const h: HealthReport = read.data
  const pool = h.pool
  const inUse = pool.size != null && pool.idle != null ? pool.size - pool.idle : null

  return (
    <div className="db-health">
      <div className="db-health-bar">
        <span className="db-health-driver">{h.driver}</span>
        <span className="db-health-ver">worker {h.worker_version}</span>
        <div className="db-erd-spacer" />
        <Select value={every} onChange={setEvery} options={INTERVALS} aria-label="refresh interval" />
        <Button variant="ghost" size="sm" onClick={refresh} disabled={read.loading}>
          <RefreshCw size={13} aria-hidden />
          refresh
        </Button>
      </div>

      <div className="db-health-cards">
        <Card title="pool">
          <dl className="db-kv">
            <Stat label="max" value={pool.max} />
            <Stat label="open" value={pool.size} />
            <Stat label="idle" value={pool.idle} />
            <Stat label="in use" value={inUse} />
            <Stat label="waiting" value={pool.waiting} />
          </dl>
          {pool.waiting != null && pool.waiting > 0 ? (
            <p className="db-health-note warn">
              {pool.waiting} caller{pool.waiting === 1 ? '' : 's'} waiting for a connection.
            </p>
          ) : null}
        </Card>

        <Card title="active queries">
          <ProbeBody
            probe={h.active_queries as Probe<HealthQuery[]>}
            empty="no queries are running."
            render={(rows: HealthQuery[]) => (
              <ul className="db-health-list">
                {rows.map((q) => (
                  <li key={q.id} className="db-health-query">
                    <div className="db-health-query-head">
                      <span className="db-health-id">{q.id}</span>
                      {q.state ? <span className="db-health-state">{q.state}</span> : null}
                      {q.duration_ms != null ? <span className="db-num">{formatMs(q.duration_ms)}</span> : null}
                      {q.user ? <span className="db-health-user">{q.user}</span> : null}
                    </div>
                    <code className="db-health-sql" title={q.sql}>
                      {q.sql}
                    </code>
                  </li>
                ))}
              </ul>
            )}
          />
        </Card>

        <Card title="locks">
          <ProbeBody
            probe={h.locks as never}
            empty="nothing is blocked."
            render={(rows: HealthLock[]) => (
              <ul className="db-health-list">
                {rows.map((l) => (
                  <li key={`${l.blocked_id}-${l.blocking_id}`} className="db-health-lock">
                    <div className="db-health-query-head">
                      <span className="db-health-id">{l.blocked_id}</span>
                      <span className="db-health-blocked">blocked by</span>
                      <span className="db-health-id">{l.blocking_id}</span>
                      {l.relation ? <span className="db-health-rel">{l.relation}</span> : null}
                    </div>
                    <code className="db-health-sql" title={l.blocked_sql}>
                      {l.blocked_sql}
                    </code>
                  </li>
                ))}
              </ul>
            )}
          />
        </Card>

        <Card title="cache">
          <ProbeBody
            probe={h.cache as never}
            empty="no cache statistics."
            render={(c: HealthCache) => (
              <>
                <Bar label="hit ratio" ratio={c.hit_ratio} />
                <dl className="db-kv">
                  <Stat label="blocks hit" value={c.blocks_hit} />
                  <Stat label="blocks read" value={c.blocks_read} />
                </dl>
              </>
            )}
          />
        </Card>

        <Card title="table sizes" wide>
          <ProbeBody
            probe={h.table_sizes as never}
            empty="no tables."
            render={(rows: HealthTableSize[]) => {
              const largest = Math.max(...rows.map((r) => r.total_bytes ?? 0), 1)
              return (
                <table className="db-health-table">
                  <thead>
                    <tr>
                      <th>table</th>
                      <th className="num">rows</th>
                      <th className="num">indexes</th>
                      <th className="num">total</th>
                      <th className="share">share</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((r) => (
                      <tr key={`${r.schema ?? ''}.${r.table}`}>
                        <td>{r.table}</td>
                        <td className="num">{formatCount(r.row_estimate)}</td>
                        <td className="num">{formatBytes(r.index_bytes)}</td>
                        <td className="num">{formatBytes(r.total_bytes)}</td>
                        <td className="share">
                          <span className="db-track">
                            <span
                              className="db-track-fill"
                              style={{ width: `${((r.total_bytes ?? 0) / largest) * 100}%` }}
                            />
                          </span>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )
            }}
          />
        </Card>
      </div>
    </div>
  )
}

type HealthQuery = {
  id: string
  sql: string
  state?: string | null
  duration_ms?: number | null
  user?: string | null
}
type HealthLock = { blocked_id: string; blocked_sql: string; blocking_id: string; relation?: string | null }
type HealthCache = { hit_ratio: number; blocks_hit: number; blocks_read: number }
type HealthTableSize = {
  table: string
  schema?: string | null
  total_bytes?: number | null
  index_bytes?: number | null
  row_estimate?: number | null
}

function Card({ title, wide, children }: { title: string; wide?: boolean; children: React.ReactNode }) {
  return (
    <section className={`db-health-card${wide ? ' wide' : ''}`}>
      <h3>{title}</h3>
      {children}
    </section>
  )
}

/**
 * The three-state renderer. An empty `available` result gets its own sentence
 * — it is a real answer, and must not look like the absence of one.
 */
function ProbeBody<T>({
  probe,
  empty,
  render,
}: {
  probe: Probe<T>
  empty: string
  render: (data: T) => React.ReactNode
}) {
  if (probe.status === 'unsupported') {
    return <p className="db-health-note quiet">{probe.reason}</p>
  }
  if (probe.status === 'denied') {
    return <p className="db-health-note warn">{probe.reason}</p>
  }
  if (Array.isArray(probe.data) && probe.data.length === 0) {
    return <p className="db-health-note">{empty}</p>
  }
  return <>{render(probe.data)}</>
}

function Stat({ label, value }: { label: string; value: number | null | undefined }) {
  return (
    <>
      <dt>{label}</dt>
      <dd className="db-num">{value == null ? '—' : value.toLocaleString()}</dd>
    </>
  )
}

function Bar({ label, ratio }: { label: string; ratio: number }) {
  const pct = Math.max(0, Math.min(1, ratio))
  return (
    <div className="db-bar-row">
      <span className="db-bar-label">{label}</span>
      <span className="db-track">
        <span className="db-track-fill" style={{ width: `${pct * 100}%` }} />
      </span>
      <span className="db-num">{(pct * 100).toFixed(1)}%</span>
    </div>
  )
}

function formatBytes(n: number | null | undefined): string {
  if (n == null) return '—'
  const units = ['b', 'kb', 'mb', 'gb', 'tb']
  let v = n
  let u = 0
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024
    u += 1
  }
  return `${u === 0 ? v : v.toFixed(1)}${units[u]}`
}

function formatCount(n: number | null | undefined): string {
  return n == null ? '—' : n.toLocaleString()
}

function formatMs(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(1)}s`
}

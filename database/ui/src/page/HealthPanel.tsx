/**
 * Connection health.
 *
 * Every section is a `ProbeResult`, and rendering that honestly is the entire
 * point of the panel. "sqlite has no equivalent of pg_stat_activity",
 * "permission denied on pg_stat_activity" and "no queries are running" are
 * three different facts, and a panel that shows an empty list for all three
 * teaches you to distrust it.
 */

import {
  Badge,
  Button,
  type Host,
  MetaRow,
  Panel,
  PanelBody,
  PanelHeader,
  Select,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  Toolbar,
  uiClasses,
} from '@iii-dev/console-ui'
import { formatBytes, formatDuration } from '@iii-dev/console-ui/format'
import { CircleAlert, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { type HealthReport, health } from '../lib/rpc'
import { driverLabel } from './db-data'
import { useDatabaseRead } from './useDatabaseRead'

type Probe<T> =
  | { status: 'available'; data: T }
  | { status: 'unsupported'; reason: string }
  | { status: 'denied'; reason: string }

const INTERVALS = [
  { value: '0', label: 'Manual' },
  { value: '5', label: 'Every 5s' },
  { value: '15', label: 'Every 15s' },
  { value: '60', label: 'Every 60s' },
]

function intervalKey(db: string): string {
  return `iii-console:database:health-interval:${db}`
}

export function HealthPanel({
  host,
  db,
  refreshToken,
}: {
  host: Host
  db: string
  /** Bumped by the page's refresh — the header button must reach this tab
      too, not silently no-op on it. */
  refreshToken?: number
}) {
  const [nonce, setNonce] = useState(0)
  // The cadence you picked survives leaving the tab (the panel itself
  // remounts by design — it is a fresh-read view).
  const [every, setEveryState] = useState(() => {
    try {
      const stored = window.localStorage.getItem(intervalKey(db))
      return stored && INTERVALS.some((i) => i.value === stored) ? stored : '0'
    } catch {
      return '0'
    }
  })
  const setEvery = (next: string) => {
    setEveryState(next)
    try {
      window.localStorage.setItem(intervalKey(db), next)
    } catch {
      // the cadence is a convenience, not state
    }
  }
  const fetcher = useCallback(() => {
    void nonce
    void refreshToken
    return health(host, db)
  }, [host, db, nonce, refreshToken])
  const read = useDatabaseRead(true, fetcher)
  // When the report was read, so a stale card never poses as current.
  const [asOf, setAsOf] = useState<Date | null>(null)
  useEffect(() => {
    if (read.data) setAsOf(new Date())
  }, [read.data])

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
        icon={<CircleAlert size={18} />}
        headline="Could not read connection health"
        detail={read.error}
      />
    )
  }
  if (!read.data) {
    return (
      <div className={`db-msg ${uiClasses.pulse}`}>Probing the connection…</div>
    )
  }

  const h: HealthReport = read.data
  const pool = h.pool
  const inUse =
    pool.size != null && pool.idle != null ? pool.size - pool.idle : null

  return (
    <div className="db-health">
      <Toolbar
        aria-label="connection health"
        className="db-bar"
        end={
          <>
            <Select
              value={every}
              onChange={setEvery}
              options={INTERVALS}
              aria-label="refresh interval"
            />
            <Button
              variant="ghost"
              size="sm"
              onClick={refresh}
              disabled={read.loading}
            >
              <RefreshCw size={16} aria-hidden />
              Refresh
            </Button>
          </>
        }
      >
        <span className="db-health-driver">{driverLabel(h.driver)}</span>
        <span className="db-health-ver">Worker {h.worker_version}</span>
        {asOf ? (
          <span className="db-health-asof">
            As of{' '}
            {asOf.toLocaleTimeString([], {
              hour: '2-digit',
              minute: '2-digit',
              second: '2-digit',
            })}
          </span>
        ) : null}
      </Toolbar>

      <div className={`db-health-cards${read.loading ? ' stale' : ''}`}>
        <Card title="Pool">
          <MetaRow
            items={[
              { label: 'Max', value: formatCount(pool.max) },
              { label: 'Open', value: formatCount(pool.size) },
              { label: 'Idle', value: formatCount(pool.idle) },
              { label: 'In use', value: formatCount(inUse) },
              { label: 'Waiting', value: formatCount(pool.waiting) },
            ]}
          />
          {pool.waiting != null && pool.waiting > 0 ? (
            <p className="db-health-note warn">
              {pool.waiting} caller{pool.waiting === 1 ? '' : 's'} waiting for a
              connection.
            </p>
          ) : null}
        </Card>

        <Card title="Active queries">
          <ProbeBody
            probe={h.active_queries as Probe<HealthQuery[]>}
            empty="No queries are running."
            render={(rows: HealthQuery[]) => {
              // MySQL's processlist includes replication daemons that sit
              // "running" for the server's whole uptime. Billing them as
              // active queries reads as an alarm — show them (honest), but
              // name them and sort them after the real work.
              const daemons = rows.filter(isReplication)
              const real = rows.filter((q) => !isReplication(q))
              return (
                <ul className="db-health-list">
                  {[...real, ...daemons].map((q) => (
                    <li
                      key={q.id}
                      className={`db-health-query${isReplication(q) ? ' daemon' : ''}`}
                    >
                      <div className="db-health-query-head">
                        <span className="db-health-id">{q.id}</span>
                        {isReplication(q) ? <Badge>Replication</Badge> : null}
                        {q.state ? (
                          <span className="db-health-state">{q.state}</span>
                        ) : null}
                        {q.duration_ms != null ? (
                          <span className="db-num">
                            {formatDuration(q.duration_ms)}
                          </span>
                        ) : null}
                        {q.user ? (
                          <span className="db-health-user">{q.user}</span>
                        ) : null}
                      </div>
                      <code className="db-health-sql" title={q.sql}>
                        {q.sql}
                      </code>
                    </li>
                  ))}
                </ul>
              )
            }}
          />
        </Card>

        <Card title="Locks">
          <ProbeBody
            probe={h.locks as never}
            empty="Nothing is blocked."
            render={(rows: HealthLock[]) => (
              <ul className="db-health-list">
                {rows.map((l) => (
                  <li key={`${l.blocked_id}-${l.blocking_id}`}>
                    <div className="db-health-query-head">
                      <span className="db-health-id">{l.blocked_id}</span>
                      <span className="db-health-blocked">Blocked by</span>
                      <span className="db-health-id">{l.blocking_id}</span>
                      {l.relation ? (
                        <span className="db-health-rel">{l.relation}</span>
                      ) : null}
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

        <Card title="Cache">
          <ProbeBody
            probe={h.cache as never}
            empty="No cache statistics."
            render={(c: HealthCache) => (
              <>
                <Bar label="Hit ratio" ratio={c.hit_ratio} />
                <MetaRow
                  items={[
                    { label: 'Blocks hit', value: formatCount(c.blocks_hit) },
                    { label: 'Blocks read', value: formatCount(c.blocks_read) },
                  ]}
                />
              </>
            )}
          />
        </Card>

        <Card title="Table sizes" wide>
          <ProbeBody
            probe={h.table_sizes as never}
            empty="No tables."
            render={(rows: HealthTableSize[]) => {
              const largest = Math.max(
                ...rows.map((r) => r.total_bytes ?? 0),
                1,
              )
              return (
                <TableViewport>
                  <TableFrame>
                    <Table density="compact" className="db-health-table">
                      <TableHeader>
                        <TableRow>
                          <TableHead>Table</TableHead>
                          <TableHead className="num">Rows</TableHead>
                          <TableHead className="num">Indexes</TableHead>
                          <TableHead className="num">Total</TableHead>
                          <TableHead className="share">Share</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {rows.map((r) => (
                          <TableRow key={`${r.schema ?? ''}.${r.table}`}>
                            <TableCell>{r.table}</TableCell>
                            <TableCell className="num">
                              {formatCount(r.row_estimate)}
                            </TableCell>
                            <TableCell className="num">
                              {formatBytes(r.index_bytes)}
                            </TableCell>
                            <TableCell className="num">
                              {formatBytes(r.total_bytes)}
                            </TableCell>
                            <TableCell className="share">
                              <span className="db-track">
                                <span
                                  className="db-track-fill"
                                  style={{
                                    width: `${((r.total_bytes ?? 0) / largest) * 100}%`,
                                  }}
                                />
                              </span>
                            </TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </TableFrame>
                </TableViewport>
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

/** A replication/binlog daemon connection, not a query anyone ran. */
function isReplication(q: HealthQuery): boolean {
  return (
    /binlog|replic/i.test(q.state ?? '') || /binlog dump/i.test(q.sql ?? '')
  )
}
type HealthLock = {
  blocked_id: string
  blocked_sql: string
  blocking_id: string
  relation?: string | null
}
type HealthCache = {
  hit_ratio: number
  blocks_hit: number
  blocks_read: number
}
type HealthTableSize = {
  table: string
  schema?: string | null
  total_bytes?: number | null
  index_bytes?: number | null
  row_estimate?: number | null
}

function Card({
  title,
  wide,
  children,
}: {
  title: string
  wide?: boolean
  children: React.ReactNode
}) {
  return (
    <Panel className={`db-health-card${wide ? ' wide' : ''}`}>
      <PanelHeader>{title}</PanelHeader>
      <PanelBody>{children}</PanelBody>
    </Panel>
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

function formatCount(n: number | null | undefined): string {
  return n == null ? '—' : n.toLocaleString()
}

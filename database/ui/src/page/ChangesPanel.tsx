/**
 * What has changed in the selected table, as it happens.
 *
 * The worker has emitted `database::row-changed` for a while and #640 gave it
 * real cross-client capture, but nothing ever rendered it — so a write from
 * another client was observable to agents and invisible to the person looking
 * at the table. This is that view.
 *
 * The panel is deliberate about what it promises. It never uses the words
 * "live" or "realtime"; it names the guarantee in force, because the two
 * capture modes differ in a way that matters and only one of them sees other
 * clients' writes.
 */

import { Badge, Button, EmptyState, type Host, StatusDot } from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { History, RefreshCw } from './icons'
import { type RowChange, useRowChanges } from './useRowChanges'

const HistoryIcon = (p: { className?: string }) => <History size={28} {...p} />

/** How long a row keeps its arrival highlight. */
const FRESH_MS = 4000

const OP_LABEL: Record<string, string> = {
  insert: 'insert',
  update: 'update',
  delete: 'delete',
  other: 'other',
}

/**
 * The capture-mode caveat, shown on demand. It used to live only in a title
 * attribute the idle copy told you to hover — unreachable by keyboard, touch,
 * or a screen reader. The badge is a real disclosure button now; the tooltip
 * stays for mouse users.
 */
const CAPTURE_NOTE =
  'following committed writes to this table. a connection using statement capture reports only writes made through this worker; one using native capture reports every client. delivery is best effort — reload to be certain.'

export function ChangesPanel({
  host,
  db,
  table,
  kind,
  onRefresh,
}: {
  host: Host
  db: string
  table: string | null
  /** `view` cannot be followed — see below. */
  kind?: string
  onRefresh?: () => void
}) {
  const [capOpen, setCapOpen] = useState(false)
  // A view is never the target of a write. The worker keys every change on the
  // table the statement actually touched, so a binding on a view matches
  // nothing and would sit at "following" forever while rows visibly change
  // underneath it. Refuse the binding and say why.
  const isView = kind === 'view'
  const feed = useRowChanges(host, db, isView ? null : table)
  // Re-render on a slow cadence so relative stamps and the fade advance
  // without an interval per row.
  const [, tick] = useState(0)
  useEffect(() => {
    if (feed.changes.length === 0) return
    const id = setInterval(() => tick((n) => n + 1), 1000)
    return () => clearInterval(id)
  }, [feed.changes.length])

  if (!table) {
    return (
      <EmptyState
        icon={HistoryIcon}
        title="no table selected"
        description="pick a table in the tree to follow the writes landing in it."
      />
    )
  }

  if (isView) {
    return (
      <EmptyState
        icon={HistoryIcon}
        title={`${table} is a view`}
        description="changes are reported against the table a statement writes to, so a view never reports any of its own. select one of the tables it reads from to follow the writes behind it."
      />
    )
  }

  if (feed.status === 'unsupported') {
    return (
      <EmptyState
        icon={HistoryIcon}
        title="this worker does not announce row changes"
        description="run `iii worker update database` to follow writes as they commit."
      />
    )
  }

  return (
    <div className="db-changes">
      <div className="db-changes-bar db-toolbar">
        <FreshnessBadge
          status={feed.status}
          lastAt={feed.lastAt}
          expanded={capOpen}
          onToggle={() => setCapOpen((v) => !v)}
        />
        <span className="db-changes-target">{table}</span>
        <div className="db-toolbar-spacer" />
        {feed.pending > 0 && onRefresh ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              feed.acknowledge()
              onRefresh()
            }}
          >
            <RefreshCw size={13} aria-hidden />
            reload rows · {feed.pending}
          </Button>
        ) : null}
      </div>

      {capOpen ? <p className="db-changes-capnote">{CAPTURE_NOTE}</p> : null}

      {feed.changes.length === 0 ? (
        <EmptyState
          icon={HistoryIcon}
          title="nothing yet"
          description={`writes to ${table} appear here as they commit. what counts as a write depends on this connection's capture mode — the "following" indicator above explains it.`}
        />
      ) : (
        // role="log": arrivals are announced politely without stealing focus —
        // the whole point of a feed whose job is telling you about writes.
        <ol className="db-changes-list" role="log" aria-label={`writes to ${table}`}>
          {feed.changes.map((c) => (
            <ChangeRow key={c.seq} change={c} />
          ))}
        </ol>
      )}
    </div>
  )
}

function ChangeRow({ change }: { change: RowChange }) {
  const age = Date.now() - change.seen
  const fresh = age < FRESH_MS
  const rows = change.affected_rows
  return (
    <li className={`db-change${fresh ? ' fresh' : ''}`}>
      <span className={`db-change-op op-${change.op}`}>{OP_LABEL[change.op] ?? change.op}</span>
      <span className="db-change-rows">
        {rows} {rows === 1 ? 'row' : 'rows'}
      </span>
      {change.returning?.length ? <Badge variant="default">{change.returning.length} returned</Badge> : null}
      <span className="db-change-at" title={new Date(change.at).toISOString()}>
        {relative(age)}
      </span>
    </li>
  )
}

/**
 * Which guarantee is in force.
 *
 * The mode is a property of the connection and is not exposed as a read, so
 * this reports what it can actually stand behind: that a binding is attached,
 * and when something last arrived. It never claims completeness — the tooltip
 * carries the caveat rather than a word in the badge implying more than is
 * true.
 */
function FreshnessBadge({
  status,
  lastAt,
  expanded,
  onToggle,
}: {
  status: string
  lastAt: number | null
  expanded: boolean
  onToggle: () => void
}) {
  const bound = status === 'bound'
  const recent = lastAt != null && Date.now() - lastAt < 10_000
  return (
    <button
      type="button"
      className="db-freshness"
      onClick={onToggle}
      aria-expanded={expanded}
      title={bound ? CAPTURE_NOTE : 'not following this table.'}
    >
      <StatusDot tone={bound ? 'accent' : 'ink'} pulse={bound && recent} />
      <span>{bound ? 'following' : 'not following'}</span>
      {lastAt != null ? <span className="db-freshness-at">· last {relative(Date.now() - lastAt)}</span> : null}
    </button>
  )
}

function relative(ms: number): string {
  if (ms < 1000) return 'just now'
  const s = Math.floor(ms / 1000)
  if (s < 60) return `${s}s ago`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m ago`
  const h = Math.floor(m / 60)
  return `${h}h ago`
}

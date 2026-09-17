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

import {
  Badge,
  Button,
  EmptyState,
  type Host,
  StatusDot,
  Toolbar,
} from '@iii-dev/console-ui'
import { formatRelative } from '@iii-dev/console-ui/format'
import { History, RefreshCw } from 'lucide-react'
import { useEffect, useState } from 'react'
import { type RowChange, useRowChanges } from './useRowChanges'

/** How long a row keeps its arrival highlight. */
const FRESH_MS = 4000

const OP_LABEL: Record<string, string> = {
  insert: 'Insert',
  update: 'Update',
  delete: 'Delete',
  other: 'Other',
}

/** Insert and delete carry their status tone; update stays neutral. */
const OP_VARIANT: Record<string, 'ok' | 'alert'> = {
  insert: 'ok',
  delete: 'alert',
}

/**
 * The capture-mode caveat, shown on demand. It used to live only in a title
 * attribute the idle copy told you to hover — unreachable by keyboard, touch,
 * or a screen reader. The badge is a real disclosure button now; the tooltip
 * stays for mouse users.
 */
const CAPTURE_NOTE =
  'Following committed writes to this table. A connection using statement capture reports only writes made through this worker; one using native capture reports every client. Delivery is best effort — reload to be certain.'

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
        icon={History}
        title="No table selected"
        description="Pick a table in the tree to follow the writes landing in it."
      />
    )
  }

  if (isView) {
    return (
      <EmptyState
        icon={History}
        title={`${table} is a view`}
        description="Changes are reported against the table a statement writes to, so a view never reports any of its own. Select one of the tables it reads from to follow the writes behind it."
      />
    )
  }

  if (feed.status === 'unsupported') {
    return (
      <EmptyState
        icon={History}
        title="This worker does not announce row changes"
        description="Run `iii worker update database` to follow writes as they commit."
      />
    )
  }

  return (
    <div className="db-changes">
      <Toolbar
        aria-label="row changes"
        end={
          feed.pending > 0 && onRefresh ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                feed.acknowledge()
                onRefresh()
              }}
            >
              <RefreshCw size={16} aria-hidden />
              Reload rows · {feed.pending}
            </Button>
          ) : undefined
        }
      >
        <FreshnessBadge
          status={feed.status}
          lastAt={feed.lastAt}
          expanded={capOpen}
          onToggle={() => setCapOpen((v) => !v)}
        />
        <span className="db-changes-target">{table}</span>
      </Toolbar>

      {capOpen ? <p className="db-changes-capnote">{CAPTURE_NOTE}</p> : null}

      {feed.changes.length === 0 ? (
        <EmptyState
          icon={History}
          title="Nothing yet"
          description={`Writes to ${table} appear here as they commit. What counts as a write depends on this connection's capture mode — the "Following" indicator above explains it.`}
        />
      ) : (
        // role="log": arrivals are announced politely without stealing focus —
        // the whole point of a feed whose job is telling you about writes.
        <ol
          className="db-changes-list"
          role="log"
          aria-label={`writes to ${table}`}
        >
          {feed.changes.map((c) => (
            <ChangeRow key={c.seq} change={c} />
          ))}
        </ol>
      )}
    </div>
  )
}

function ChangeRow({ change }: { change: RowChange }) {
  const fresh = Date.now() - change.seen < FRESH_MS
  const rows = change.affected_rows
  return (
    <li className={`db-change${fresh ? ' fresh' : ''}`}>
      <Badge
        variant={OP_VARIANT[change.op] ?? 'default'}
        className="db-change-op"
      >
        {OP_LABEL[change.op] ?? change.op}
      </Badge>
      <span className="db-change-rows">
        {rows} {rows === 1 ? 'row' : 'rows'}
      </span>
      {change.returning?.length ? (
        <Badge variant="default">
          {change.returning.length}
          {change.truncated ? '+' : ''} returned
        </Badge>
      ) : null}
      <span className="db-change-at" title={new Date(change.at).toISOString()}>
        {formatRelative(change.seen)}
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
      <span>{bound ? 'Following' : 'Not following'}</span>
      {lastAt != null ? (
        <span className="db-freshness-at">· Last {formatRelative(lastAt)}</span>
      ) : null}
    </button>
  )
}

import { Card, CardBody, CardHeader, EmptyState, Skeleton } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useState } from 'react'
import type { Client, GroupSummary, GroupTransition } from '../api'
import { Dot } from './marks'
import { ago, stamp, transitionLook } from './present.js'

/**
 * How the group got where it is, newest first.
 *
 * Every row was written inside the transaction that made the move, so this
 * cannot disagree with the status above it. A group older than the history
 * table still has its birth: the first-seen fields say when, and the row is
 * drawn from them.
 */
export function GroupTimeline({ api, group, now }: { api: Client; group: GroupSummary; now: number }) {
  const [rows, setRows] = useState<GroupTransition[] | null>(null)
  const [total, setTotal] = useState(0)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let live = true
    setRows(null)
    api
      .history(group.id)
      .then((response) => {
        if (!live) return
        setRows(response.transitions)
        setTotal(response.total)
      })
      .catch((cause) => live && setError(errorMessage(cause)))
    return () => {
      live = false
    }
  }, [api, group.id, group.status])

  if (error) return <EmptyState compact title="Could not read the history" description={error} />
  if (!rows) return <Skeleton />

  const born = rows.some((row) => !row.from_status)
  const entries = [
    ...rows.map((row) => ({ key: row.id, at_ms: row.at_ms, ...transitionLook(row) })),
    ...(born || total > rows.length
      ? []
      : [
          {
            key: 'first-seen',
            at_ms: group.first_seen_ms,
            what: 'First seen',
            note: [group.service_name, group.first_version].filter(Boolean).join(' '),
            dot: 'ghost',
          },
        ]),
  ]

  return (
    <div className="sentinel-ui-stack">
      <Card>
        <CardHeader>
          <span>History</span>
          <span className="sentinel-ui-card-note">state transitions, newest first</span>
        </CardHeader>
        <CardBody>
          <ol className="sentinel-ui-history">
            {entries.map((entry) => (
              <li key={entry.key}>
                <Dot tone={entry.dot} />
                <span className="sentinel-ui-quiet-mono sentinel-ui-history-when" title={stamp(entry.at_ms)}>
                  {ago(entry.at_ms, now)}
                </span>
                <b>{entry.what}</b>
                {entry.note ? <span className="sentinel-ui-quiet">{entry.note}</span> : null}
              </li>
            ))}
          </ol>
          {total > rows.length ? (
            <p className="sentinel-ui-card-foot">
              showing the latest {rows.length} of {total}
            </p>
          ) : null}
        </CardBody>
      </Card>
      <p className="sentinel-ui-foot-note">
        Every transition is a row Sentinel wrote — the fingerprint is stable, so a group survives a rename
        of the message it started from.
      </p>
    </div>
  )
}

import { Chip, EmptyState, Eyebrow, Skeleton } from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import { useEffect, useState } from 'react'
import type { Client, GroupTransition } from '../api'
import { transitionSentence } from './present.js'

/**
 * How the group got where it is.
 *
 * Every row was written inside the transaction that made the move, so this
 * cannot disagree with the status above it. What it adds over those fields is
 * the part they cannot hold: the order, and who decided — the pipeline, the
 * agent, or a person.
 */
export function GroupTimeline({ api, groupId }: { api: Client; groupId: string }) {
  const [rows, setRows] = useState<GroupTransition[] | null>(null)
  const [total, setTotal] = useState(0)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let live = true
    setRows(null)
    api
      .history(groupId)
      .then((response) => {
        if (!live) return
        setRows(response.transitions)
        setTotal(response.total)
      })
      .catch((cause) => live && setError(errorMessage(cause)))
    return () => {
      live = false
    }
  }, [api, groupId])

  if (error) return <EmptyState compact title="Could not read the history" description={error} />
  if (!rows) return <Skeleton />
  if (rows.length === 0) {
    return (
      <EmptyState
        compact
        title="No moves recorded"
        description="This group predates the history, or it has not changed state since it was created."
      />
    )
  }

  return (
    <div className="sentinel-ui-timeline">
      <ol>
        {rows.map((row) => (
          <li key={row.id} data-actor={row.actor}>
            <span className="sentinel-ui-timeline-when">{formatRelative(row.at_ms)}</span>
            <span className="sentinel-ui-timeline-move">{transitionSentence(row)}</span>
            {/* Neutral on purpose: the actor is who, not how bad. Colouring
                it by the state it moved to made every reopen look like an
                error. */}
            <Chip tone="neutral">{row.actor}</Chip>
          </li>
        ))}
      </ol>
      {total > rows.length ? (
        <Eyebrow className="sentinel-ui-more">
          showing {rows.length} of {total}
        </Eyebrow>
      ) : null}
    </div>
  )
}

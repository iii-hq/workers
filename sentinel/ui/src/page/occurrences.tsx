import {
  Button,
  Card,
  CardBody,
  CardHeader,
  Chip,
  EmptyState,
  Skeleton,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import type { Host } from '@iii-dev/console-ui'
import { ChevronRight, MessageSquare } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { Client, OccurrenceSummary } from '../api'
import { ago, spaced, stamp } from './present.js'

/** Every recorded instance. The row count is the incident's size and never
    shrinks; what retention removes is the bundle behind a row. */
export function OccurrencesTable({
  api,
  groupId,
  host,
  now,
  total,
  onOpen,
}: {
  api: Client
  groupId: string
  host: Host
  now: number
  total: number
  onOpen: (occurrence: OccurrenceSummary) => void
}) {
  const [rows, setRows] = useState<OccurrenceSummary[] | null>(null)
  const [kept, setKept] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)

  // A new group starts empty; a new occurrence of the same group re-reads
  // in place, rows and scroll untouched.
  useEffect(() => setRows(null), [groupId])
  useEffect(() => {
    let live = true
    setError(null)
    api
      .occurrences(groupId)
      .then((response) => {
        if (!live) return
        setRows(response.occurrences)
        setKept(response.total)
      })
      .catch((cause) => live && setError(errorMessage(cause)))
    return () => {
      live = false
    }
  }, [api, groupId, total, attempt])

  if (error) {
    return (
      <StatusPanel
        variant="alert"
        headline="Could not read the occurrences"
        detail={error}
        action={
          <Button size="sm" onClick={() => setAttempt((previous) => previous + 1)}>
            Retry
          </Button>
        }
      />
    )
  }
  if (!rows) return <Skeleton />
  if (rows.length === 0) {
    return <EmptyState compact title="No occurrences kept" description="Retention removed the rows; the counters stay." />
  }

  const snapshots = rows.filter((row) => row.has_evidence).length
  return (
    <div className="sentinel-ui-stack">
      <Card>
        <CardHeader>
          <span>Occurrences</span>
          <span className="sentinel-ui-card-note">
            {spaced(kept)} of {spaced(total)} rows kept · {snapshots} of the {rows.length} shown carry a full
            evidence snapshot
          </span>
        </CardHeader>
        <CardBody>
          <TableViewport>
            <TableFrame>
              <Table density="compact" className="sentinel-ui-occurrences">
                <TableHeader>
                  <TableRow>
                    <TableHead>When</TableHead>
                    <TableHead>Worker version</TableHead>
                    <TableHead className="sentinel-ui-grow">Session</TableHead>
                    <TableHead>Turn</TableHead>
                    <TableHead>Evidence</TableHead>
                    <TableHead className="sentinel-ui-chevron">
                      <span className="sentinel-ui-visually-hidden">Open</span>
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {rows.map((row) => (
                    <TableRow
                      key={row.id}
                      interactive={row.has_evidence}
                      onClick={row.has_evidence ? () => onOpen(row) : undefined}
                    >
                      <TableCell className="sentinel-ui-mono" title={stamp(row.at_ms)}>
                        {ago(row.at_ms, now)}
                      </TableCell>
                      <TableCell className="sentinel-ui-mono">{row.worker_version ?? '—'}</TableCell>
                      <TableCell className="sentinel-ui-grow">
                        <SessionLink host={host} occurrence={row} />
                      </TableCell>
                      <TableCell className="sentinel-ui-mono sentinel-ui-quiet">
                        {row.turn_id ? row.turn_id.slice(0, 10) : '—'}
                      </TableCell>
                      <TableCell>
                        {row.has_evidence ? (
                          <Chip tone={row.settled ? 'neutral' : 'warning'}>{row.settled ? 'snapshot' : 'partial'}</Chip>
                        ) : (
                          <Chip className="sentinel-ui-quiet">pruned</Chip>
                        )}
                      </TableCell>
                      <TableCell className="sentinel-ui-chevron">
                        {row.has_evidence ? <ChevronRight size={16} aria-hidden="true" /> : null}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </TableFrame>
          </TableViewport>
        </CardBody>
      </Card>
      <p className="sentinel-ui-foot-note">
        Older rows keep their counters after the evidence is pruned — the hourly buckets behind the
        sparkline are never dropped.
      </p>
    </div>
  )
}

/**
 * The conversation a failure happened in — the most useful jump on this
 * table: an error with a session is an error somebody was in the middle of.
 * A session the console cannot open, or an occurrence with none, shows the
 * id rather than a button that would do nothing.
 */
function SessionLink({ host, occurrence }: { host: Host; occurrence: OccurrenceSummary }) {
  const sessionId = occurrence.session_id
  if (!sessionId) return <span className="sentinel-ui-quiet">—</span>
  const open = host.chat?.selectConversation
  if (!open) return <span className="sentinel-ui-mono">{sessionId}</span>
  return (
    <Button
      size="sm"
      variant="ghost"
      title="Open the conversation this happened in"
      onClick={(event) => {
        // The button, not the row: the row opens the evidence.
        event.stopPropagation()
        open(sessionId)
      }}
    >
      <MessageSquare size={16} />
      <span className="sentinel-ui-mono">{sessionId}</span>
    </Button>
  )
}

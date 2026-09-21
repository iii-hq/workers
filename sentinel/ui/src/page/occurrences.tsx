import {
  Button,
  Chip,
  EmptyState,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
} from '@iii-dev/console-ui'
import { errorMessage, formatRelative } from '@iii-dev/console-ui/format'
import type { Host } from '@iii-dev/console-ui'
import { MessageSquare } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { Client, OccurrenceSummary } from '../api'

/** Every recorded instance. The row count is the incident's size and never
    shrinks; what retention removes is the bundle behind a row. */
export function OccurrencesTable({
  api,
  groupId,
  host,
}: {
  api: Client
  groupId: string
  host: Host
}) {
  const [rows, setRows] = useState<OccurrenceSummary[] | null>(null)
  const [total, setTotal] = useState(0)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let live = true
    api
      .occurrences(groupId)
      .then((response) => {
        if (!live) return
        setRows(response.occurrences)
        setTotal(response.total)
      })
      .catch((cause) => live && setError(errorMessage(cause)))
    return () => {
      live = false
    }
  }, [api, groupId])

  if (error) return <EmptyState compact title="Could not read the occurrences" description={error} />
  if (!rows) return <Skeleton />
  if (rows.length === 0) {
    return <EmptyState compact title="No occurrences kept" description="Retention removed them." />
  }

  return (
    <TableViewport>
      <TableFrame>
        <Table density="compact">
          <TableHeader>
            <TableRow>
              <TableHead>when</TableHead>
              <TableHead>version</TableHead>
              <TableHead>session</TableHead>
              <TableHead>message</TableHead>
              <TableHead>evidence</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={row.id}>
                <TableCell>{formatRelative(row.at_ms)}</TableCell>
                <TableCell>{row.worker_version ?? '—'}</TableCell>
                <TableCell>
                  <SessionLink host={host} occurrence={row} />
                </TableCell>
                <TableCell>
                  <span className="sentinel-ui-occurrence-message">{row.message}</span>
                </TableCell>
                <TableCell>
                  {row.has_evidence ? (
                    <Chip tone={row.settled ? 'neutral' : 'warning'}>
                      {row.settled ? 'kept' : 'partial'}
                    </Chip>
                  ) : (
                    <Chip tone="neutral">pruned</Chip>
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableFrame>
      {total > rows.length ? (
        <p className="sentinel-ui-more">
          showing {rows.length} of {total}
        </p>
      ) : null}
    </TableViewport>
  )
}

/**
 * The conversation a failure happened in.
 *
 * It is the single most useful jump on this table: an error with a session is
 * an error somebody was in the middle of, and the transcript says what they
 * were doing. A session the console cannot open, or an occurrence with none,
 * shows the id rather than a button that would do nothing.
 */
function SessionLink({ host, occurrence }: { host: Host; occurrence: OccurrenceSummary }) {
  const sessionId = occurrence.session_id
  if (!sessionId) return <span className="sentinel-ui-session-none">—</span>
  const open = host.chat?.selectConversation
  const label = occurrence.turn_id
    ? `${sessionId.slice(0, 10)} · turn ${occurrence.turn_id.slice(0, 6)}`
    : sessionId.slice(0, 10)
  if (!open) return <span className="sentinel-ui-session">{label}</span>
  return (
    <Button
      size="sm"
      variant="ghost"
      title="Open the conversation this happened in"
      onClick={() => open(sessionId)}
    >
      <MessageSquare size={16} />
      <span className="sentinel-ui-session">{label}</span>
    </Button>
  )
}

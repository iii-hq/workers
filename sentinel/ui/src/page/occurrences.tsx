import {
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
import { useEffect, useState } from 'react'
import type { Client, OccurrenceSummary } from '../api'

/** Every recorded instance. The row count is the incident's size and never
    shrinks; what retention removes is the bundle behind a row. */
export function OccurrencesTable({ api, groupId }: { api: Client; groupId: string }) {
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

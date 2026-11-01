import {
  Badge,
  EmptyState,
  StatusBar,
  StatusDot,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@iii-dev/console-ui'
import { formatDuration } from '@iii-dev/console-ui/format'
import { truncateMiddle } from '../lib/format'
import { formatEpochMs, jobDurationMs, jobStatusPill } from './format'
import { safeParseResponse, shellListResponseSchema } from './parsers'

interface ShellListViewProps {
  output: unknown
}

/**
 * `shell::list` — background-job summary table. `JobSummary`
 * deliberately omits argv/stdout/stderr (cross-caller secrecy), so
 * there is no command column; the footer line points at
 * `shell::status` for the full record. `count` is not rendered — it
 * always equals `jobs.length` (the schema keeps it required as a
 * contract canary). Request renders nothing (ignored server-side).
 */
export function ShellListView({ output }: ShellListViewProps) {
  const parsed = safeParseResponse(shellListResponseSchema, output)
  if (!parsed) return null
  const jobs = parsed.jobs

  if (jobs.length === 0) {
    return (
      <div className="shui-card shui-card-body">
        <EmptyState title="No jobs" description="No background jobs for this worker." />
      </div>
    )
  }

  return (
    <div className="shui-card">
      <Table density="compact">
        <TableHeader>
          <TableRow>
            <TableHead>job</TableHead>
            <TableHead>status</TableHead>
            <TableHead>started</TableHead>
            <TableHead>duration</TableHead>
            <TableHead>exit</TableHead>
            <TableHead>output</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {jobs.map((j) => {
            const status = jobStatusPill(j.status)
            const duration = jobDurationMs(j)
            const truncated = j.stdout_truncated || j.stderr_truncated
            return (
              <TableRow key={j.id}>
                <TableCell>
                  <code className="t-ink">{truncateMiddle(j.id, 18)}</code>
                </TableCell>
                <TableCell>
                  <span className="shui-row">
                    {j.status === 'running' ? <StatusDot tone="accent" pulse /> : null}
                    <Badge variant={status.tone}>{status.label}</Badge>
                  </span>
                </TableCell>
                <TableCell className="t-faint num">{formatEpochMs(j.started_at_ms)}</TableCell>
                <TableCell className="t-faint num">{duration != null ? formatDuration(duration) : '—'}</TableCell>
                <TableCell className="num">
                  {j.exit_code == null ? (
                    <span className="t-faint">—</span>
                  ) : (
                    <span className={j.exit_code === 0 ? 't-accent' : 't-warn'}>{j.exit_code}</span>
                  )}
                </TableCell>
                <TableCell className="t-faint">{truncated ? 'truncated' : '—'}</TableCell>
              </TableRow>
            )
          })}
        </TableBody>
      </Table>
      <StatusBar>summaries only — full output via shell::status</StatusBar>
    </div>
  )
}

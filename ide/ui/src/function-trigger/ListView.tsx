import { EmptyState, StatusDot } from '@iii-dev/console-ui'
import { formatAgeSecs, truncateMiddle } from '../lib/format'
import { StatusPill } from '../lib/terminal'
import { formatEpochMs, jobDurationMs, jobStatusPill } from './format'
import { safeParseResponse, shellListResponseSchema } from './parsers'

interface ShellListViewProps {
  output: unknown
}

/** `1234` → `"1234ms"`; ≥ 10 s humanize via `formatAgeSecs` so
    long-lived jobs read as `2m`/`3h` instead of raw millis. */
function formatDurationMs(ms: number): string {
  return ms < 10_000 ? `${ms}ms` : formatAgeSecs(Math.floor(ms / 1000))
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
      <div className="shui-card pad">
        <EmptyState
          title="no jobs"
          description="no background jobs for this worker."
        />
      </div>
    )
  }

  return (
    <div className="shui-card scroll-x">
      <table className="shui-table">
        <thead>
          <tr className="head-paper">
            <th className="pad-l">job</th>
            <th>status</th>
            <th>started</th>
            <th>duration</th>
            <th>exit</th>
            <th className="pad-r">output</th>
          </tr>
        </thead>
        <tbody>
          {jobs.map((j) => {
            const status = jobStatusPill(j.status)
            const duration = jobDurationMs(j)
            const truncated = j.stdout_truncated || j.stderr_truncated
            return (
              <tr key={j.id}>
                <td className="pad-l">
                  <code className="t-ink">{truncateMiddle(j.id, 18)}</code>
                </td>
                <td>
                  <span className="shui-status-cell">
                    {j.status === 'running' ? (
                      <StatusDot tone="accent" pulse />
                    ) : null}
                    <StatusPill label={status.label} variant={status.tone} />
                  </span>
                </td>
                <td className="t-faint num">{formatEpochMs(j.started_at_ms)}</td>
                <td className="t-faint num">
                  {duration != null ? formatDurationMs(duration) : '—'}
                </td>
                <td className="num">
                  {j.exit_code == null ? (
                    <span className="t-faint">—</span>
                  ) : (
                    <span className={j.exit_code === 0 ? 't-accent' : 't-warn'}>
                      {j.exit_code}
                    </span>
                  )}
                </td>
                <td className="t-faint pad-r">
                  {truncated ? '✂ truncated' : '—'}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
      <div className="shui-table-footnote">
        summaries only — full output via shell::status
      </div>
    </div>
  )
}

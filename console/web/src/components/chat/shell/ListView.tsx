import { Inbox } from 'lucide-react'
import { formatAgeSecs, truncateMiddle } from '@/components/chat/sandbox/format'
import { StatusPill } from '@/components/chat/sandbox/shared'
import { EmptyState } from '@/components/ui/EmptyState'
import { StatusDot } from '@/components/ui/StatusDot'
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
      <div className="border-t border-rule-2 bg-bg p-3">
        <EmptyState
          icon={Inbox}
          title="no jobs"
          description="no background jobs for this worker."
        />
      </div>
    )
  }

  return (
    <div className="border-t border-rule-2 bg-bg overflow-x-auto">
      <table className="w-full font-mono text-[12px] text-ink">
        <thead>
          <tr className="bg-paper-2 border-b border-rule-2 text-[11px] uppercase tracking-[0.06em] text-ink-faint">
            <th className="text-left font-normal px-3 py-1.5">job</th>
            <th className="text-left font-normal px-2 py-1.5">status</th>
            <th className="text-left font-normal px-2 py-1.5">started</th>
            <th className="text-left font-normal px-2 py-1.5">duration</th>
            <th className="text-left font-normal px-2 py-1.5">exit</th>
            <th className="text-left font-normal px-3 py-1.5">output</th>
          </tr>
        </thead>
        <tbody>
          {jobs.map((j) => {
            const status = jobStatusPill(j.status)
            const duration = jobDurationMs(j)
            const truncated = j.stdout_truncated || j.stderr_truncated
            return (
              <tr key={j.id} className="border-b border-rule-2 last:border-b-0">
                <td className="px-3 py-1.5">
                  <code className="text-ink">{truncateMiddle(j.id, 18)}</code>
                </td>
                <td className="px-2 py-1.5">
                  <span className="inline-flex items-center gap-1.5">
                    {j.status === 'running' ? (
                      <StatusDot tone="accent" pulse />
                    ) : null}
                    <StatusPill label={status.label} variant={status.tone} />
                  </span>
                </td>
                <td className="px-2 py-1.5 text-ink-faint tabular-nums">
                  {formatEpochMs(j.started_at_ms)}
                </td>
                <td className="px-2 py-1.5 text-ink-faint tabular-nums">
                  {duration != null ? formatDurationMs(duration) : '—'}
                </td>
                <td className="px-2 py-1.5 tabular-nums">
                  {j.exit_code == null ? (
                    <span className="text-ink-faint">—</span>
                  ) : (
                    <span
                      className={
                        j.exit_code === 0 ? 'text-accent' : 'text-warn'
                      }
                    >
                      {j.exit_code}
                    </span>
                  )}
                </td>
                <td className="px-3 py-1.5 text-ink-faint">
                  {truncated ? '✂ truncated' : '—'}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
      <div className="px-3 py-1.5 font-mono text-[11px] text-ink-ghost border-t border-rule-2">
        summaries only — full output via shell::status
      </div>
    </div>
  )
}

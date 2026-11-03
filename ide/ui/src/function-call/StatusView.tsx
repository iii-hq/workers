import { Badge } from '@iii-dev/console-ui'
import { formatDuration } from '@iii-dev/console-ui/format'
import { pillForExit, truncateMiddle } from '../lib/format'
import {
  formatArgv,
  formatEpochMs,
  jobDurationMs,
  jobStatusPill,
} from './format'
import {
  type JobRecord,
  safeParseResponse,
  shellStatusRequestSchema,
  shellStatusResponseSchema,
} from './parsers'
import { items, kv, StreamBody, TerminalCard } from './shared'

interface ShellStatusViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/**
 * `shell::status` — the full JobRecord as real terminal chrome (the
 * record carries argv + buffered stdout/stderr, unlike list's
 * summaries). The frequent `S211 no such job` failure never reaches
 * this view — the dispatch error pre-pass renders it as an error card.
 */
export function ShellStatusView({
  input,
  output,
  running,
}: ShellStatusViewProps) {
  const req = shellStatusRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp =
    output != null ? safeParseResponse(shellStatusResponseSchema, output) : null

  if (!resp) {
    // The status call itself is in flight (or output is missing):
    // items-only header, `running` drives the executing shimmer.
    return <TerminalCard running={running} items={[kv('job', truncateMiddle(req.data.job_id, 18))]} />
  }

  const job = resp.job
  return (
    <TerminalCard
      command={formatArgv(job.argv)}
      running={running}
      items={items(
        kv('job', truncateMiddle(job.id, 18)),
        kv('started', formatEpochMs(job.started_at_ms)),
        job.finished_at_ms != null ? kv('finished', formatEpochMs(job.finished_at_ms)) : null,
      )}
      footer={<StatusFooter job={job} />}
    >
      <StreamBody stdout={job.stdout} stderr={job.stderr} />
    </TerminalCard>
  )
}

function StatusFooter({ job }: { job: JobRecord }) {
  const status = jobStatusPill(job.status)
  const exit = pillForExit(job.exit_code)
  const duration = jobDurationMs(job)
  return (
    <>
      <Badge variant={status.tone}>{status.label}</Badge>
      {/* exit_code is null by definition while running — "no exit"/warn
         would be misleading there, so the pill is terminal-only. */}
      {job.status !== 'running' ? <Badge variant={exit.tone}>{exit.label}</Badge> : null}
      {/* No live-ticking duration for running jobs (no re-render
         source); the `started` item's relative time covers it. */}
      {duration != null ? <Badge>{formatDuration(duration)}</Badge> : null}
      {job.stdout_truncated ? <Badge variant="warn">stdout truncated</Badge> : null}
      {job.stderr_truncated ? <Badge variant="warn">stderr truncated</Badge> : null}
    </>
  )
}

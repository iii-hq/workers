import {
  formatAgeSecs,
  pillForExit,
  truncateMiddle,
} from '@/components/chat/sandbox/format'
import { AnsiOutput } from '@/components/chat/sandbox/terminal/AnsiOutput'
import {
  Chip,
  FooterPill,
  Terminal,
} from '@/components/chat/sandbox/terminal/Terminal'
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

interface ShellStatusViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/** `1234` → `"1234ms"`; ≥ 10 s humanize via `formatAgeSecs` so
    long-lived jobs read as `2m`/`3h` instead of raw millis. */
function formatDurationMs(ms: number): string {
  return ms < 10_000 ? `${ms}ms` : formatAgeSecs(Math.floor(ms / 1000))
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
    // chips-only header, `running` drives the executing shimmer.
    return (
      <Terminal
        running={running}
        chips={<Chip label="job">{truncateMiddle(req.data.job_id, 18)}</Chip>}
      />
    )
  }

  const job = resp.job
  return (
    <Terminal
      command={formatArgv(job.argv)}
      running={running}
      chips={
        <>
          <Chip label="job">{truncateMiddle(job.id, 18)}</Chip>
          <Chip label="started">{formatEpochMs(job.started_at_ms)}</Chip>
          {job.finished_at_ms != null ? (
            <Chip label="finished">{formatEpochMs(job.finished_at_ms)}</Chip>
          ) : null}
        </>
      }
      footer={<StatusFooter job={job} />}
    >
      <AnsiOutput stdout={job.stdout} stderr={job.stderr} />
    </Terminal>
  )
}

function StatusFooter({ job }: { job: JobRecord }) {
  const status = jobStatusPill(job.status)
  const exit = pillForExit(job.exit_code)
  const duration = jobDurationMs(job)
  return (
    <>
      <FooterPill tone={status.tone}>{status.label}</FooterPill>
      {/* exit_code is null by definition while running — "no exit"/warn
         would be misleading there, so the pill is terminal-only. */}
      {job.status !== 'running' ? (
        <FooterPill tone={exit.tone}>{exit.label}</FooterPill>
      ) : null}
      {/* No live-ticking duration for running jobs (no re-render
         source); the `started` chip's relative time covers it. */}
      {duration != null ? (
        <FooterPill>{formatDurationMs(duration)}</FooterPill>
      ) : null}
      {job.stdout_truncated ? (
        <FooterPill tone="warn">stdout truncated</FooterPill>
      ) : null}
      {job.stderr_truncated ? (
        <FooterPill tone="warn">stderr truncated</FooterPill>
      ) : null}
    </>
  )
}

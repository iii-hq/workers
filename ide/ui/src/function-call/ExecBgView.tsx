import { Badge, Button, Chip, type Host } from '@iii-dev/console-ui'
import { errorMessage, formatDuration } from '@iii-dev/console-ui/format'
import { RefreshCw } from 'lucide-react'
import { useEffect, useState } from 'react'
import { execItems, ShellExecPreviewRow } from './ExecView'
import { formatArgv, formatShellCommand } from './format'
import {
  type JobRecord,
  type ShellExecBgRequest,
  type ShellExecBgResponse,
  safeParseResponse,
  shellExecBgRequestSchema,
  shellExecBgResponseSchema,
  shellStatusResponseSchema,
} from './parsers'
import { isSandboxTarget, StreamBody, TerminalCard } from './shared'
import { StatusFooter } from './StatusView'

const OUTPUT_POLL_MS = 2_000
/** shell::kill marks a job killed before its output is final, which a host job's
    drain can delay by 10 s. Any other end publishes its output with its status. */
const READS_AFTER_KILL = 6
/** What the stream pane shows without its expand toggle. Few lines: long ones
    wrap on a phone, and the pane's height must still reach the newest. */
const LIVE_LINES = 6
const LIVE_CHARS = 2_000

interface ShellExecBgViewProps {
  host: Host
  input: unknown
  output: unknown
  running?: boolean
}

export function ShellExecBgView({
  host,
  input,
  output,
  running,
}: ShellExecBgViewProps) {
  const req = shellExecBgRequestSchema.safeParse(input)
  if (!req.success) return null
  const respData =
    output != null ? safeParseResponse(shellExecBgResponseSchema, output) : null
  return (
    <TerminalCard
      command={formatShellCommand(req.data)}
      running={running}
      items={execItems(req.data)}
      chips={<Chip>bg</Chip>}
      footer={respData ? <ShellExecBgFooter req={req.data} /> : null}
    >
      {respData ? (
        <>
          <BgStartedBody req={req.data} resp={respData} />
          <JobOutput host={host} jobId={respData.job_id} />
        </>
      ) : null}
    </TerminalCard>
  )
}

/** Compact `$ cmd args` preview used in the pending-approval state —
    the shared exec row plus the `bg` marker chip. */
export function ShellExecBgPreview({ input }: { input: unknown }) {
  return <ShellExecPreviewRow input={input} bg />
}

/** Success body — there is no stdout for a background spawn. The job_id
    is the copy-handle for `shell::status`/`shell::kill`, so it renders
    full and untruncated. The server-resolved argv appears as a faint
    second line only when shell-words tokenization changed the spelling
    of what was typed (that's when it's informative). */
function BgStartedBody({
  req,
  resp,
}: {
  req: ShellExecBgRequest
  resp: ShellExecBgResponse
}) {
  const resolved = formatArgv(resp.argv)
  return (
    <div className="shui-card-body">
      <div className="shui-baseline-row">
        <RefreshCw aria-hidden className="shui-glyph t-accent" />
        <span className="t-ink">started</span>
        <code className="shui-inline-code">{resp.job_id}</code>
      </div>
      {resolved !== formatShellCommand(req) ? (
        <div className="shui-resolved-argv">{`$ ${resolved}`}</div>
      ) : null}
    </div>
  )
}

/** The newest lines of a running job's stream: the pane below shows the start of
    what it is given, and on a running job the end is the part that answers "is it
    stuck?". */
export function liveTail(text: string): string {
  const body = text.endsWith('\n') ? text.slice(0, -1) : text
  return body.split('\n').slice(-LIVE_LINES).join('\n').slice(-LIVE_CHARS)
}

/** Read a job's record while the tab is visible, until the job ends or a call
    fails; returns the stop. */
export function pollJob(
  host: Host,
  jobId: string,
  onJob: (job: JobRecord) => void,
  onError: (message: string) => void,
): () => void {
  let stopped = false
  let readsAfterKill = READS_AFTER_KILL
  let timer: ReturnType<typeof setTimeout> | undefined
  const poll = async () => {
    if (globalThis.document?.hidden) {
      timer = setTimeout(poll, OUTPUT_POLL_MS)
      return
    }
    try {
      const output = await host.iii.call('shell::status', { job_id: jobId })
      if (stopped) return
      const job = safeParseResponse(shellStatusResponseSchema, output)?.job
      if (!job) return onError('unexpected shell::status response')
      onJob(job)
      if (job.status === 'running' || (job.status === 'killed' && readsAfterKill-- > 0)) {
        timer = setTimeout(poll, OUTPUT_POLL_MS)
      }
    } catch (error) {
      if (!stopped) onError(errorMessage(error))
    }
  }
  void poll()
  return () => {
    stopped = true
    clearTimeout(timer)
  }
}

/** A job's output on request. Closed by default, because every job card in a
    long chat would otherwise poll `shell::status`. A running host job shows
    its output so far; a sandbox job shows it once it ends. */
function JobOutput({ host, jobId }: { host: Host; jobId: string }) {
  const [open, setOpen] = useState(false)
  const [job, setJob] = useState<JobRecord | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!open) return
    setError(null)
    return pollJob(host, jobId, setJob, setError)
  }, [open, host, jobId])
  return (
    <>
      <div className="shell-agent-card-actions">
        <Button
          variant="ghost"
          size="sm"
          aria-expanded={open}
          onClick={() => setOpen((current) => !current)}
        >
          {open ? 'Hide output' : 'Show output'}
        </Button>
      </div>
      {open && error ? <JobOutputMessage>{`Output unavailable: ${error}`}</JobOutputMessage> : null}
      {open && !job && !error ? <JobOutputMessage>Reading output…</JobOutputMessage> : null}
      {open && job ? <JobOutputBody job={job} stale={error !== null} /> : null}
    </>
  )
}

function JobOutputMessage({ children }: { children: string }) {
  return (
    <div className="shui-card-body" role="status">
      <div className="shui-resolved-argv">{children}</div>
    </div>
  )
}

export function JobOutputBody({ job, stale }: { job: JobRecord; stale: boolean }) {
  const running = job.status === 'running'
  const stdout = liveTail(job.stdout)
  const stderr = liveTail(job.stderr)
  return (
    <>
      {running && !stdout && !stderr ? (
        <JobOutputMessage>No output yet.</JobOutputMessage>
      ) : running ? (
        <>
          {/* A failed read means no more is coming, so nothing is promised. */}
          {stale ? null : <JobOutputMessage>Latest lines; the saved output appears when the job ends.</JobOutputMessage>}
          <StreamBody stdout={stdout} stderr={stderr} />
        </>
      ) : (
        <StreamBody stdout={job.stdout} stderr={job.stderr} />
      )}
      {/* A failed read leaves this record out of date, so its status stays out. */}
      {stale ? null : (
        <div className="shui-card-foot">
          <StatusFooter job={job} />
          {running ? <Badge>{formatDuration(Date.now() - job.started_at_ms)}</Badge> : null}
        </div>
      )}
    </>
  )
}

function ShellExecBgFooter({ req }: { req: ShellExecBgRequest }) {
  return (
    <>
      <Badge variant="accent">job started</Badge>
      {/* Host-targeted background jobs IGNORE timeout_ms (wire semantic);
          surface the silently-dropped knob. */}
      {!isSandboxTarget(req.target) && typeof req.timeout_ms === 'number' ? (
        <Badge variant="warn">timeout_ms ignored (host bg)</Badge>
      ) : null}
    </>
  )
}

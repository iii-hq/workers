import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import {
  safeParseRequest,
  safeParseResponse,
  statusState,
  type WorkerStatusState,
  workerStatusRequestSchema,
  workerStatusResponseSchema,
} from './parsers'

interface WorkerStatusViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/* Pill copy + variant for each of the four derived states. The variant
 * intentionally mirrors the daemon's own severity ordering:
 *   running       -> accent  (alive)
 *   provisioning  -> default (not running, no log signal — the daemon's own
 *                            "likely still provisioning" branch; label tracks
 *                            the hint, neutral variant never over-promises a
 *                            healthy boot since the tail may be rotated)
 *   stopped       -> warn    (crashed / never booted; failure in stderr_tail)
 *   not-installed -> alert   (not declared in project config at all) */
const STATE_PRESENTATION: Record<
  WorkerStatusState,
  { label: string; variant: 'default' | 'accent' | 'warn' | 'alert' }
> = {
  running: { label: 'running', variant: 'accent' },
  provisioning: { label: 'provisioning', variant: 'default' },
  stopped: { label: 'stopped', variant: 'warn' },
  'not-installed': { label: 'not installed', variant: 'alert' },
}

export function WorkerStatusView({
  input,
  output,
  running,
}: WorkerStatusViewProps) {
  const req = safeParseRequest(workerStatusRequestSchema, input)
  if (!req) return null

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="checking…" variant="default" />
        </MetaRow>
        <TitleRow name={req.name} />
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          {`· checking ${req.name}…`}
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(workerStatusResponseSchema, output)
  if (!resp) {
    // Request parsed but response didn't: never null out; show the name we
    // already know plus an unavailable affordance so the row stays legible.
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="status unavailable" variant="warn" />
        </MetaRow>
        <TitleRow name={req.name} />
        <GhostRow label="status payload could not be read" />
      </div>
    )
  }

  const state = statusState(resp)
  const presentation = STATE_PRESENTATION[state]

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label={presentation.label} variant={presentation.variant} />
        <TypeChip workerType={resp.worker_type} />
        {resp.version ? <VersionChip version={resp.version} /> : null}
        {typeof resp.pid === 'number' ? <PidChip pid={resp.pid} /> : null}
        <InstalledChip installed={resp.installed} />
      </MetaRow>
      <TitleRow name={resp.name} />
      <HintRow hint={resp.hint} />
      {resp.logs_dir ? <LogsDirRow path={resp.logs_dir} /> : null}
      <LogPane label="stderr" lines={resp.stderr_tail} tone="warn" />
      <LogPane label="stdout" lines={resp.stdout_tail} tone="ink" />
    </div>
  )
}

function TitleRow({ name }: { name: string }) {
  return (
    <ActionLine symbol="ƒ" tone="accent">
      <span className="font-mono text-[13px] text-accent break-all">
        {name}
      </span>
    </ActionLine>
  )
}

/* The hint is the agent-facing next-step guidance: surface it as an advisory
 * ActionLine, never tuck it away. */
function HintRow({ hint }: { hint: string }) {
  return (
    <ActionLine symbol="›" tone="ink">
      <span className="font-mono text-[12.5px] text-ink break-words">
        {hint}
      </span>
    </ActionLine>
  )
}

function LogsDirRow({ path }: { path: string }) {
  return (
    <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[11px] text-ink-faint break-all">
      <span className="uppercase tracking-[0.06em] text-[10px] mr-1">logs</span>
      {path}
    </div>
  )
}

/* Labeled monospace log pane. Empty tail collapses to a quiet ghost row
 * (never a blank box); line order is preserved verbatim. stderr is toned
 * apart from stdout so failures read at a glance. */
function LogPane({
  label,
  lines,
  tone,
}: {
  label: string
  lines: string[]
  tone: 'warn' | 'ink'
}) {
  const labelTone = tone === 'warn' ? 'text-warn' : 'text-ink-faint'
  return (
    <div className="border-b border-rule-2">
      <div className="px-3 pt-2 pb-1 bg-paper-2 border-b border-rule-2">
        <span
          className={`font-mono text-[10px] uppercase tracking-[0.06em] ${labelTone}`}
        >
          {label}
        </span>
      </div>
      {lines.length === 0 ? (
        <GhostRow label={`no ${label}`} />
      ) : (
        // Newline-joined into one <pre> so order + blank lines survive
        // verbatim without per-line index keys (log lines can repeat).
        <pre className="px-3 py-2 font-mono text-[11.5px] text-ink whitespace-pre-wrap break-words leading-relaxed">
          {lines.join('\n')}
        </pre>
      )}
    </div>
  )
}

function GhostRow({ label }: { label: string }) {
  return (
    <div className="px-3 py-2 font-mono text-[11.5px] text-ink-ghost">
      {`· ${label}`}
    </div>
  )
}

function TypeChip({ workerType }: { workerType: string }) {
  return (
    <Chip>
      <span className="text-ink-faint uppercase tracking-[0.06em]">type</span>
      <span className="ml-1 text-ink">{workerType}</span>
    </Chip>
  )
}

function VersionChip({ version }: { version: string }) {
  return (
    <Chip>
      <span className="text-ink-faint uppercase tracking-[0.06em]">
        version
      </span>
      <span className="ml-1 text-ink tabular-nums">{version}</span>
    </Chip>
  )
}

function PidChip({ pid }: { pid: number }) {
  return (
    <Chip>
      <span className="text-ink-faint uppercase tracking-[0.06em]">pid</span>
      <span className="ml-1 text-ink tabular-nums">{pid}</span>
    </Chip>
  )
}

function InstalledChip({ installed }: { installed: boolean }) {
  return (
    <Chip>
      <span className="uppercase tracking-[0.06em] text-ink-faint">
        {installed ? 'installed' : 'not installed'}
      </span>
    </Chip>
  )
}

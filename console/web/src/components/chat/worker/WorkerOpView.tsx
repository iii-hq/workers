import type { ReactNode } from 'react'
import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import {
  safeParseRequest,
  safeParseResponse,
  type WorkerSource,
  workerAddRequestSchema,
  workerAddResponseSchema,
  workerClearRequestSchema,
  workerClearResponseSchema,
  workerRemoveRequestSchema,
  workerRemoveResponseSchema,
  workerStartRequestSchema,
  workerStartResponseSchema,
  workerStopRequestSchema,
  workerStopResponseSchema,
  workerUpdateRequestSchema,
  workerUpdateResponseSchema,
} from './parsers'

interface WorkerOpViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/* ---------------- worker::start ---------------- */

export function WorkerStartView({ input, output, running }: WorkerOpViewProps) {
  const req = safeParseRequest(workerStartRequestSchema, input)
  if (!req) return null

  if (running) {
    return (
      <OpShell
        statusLabel="starting…"
        statusVariant="default"
        title={req.name}
        chips={[
          req.port != null ? portChip(req.port) : null,
          req.config ? configChip(req.config) : null,
        ]}
      >
        <RunningHint label={`starting ${req.name}…`} />
      </OpShell>
    )
  }

  const resp = safeParseResponse(workerStartResponseSchema, output)
  if (!resp) return null

  return (
    <OpShell
      statusLabel="started"
      statusVariant="accent"
      title={resp.name}
      chips={[
        typeof resp.pid === 'number' ? (
          <PidChip key="pid" pid={resp.pid} />
        ) : null,
        typeof resp.port === 'number' ? portChip(resp.port) : null,
      ]}
    />
  )
}

/* ---------------- worker::stop ---------------- */

export function WorkerStopView({ input, output, running }: WorkerOpViewProps) {
  const req = safeParseRequest(workerStopRequestSchema, input)
  if (!req) return null

  if (running) {
    return (
      <OpShell statusLabel="stopping…" statusVariant="default" title={req.name}>
        <RunningHint label={`stopping ${req.name}…`} />
      </OpShell>
    )
  }

  const resp = safeParseResponse(workerStopResponseSchema, output)
  if (!resp) return null

  return (
    <OpShell
      statusLabel={resp.stopped ? 'stopped' : 'still running'}
      statusVariant={resp.stopped ? 'accent' : 'warn'}
      title={resp.name}
    />
  )
}

/* ---------------- worker::add ---------------- */

export function WorkerAddView({ input, output, running }: WorkerOpViewProps) {
  const req = safeParseRequest(workerAddRequestSchema, input)
  if (!req) return null
  const sourceLabel = describeSource(req.source)

  if (running) {
    return (
      <OpShell
        statusLabel="installing…"
        statusVariant="default"
        title={sourceTitle(req.source)}
        chips={[
          <Chip key="src">
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              source
            </span>
            <span className="ml-1 text-ink">{req.source.kind}</span>
          </Chip>,
          req.force ? <FlagChip key="force" label="force" /> : null,
          req.reset_config ? (
            <FlagChip key="reset" label="reset config" />
          ) : null,
        ]}
      >
        <ActionLine symbol="→" tone="ink">
          <span className="break-all">{sourceLabel}</span>
        </ActionLine>
        <RunningHint label="installing worker…" />
      </OpShell>
    )
  }

  const resp = safeParseResponse(workerAddResponseSchema, output)
  if (!resp) return null

  return (
    <OpShell
      statusLabel={resp.status}
      statusVariant={statusVariantForAddStatus(resp.status)}
      title={resp.name}
      chips={[
        resp.version ? (
          <Chip key="v">
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              version
            </span>
            <span className="ml-1 text-ink">{resp.version}</span>
          </Chip>
        ) : null,
        resp.awaited_ready ? <FlagChip key="ready" label="ready" /> : null,
      ]}
    >
      <ActionLine symbol="→" tone="ink">
        <span className="break-all">{sourceLabel}</span>
      </ActionLine>
      <ConfigPathRow path={resp.config_path} />
    </OpShell>
  )
}

/* ---------------- worker::remove ---------------- */

export function WorkerRemoveView({
  input,
  output,
  running,
}: WorkerOpViewProps) {
  const req = safeParseRequest(workerRemoveRequestSchema, input)
  if (!req) return null

  if (running) {
    return (
      <OpShell
        statusLabel="removing…"
        statusVariant="default"
        title={removeTargetTitle(req)}
        chips={[
          req.all ? <FlagChip key="all" label="all" /> : null,
          req.yes ? <FlagChip key="yes" label="yes" /> : null,
        ]}
      >
        <RunningHint label="removing workers…" />
      </OpShell>
    )
  }

  const resp = safeParseResponse(workerRemoveResponseSchema, output)
  if (!resp) return null

  return (
    <OpShell
      statusLabel={
        resp.removed.length === 0
          ? 'nothing removed'
          : `removed · ${resp.removed.length}`
      }
      statusVariant={resp.removed.length === 0 ? 'warn' : 'accent'}
      title={removeTargetTitle(req)}
    >
      <NameList names={resp.removed} empty="· no workers removed" />
    </OpShell>
  )
}

/* ---------------- worker::update ---------------- */

export function WorkerUpdateView({
  input,
  output,
  running,
}: WorkerOpViewProps) {
  const req = safeParseRequest(workerUpdateRequestSchema, input)
  if (!req) return null
  const target =
    req.names && req.names.length > 0 ? req.names.join(', ') : 'all installed'

  if (running) {
    return (
      <OpShell statusLabel="updating…" statusVariant="default" title={target}>
        <RunningHint label="updating workers…" />
      </OpShell>
    )
  }

  const resp = safeParseResponse(workerUpdateResponseSchema, output)
  if (!resp) return null

  return (
    <OpShell
      statusLabel={
        resp.updated.length === 0
          ? 'already current'
          : `updated · ${resp.updated.length}`
      }
      statusVariant={resp.updated.length === 0 ? 'default' : 'accent'}
      title={target}
    >
      {resp.updated.length === 0 ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          · everything was already at the latest version
        </div>
      ) : (
        <ul className="divide-y divide-rule-2">
          {resp.updated.map((u) => (
            <li
              key={u.name}
              className="px-3 py-1.5 flex items-baseline gap-2 flex-wrap"
            >
              <span className="font-mono text-[12.5px] text-accent break-all">
                {u.name}
              </span>
              <span className="font-mono text-[11.5px] text-ink-faint tabular-nums">
                v{u.from_version} → v{u.to_version}
              </span>
            </li>
          ))}
        </ul>
      )}
    </OpShell>
  )
}

/* ---------------- worker::clear ---------------- */

export function WorkerClearView({ input, output, running }: WorkerOpViewProps) {
  const req = safeParseRequest(workerClearRequestSchema, input)
  if (!req) return null
  const target =
    req.names && req.names.length > 0
      ? req.names.join(', ')
      : req.all
        ? 'all artifacts'
        : 'no target'

  if (running) {
    return (
      <OpShell statusLabel="clearing…" statusVariant="default" title={target}>
        <RunningHint label="clearing artifacts…" />
      </OpShell>
    )
  }

  const resp = safeParseResponse(workerClearResponseSchema, output)
  if (!resp) return null

  return (
    <OpShell
      statusLabel={
        resp.cleared.length === 0
          ? 'nothing cleared'
          : `cleared · ${resp.cleared.length}`
      }
      statusVariant={resp.cleared.length === 0 ? 'warn' : 'accent'}
      title={target}
    >
      <NameList names={resp.cleared} empty="· no artifacts cleared" />
    </OpShell>
  )
}

/* ---------------- shared shell ---------------- */

interface OpShellProps {
  statusLabel: string
  statusVariant: 'default' | 'accent' | 'warn' | 'alert'
  title: string
  chips?: (ReactNode | null)[]
  children?: ReactNode
}

function OpShell({
  statusLabel,
  statusVariant,
  title,
  chips,
  children,
}: OpShellProps) {
  const renderedChips = (chips ?? []).filter(Boolean) as ReactNode[]
  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label={statusLabel} variant={statusVariant} />
        {renderedChips}
      </MetaRow>
      <ActionLine symbol="ƒ" tone="accent">
        <span className="font-mono text-[13px] text-accent break-all">
          {title}
        </span>
      </ActionLine>
      {children}
    </div>
  )
}

function RunningHint({ label }: { label: string }) {
  return (
    <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
      · {label}
    </div>
  )
}

function ConfigPathRow({ path }: { path: string }) {
  return (
    <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[11px] text-ink-faint break-all">
      <span className="uppercase tracking-[0.06em] text-[10px] mr-1">
        config
      </span>
      {path}
    </div>
  )
}

function NameList({ names, empty }: { names: string[]; empty: string }) {
  if (names.length === 0) {
    return (
      <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
        {empty}
      </div>
    )
  }
  return (
    <ul className="divide-y divide-rule-2">
      {names.map((n) => (
        <li
          key={n}
          className="px-3 py-1.5 font-mono text-[12.5px] text-accent break-all"
        >
          {n}
        </li>
      ))}
    </ul>
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

function portChip(port: number) {
  return (
    <Chip key="port">
      <span className="text-ink-faint uppercase tracking-[0.06em]">port</span>
      <span className="ml-1 text-ink tabular-nums">{port}</span>
    </Chip>
  )
}

function configChip(path: string) {
  return (
    <Chip key="config">
      <span className="text-ink-faint uppercase tracking-[0.06em]">config</span>
      <span
        className="ml-1 text-ink break-all max-w-[24ch] truncate inline-block align-bottom"
        title={path}
      >
        {path}
      </span>
    </Chip>
  )
}

function FlagChip({ label }: { label: string }) {
  return (
    <Chip>
      <span className="uppercase tracking-[0.06em] text-ink-faint">
        {label}
      </span>
    </Chip>
  )
}

/* ---------------- helpers ---------------- */

function describeSource(source: WorkerSource): string {
  switch (source.kind) {
    case 'registry':
      return source.version
        ? `registry:${source.name}@${source.version}`
        : `registry:${source.name}`
    case 'oci':
      return `oci:${source.reference}`
    case 'local':
      return `local:${source.path}`
  }
}

function sourceTitle(source: WorkerSource): string {
  switch (source.kind) {
    case 'registry':
      return source.name
    case 'oci':
      return source.reference.split('/').pop() ?? source.reference
    case 'local':
      return source.path.split('/').pop() ?? source.path
  }
}

function removeTargetTitle(req: { names?: string[]; all?: boolean }): string {
  if (req.all) return 'all installed'
  if (req.names && req.names.length > 0) return req.names.join(', ')
  return '—'
}

function statusVariantForAddStatus(
  status: 'installed' | 'already_current' | 'repaired' | 'replaced',
): 'accent' | 'default' | 'warn' {
  if (status === 'installed' || status === 'repaired' || status === 'replaced')
    return 'accent'
  return 'default'
}

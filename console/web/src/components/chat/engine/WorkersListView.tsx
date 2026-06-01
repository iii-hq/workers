import type { ReactNode } from 'react'
import {
  safeParseRequest,
  safeParseResponse,
  type WorkerSummary,
  type WorkersListRequest,
  workersListRequestSchema,
  workersListResponseSchema,
} from './parsers'
import { FilterChip, ListHeader } from './shared'

interface WorkersListViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function WorkersListView({
  input,
  output,
  running,
}: WorkersListViewProps) {
  const req = safeParseRequest(workersListRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <ListHeader
          count={0}
          noun="workers"
          tone="default"
          filters={<RequestFilters req={req ?? undefined} />}
        />
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · listing workers…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(workersListResponseSchema, output)
  if (!resp) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <ListHeader
        count={resp.workers.length}
        noun="workers"
        filters={<RequestFilters req={req ?? undefined} />}
      />
      {resp.workers.length === 0 ? (
        <div className="px-3 py-4 font-mono text-[12.5px] text-ink-ghost">
          · no workers connected
        </div>
      ) : (
        <ul className="divide-y divide-rule-2">
          {resp.workers.map((w) => (
            <WorkerRow key={w.id} worker={w} />
          ))}
        </ul>
      )}
    </div>
  )
}

function WorkerRow({ worker }: { worker: WorkerSummary }) {
  return (
    <li className="px-3 py-2 flex flex-col gap-1">
      <div className="flex items-baseline gap-2 flex-wrap">
        <span className="font-mono text-[12.5px] text-accent break-all">
          {worker.name ?? worker.id}
        </span>
        <StatusBadge status={worker.status} />
        {worker.runtime ? (
          <SmallChip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              runtime
            </span>
            <span className="ml-1 text-ink">{worker.runtime}</span>
          </SmallChip>
        ) : null}
        {worker.os ? (
          <SmallChip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              os
            </span>
            <span className="ml-1 text-ink">{worker.os}</span>
          </SmallChip>
        ) : null}
        {worker.isolation ? (
          <SmallChip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              isolation
            </span>
            <span className="ml-1 text-ink">{worker.isolation}</span>
          </SmallChip>
        ) : null}
      </div>
      <div className="flex items-baseline gap-2 flex-wrap font-mono text-[11px] text-ink-faint">
        <span title={worker.id}>id: {shortenId(worker.id)}</span>
        {worker.version ? <span>· v{worker.version}</span> : null}
        <span>
          · <span className="tabular-nums">{worker.function_count}</span>{' '}
          {worker.function_count === 1 ? 'fn' : 'fns'}
        </span>
        {worker.active_invocations > 0 ? (
          <span className="text-accent">
            · <span className="tabular-nums">{worker.active_invocations}</span>{' '}
            active
          </span>
        ) : null}
        {worker.ip_address ? <span>· {worker.ip_address}</span> : null}
        <span>· {formatConnectedFor(worker.connected_at_ms)}</span>
      </div>
      {worker.description ? (
        <div className="font-mono text-[12px] text-ink-faint leading-[1.55]">
          {worker.description}
        </div>
      ) : null}
    </li>
  )
}

function StatusBadge({ status }: { status: string }) {
  const variant = statusToVariant(status)
  return (
    <span
      className={`font-mono text-[10px] uppercase tracking-[0.06em] border px-1.5 py-0.5 ${variant}`}
    >
      {status}
    </span>
  )
}

function statusToVariant(status: string): string {
  const s = status.toLowerCase()
  if (s === 'connected' || s === 'active' || s === 'ready') {
    return 'text-accent border-accent/40 bg-paper-2'
  }
  if (s === 'disconnected' || s === 'stopped' || s === 'down') {
    return 'text-ink-faint border-rule-2 bg-paper-2'
  }
  return 'text-warn border-warn/40 bg-paper-2'
}

function SmallChip({ children }: { children: ReactNode }) {
  return (
    <span className="font-mono text-[10px] border border-rule-2 bg-paper-2 px-1.5 py-0.5">
      {children}
    </span>
  )
}

function shortenId(id: string): string {
  if (id.length <= 14) return id
  return `${id.slice(0, 8)}…${id.slice(-4)}`
}

function formatConnectedFor(connectedAtMs: number): string {
  const now = Date.now()
  const ms = Math.max(0, now - connectedAtMs)
  if (ms < 60_000) return `${Math.floor(ms / 1000)}s up`
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m up`
  if (ms < 86_400_000) return `${Math.floor(ms / 3_600_000)}h up`
  return `${Math.floor(ms / 86_400_000)}d up`
}

function RequestFilters({ req }: { req?: WorkersListRequest }) {
  if (!req) return null
  const chips: ReactNode[] = []
  if (req.runtime) {
    chips.push(<FilterChip key="runtime" label="runtime" value={req.runtime} />)
  }
  if (req.status) {
    chips.push(<FilterChip key="status" label="status" value={req.status} />)
  }
  if (req.search) {
    chips.push(<FilterChip key="search" label="search" value={req.search} />)
  }
  return chips.length > 0 ? chips : null
}

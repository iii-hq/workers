import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import {
  safeParseRequest,
  safeParseResponse,
  type WorkerInfoResponse,
  type WorkerMetrics,
  workerInfoRequestSchema,
  workerInfoResponseSchema,
} from './parsers'

interface WorkerInfoViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function WorkerInfoView({
  input,
  output,
  running,
}: WorkerInfoViewProps) {
  const req = safeParseRequest(workerInfoRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? (
            <Chip>
              <span className="text-ink-faint uppercase tracking-[0.06em]">
                worker
              </span>
              <span className="ml-1 text-ink break-all">{req.name}</span>
            </Chip>
          ) : null}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · inspecting worker…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(workerInfoResponseSchema, output)
  if (!resp) return null

  const { worker, functions, trigger_types, registered_triggers } = resp

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={worker.status}
          variant={statusVariant(worker.status)}
        />
        {worker.runtime ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              runtime
            </span>
            <span className="ml-1 text-ink">{worker.runtime}</span>
          </Chip>
        ) : null}
        {worker.version ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              version
            </span>
            <span className="ml-1 text-ink">{worker.version}</span>
          </Chip>
        ) : null}
        {worker.os ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              os
            </span>
            <span className="ml-1 text-ink">{worker.os}</span>
          </Chip>
        ) : null}
        {worker.internal ? (
          <Chip>
            <span className="uppercase tracking-[0.06em] text-ink-faint">
              internal
            </span>
          </Chip>
        ) : null}
      </MetaRow>
      <ActionLine symbol="ƒ" tone="accent">
        <span className="font-mono text-[13px] text-accent break-all">
          {worker.name ?? worker.id}
        </span>
      </ActionLine>
      {worker.description ? (
        <div className="px-3 py-2 border-b border-rule-2 font-mono text-[12px] text-ink-faint leading-[1.55]">
          {worker.description}
        </div>
      ) : null}
      <IdentityRow worker={worker} />
      {worker.latest_metrics ? (
        <MetricsRow metrics={worker.latest_metrics} />
      ) : null}
      <SubList
        title="functions"
        empty="no functions registered"
        items={functions.map((fn) => ({
          key: fn.function_id,
          left: fn.function_id,
          right: fn.description ?? null,
        }))}
      />
      <SubList
        title="trigger types"
        empty="no trigger types"
        items={trigger_types.map((t) => ({
          key: t.id,
          left: t.id,
          right: t.description,
        }))}
      />
      <SubList
        title="registered triggers"
        empty="no registered triggers"
        items={registered_triggers.map((t) => ({
          key: t.id,
          left: `${t.trigger_type} → ${t.function_id}`,
          right: t.config_summary,
        }))}
      />
    </div>
  )
}

function IdentityRow({ worker }: { worker: WorkerInfoResponse['worker'] }) {
  const bits: string[] = []
  bits.push(`id: ${worker.id}`)
  if (worker.ip_address) bits.push(worker.ip_address)
  if (worker.isolation) bits.push(`isolation: ${worker.isolation}`)
  if (typeof worker.pid === 'number') bits.push(`pid: ${worker.pid}`)
  bits.push(`active: ${worker.active_invocations}`)
  bits.push(`fns: ${worker.function_count}`)
  bits.push(formatConnectedFor(worker.connected_at_ms))
  return (
    <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[11px] text-ink-faint flex flex-wrap gap-x-3 gap-y-0.5">
      {bits.map((b) => (
        <span key={b}>{b}</span>
      ))}
    </div>
  )
}

function MetricsRow({ metrics }: { metrics: WorkerMetrics }) {
  const cells: { key: string; label: string; value: string }[] = []
  if (typeof metrics.memory_heap_used === 'number') {
    cells.push({
      key: 'heap',
      label: 'heap',
      value: formatBytes(metrics.memory_heap_used),
    })
  }
  if (typeof metrics.memory_rss === 'number') {
    cells.push({
      key: 'rss',
      label: 'rss',
      value: formatBytes(metrics.memory_rss),
    })
  }
  if (typeof metrics.cpu_percent === 'number') {
    cells.push({
      key: 'cpu',
      label: 'cpu',
      value: `${metrics.cpu_percent.toFixed(1)}%`,
    })
  }
  if (typeof metrics.event_loop_lag_ms === 'number') {
    cells.push({
      key: 'lag',
      label: 'loop lag',
      value: `${metrics.event_loop_lag_ms.toFixed(1)}ms`,
    })
  }
  if (typeof metrics.uptime_seconds === 'number') {
    cells.push({
      key: 'uptime',
      label: 'uptime',
      value: formatUptime(metrics.uptime_seconds),
    })
  }
  if (cells.length === 0) return null
  return (
    <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 flex flex-wrap gap-1.5">
      {cells.map((c) => (
        <Chip key={c.key}>
          <span className="text-ink-faint uppercase tracking-[0.06em]">
            {c.label}
          </span>
          <span className="ml-1 text-ink tabular-nums">{c.value}</span>
        </Chip>
      ))}
    </div>
  )
}

interface SubListProps {
  title: string
  empty: string
  items: { key: string; left: string; right: string | null }[]
}

function SubList({ title, empty, items }: SubListProps) {
  return (
    <div className="border-b border-rule-2">
      <div className="px-3 py-1.5 bg-paper-2 border-b border-rule-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
        {title} · {items.length}
      </div>
      {items.length === 0 ? (
        <div className="px-3 py-2 font-mono text-[11.5px] text-ink-ghost">
          · {empty}
        </div>
      ) : (
        <ul className="divide-y divide-rule-2">
          {items.map((it) => (
            <li key={it.key} className="px-3 py-1.5 flex flex-col gap-0.5">
              <span className="font-mono text-[12px] text-accent break-all">
                {it.left}
              </span>
              {it.right ? (
                <span className="font-mono text-[11.5px] text-ink-faint leading-[1.5] break-all">
                  {it.right}
                </span>
              ) : null}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function statusVariant(
  status: string,
): 'accent' | 'default' | 'warn' | 'alert' {
  const s = status.toLowerCase()
  if (s === 'connected' || s === 'active' || s === 'ready') return 'accent'
  if (s === 'disconnected' || s === 'stopped' || s === 'down') return 'default'
  return 'warn'
}

function formatConnectedFor(connectedAtMs: number): string {
  const ms = Math.max(0, Date.now() - connectedAtMs)
  if (ms < 60_000) return `${Math.floor(ms / 1000)}s up`
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m up`
  if (ms < 86_400_000) return `${Math.floor(ms / 3_600_000)}h up`
  return `${Math.floor(ms / 86_400_000)}d up`
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)}MB`
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)}GB`
}

function formatUptime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h`
  return `${Math.floor(seconds / 86400)}d`
}

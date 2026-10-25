import { ChevronRight, EyeOff } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { TraceListItem } from '../hooks/useTraceData'
import { getWorkerColor } from '../lib/traceColors'
import { type RowLabelConfig, resolveRowLabel } from '../lib/traceListItem'
import { formatDuration } from '../lib/traceUtils'

function formatTime(timestamp: number): string {
  const ms = timestamp > 4_102_444_800_000 ? timestamp / 1_000_000 : timestamp
  const date = new Date(ms)
  if (Number.isNaN(date.getTime())) return '—'
  return date.toLocaleTimeString('en-US', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

interface TraceListRowProps {
  trace: TraceListItem
  isSelected: boolean
  isNew: boolean
  onSelect: () => void
  onAnimationEnd: () => void
  /** How the row's primary label is resolved; defaults to function id. */
  label?: RowLabelConfig
  /** When set (and the row has a functionId), hover surfaces a "hide this
   *  function" affordance that adds it to the view's hidden list. */
  onHideFunction?: (functionId: string) => void
  /**
   * True while the trace is still receiving spans beyond its own root (e.g.
   * a queue-triggered trace whose root closes instantly but whose children
   * run for seconds after) — pulses the status dot the same way a genuinely
   * pending root does, per `isTraceLive`.
   */
  isLive?: boolean
}

/**
 * One trace in the list: a status dot, the label, then the row's facts in
 * fixed columns — duration, the workers as coloured chips, and the start
 * time — so a page of traces scans as a table without ruling it as one.
 */
export function TraceListRow({
  trace,
  isSelected,
  isNew,
  onSelect,
  onAnimationEnd,
  label,
  onHideFunction,
  isLive,
}: TraceListRowProps) {
  const resolved = resolveRowLabel(trace, label)
  const canHide = !!onHideFunction && !!trace.functionId
  const pending = trace.status === 'pending' || isLive
  const error = trace.status === 'error'
  const workers = trace.workers.slice(0, 3)
  const moreWorkers = trace.workers.length - workers.length

  return (
    <button
      type="button"
      data-trace-status={trace.status}
      data-trace-function={trace.functionId}
      onClick={onSelect}
      onAnimationEnd={onAnimationEnd}
      aria-expanded={isSelected}
      className={cn(
        'group/row grid h-9 w-full grid-cols-[auto_auto_minmax(0,1fr)_auto_auto_auto] items-center gap-3 rounded-lg px-2 text-left font-inter transition-colors duration-[var(--motion-duration-fast)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50',
        isSelected ? 'bg-muted' : 'hover:bg-muted/60',
        isNew && 'trace-flash',
      )}
    >
      <ChevronRight
        className={cn(
          'size-4 shrink-0 text-muted-foreground transition-transform duration-[var(--motion-duration-control)]',
          isSelected && 'rotate-90',
        )}
        aria-hidden
      />
      <span
        aria-hidden
        title={trace.status}
        className={cn(
          'size-1.5 shrink-0 rounded-full',
          error
            ? 'bg-alert'
            : pending
              ? 'bg-warn live-dot [--live-dot-color:var(--color-warn)]'
              : 'bg-ok',
        )}
      />
      <span className="flex min-w-0 items-center gap-2">
        {resolved.prefix ? (
          <span className="shrink-0 text-[11px] text-muted-foreground">
            {resolved.prefix}
          </span>
        ) : null}
        <span
          className="truncate text-[13px] font-medium text-foreground"
          title={resolved.text}
        >
          {resolved.text}
        </span>
        <code className="hidden shrink-0 font-mono text-[11px] text-muted-foreground tabular-nums @2xl:inline">
          {trace.traceId.slice(0, 8)}
        </code>
        {canHide && (
          // A native <button> can't nest inside the row <button>; a keyboard-
          // operable span keeps the affordance valid HTML.
          // biome-ignore lint/a11y/useSemanticElements: nested interactive inside the row button
          <span
            role="button"
            tabIndex={0}
            title={`hide ${trace.functionId} from the list`}
            aria-label={`hide ${trace.functionId} from the list`}
            onClick={(e) => {
              e.stopPropagation()
              if (trace.functionId) onHideFunction?.(trace.functionId)
            }}
            onKeyDown={(e) => {
              if (e.key !== 'Enter' && e.key !== ' ') return
              e.preventDefault()
              e.stopPropagation()
              if (trace.functionId) onHideFunction?.(trace.functionId)
            }}
            className="flex size-5 shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-[opacity,color,background-color] group-hover/row:opacity-100 hover:bg-muted-foreground/15 hover:text-foreground focus:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
          >
            <EyeOff className="size-4" aria-hidden />
          </span>
        )}
      </span>
      <span className="w-16 text-right font-mono text-[11px] text-foreground tabular-nums">
        {trace.status === 'pending' && trace.duration === undefined
          ? 'running'
          : formatDuration(trace.duration ?? 0)}
      </span>
      <span className="hidden items-center gap-1 @xl:flex">
        {workers.map((worker) => (
          <span
            key={worker}
            className="inline-flex h-5 items-center gap-1 rounded-md border border-border px-1.5 text-[11px] text-muted-foreground"
          >
            <span
              aria-hidden
              className="size-1.5 rounded-full"
              style={{ backgroundColor: getWorkerColor(worker) }}
            />
            {worker}
          </span>
        ))}
        {moreWorkers > 0 ? (
          <span className="text-[11px] text-muted-foreground">
            +{moreWorkers}
          </span>
        ) : null}
      </span>
      <span className="w-16 text-right font-mono text-[11px] text-muted-foreground tabular-nums">
        {formatTime(trace.startTime)}
      </span>
    </button>
  )
}

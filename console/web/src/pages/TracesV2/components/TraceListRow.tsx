import { ChevronRight, EyeOff, Timer, Zap } from 'lucide-react'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import type { TraceListItem } from '../hooks/useTraceData'
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

function statusDotTone(
  status: TraceListItem['status'],
): 'accent' | 'alert' | 'warn' {
  switch (status) {
    case 'ok':
      return 'accent'
    case 'error':
      return 'alert'
    default:
      return 'warn'
  }
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
  return (
    <button
      type="button"
      data-trace-status={trace.status}
      data-trace-function={trace.functionId}
      onClick={onSelect}
      onAnimationEnd={onAnimationEnd}
      aria-expanded={isSelected}
      className={cn(
        'group/row w-full px-4 py-3 border-b border-rule-2 text-left transition-colors',
        isSelected ? 'bg-panel border-l-2 border-l-accent' : 'hover:bg-panel',
        isNew && 'trace-flash',
      )}
    >
      <div className="flex items-center gap-2 mb-1">
        <ChevronRight
          className={cn(
            'w-3 h-3 flex-shrink-0 text-ink-faint transition-transform',
            isSelected && 'rotate-90',
          )}
        />
        <StatusDot
          tone={statusDotTone(trace.status)}
          pulse={trace.status === 'pending' || isLive}
        />
        <span className="font-mono text-[13px] text-ink truncate flex-1 lowercase">
          {resolved.prefix && (
            <span className="text-ink-faint text-[11px] mr-1 uppercase tracking-[0.06em]">
              {resolved.prefix}
            </span>
          )}
          {resolved.text}
        </span>
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
            className="opacity-0 group-hover/row:opacity-100 focus:opacity-100 text-ink-faint hover:text-ink transition-opacity flex-shrink-0"
          >
            <EyeOff className="w-3 h-3" />
          </span>
        )}
      </div>
      <div className="flex items-center gap-3 font-mono text-[11px] text-ink-faint">
        <code className="tabular-nums">{trace.traceId.slice(0, 8)}</code>
        <span className="flex items-center gap-1 tabular-nums">
          <Timer className="w-2.5 h-2.5" />
          {trace.status === 'pending' && trace.duration === undefined
            ? 'running…'
            : formatDuration(trace.duration ?? 0)}
        </span>
        <span className="flex items-center gap-1">
          <Zap className="w-2.5 h-2.5" />
          {trace.workers.join(', ')}
        </span>
        <span className="ml-auto tabular-nums">
          {formatTime(trace.startTime)}
        </span>
      </div>
    </button>
  )
}

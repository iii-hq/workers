import { Timer, Zap } from 'lucide-react'
import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import type { TraceListItem } from '../hooks/useTraceData'
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
}

export function TraceListRow({
  trace,
  isSelected,
  isNew,
  onSelect,
  onAnimationEnd,
}: TraceListRowProps) {
  return (
    <button
      type="button"
      onClick={onSelect}
      onAnimationEnd={onAnimationEnd}
      className={cn(
        'w-full px-4 py-3 border-b border-rule-2 text-left transition-colors',
        isSelected ? 'bg-panel border-l-2 border-l-accent' : 'hover:bg-panel',
        isNew && 'trace-flash',
      )}
    >
      <div className="flex items-center gap-2 mb-1">
        <StatusDot
          tone={statusDotTone(trace.status)}
          pulse={trace.status === 'pending'}
        />
        <span className="font-mono text-[13px] text-ink truncate flex-1 lowercase">
          {trace.topic ? (
            <>
              <span className="text-ink-faint text-[11px] mr-1 uppercase tracking-[0.06em]">
                enqueue:
              </span>
              {trace.topic}
            </>
          ) : (
            (trace.functionId ?? trace.rootOperation)
          )}
        </span>
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
          {trace.services.join(', ')}
        </span>
        <span className="ml-auto tabular-nums">
          {formatTime(trace.startTime)}
        </span>
      </div>
    </button>
  )
}

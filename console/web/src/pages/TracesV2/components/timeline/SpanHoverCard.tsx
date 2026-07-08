/**
 * Hover detail card for timeline spans — shared by the live Timeline strip
 * and the static TraceTimeline. Schematic panel styling lifted from the
 * flame-graph tooltip: `bg-panel`, 1px rule border, no shadow, no rounding.
 * Rendered `fixed` at the cursor (clamped to the viewport edges) and
 * `pointer-events-none` so it never steals the hover.
 */

import { StatusDot } from '@/components/ui/StatusDot'
import { cn } from '@/lib/utils'
import {
  formatDuration,
  formatRelative,
  formatTimestamp,
} from '../../lib/traceUtils'
import { effectiveEnd, type TimelineSpan } from './layout'

const CARD_MAX_WIDTH = 280
const CARD_CLEARANCE_Y = 170

function statusTone(
  status: TimelineSpan['status'],
): 'accent' | 'alert' | 'warn' {
  if (status === 'error') return 'alert'
  if (status === 'ok') return 'accent'
  return 'warn'
}

const STATUS_TEXT: Record<'accent' | 'alert' | 'warn', string> = {
  accent: 'text-accent',
  alert: 'text-alert',
  warn: 'text-warn',
}

export interface SpanHoverCardProps {
  span: TimelineSpan
  /** wall-clock "now" used to size still-running spans */
  now: number
  /** viewport coords, typically the mouse position */
  x: number
  y: number
  /**
   * When true, `startTime` is an offset from the trace start (the static
   * TraceTimeline synthesizes offsets from 0) and renders as `+12.0ms`
   * instead of a wall-clock timestamp.
   */
  relativeStart?: boolean
  /** optional share of the whole trace, 0..100 (detail timeline) */
  tracePercent?: number
}

export function SpanHoverCard({
  span,
  now,
  x,
  y,
  relativeStart,
  tracePercent,
}: SpanHoverCardProps) {
  const running = span.endTime == null
  const duration = effectiveEnd(span, now) - span.startTime
  const status = span.status ?? (running ? 'pending' : 'ok')
  const tone = statusTone(status)
  const statusLabel = running ? 'running' : status

  return (
    <div
      className="fixed z-50 pointer-events-none"
      style={{
        left: Math.min(x + 12, window.innerWidth - CARD_MAX_WIDTH),
        top: Math.min(y + 12, window.innerHeight - CARD_CLEARANCE_Y),
      }}
    >
      <div
        className="bg-panel border border-rule px-3 py-2.5 min-w-[200px]"
        style={{ maxWidth: CARD_MAX_WIDTH }}
      >
        <div className="font-mono text-[12.5px] text-ink leading-tight mb-1.5 break-all lowercase">
          {span.label ?? span.id}
        </div>

        {span.meta ? (
          <div className="flex items-center gap-1.5 mb-2">
            <StatusDot tone={tone} pulse={running} />
            <span className="text-[10px] text-ink-faint font-mono lowercase truncate">
              {span.meta}
            </span>
          </div>
        ) : null}

        <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-[10px] font-mono">
          <div className="flex justify-between gap-2">
            <span className="text-ink-faint lowercase">duration</span>
            <span className="text-accent tabular-nums">
              {formatDuration(duration)}
            </span>
          </div>
          <div className="flex justify-between gap-2">
            <span className="text-ink-faint lowercase">start</span>
            <span className="text-ink tabular-nums">
              {relativeStart
                ? formatRelative(span.startTime)
                : formatTimestamp(span.startTime)}
            </span>
          </div>
          <div className="flex justify-between gap-2">
            <span className="text-ink-faint lowercase">status</span>
            <span className={cn('lowercase', STATUS_TEXT[tone])}>
              {statusLabel}
            </span>
          </div>
          {tracePercent != null && (
            <div className="flex justify-between gap-2">
              <span className="text-ink-faint lowercase">% trace</span>
              <span className="text-ink tabular-nums">
                {tracePercent.toFixed(1)}%
              </span>
            </div>
          )}
        </div>

        {tracePercent != null && (
          <div className="mt-2 h-1 bg-rule-2 overflow-hidden">
            <div
              className="h-full bg-accent"
              style={{
                width: `${Math.max(1, tracePercent)}%`,
                opacity: 0.7,
              }}
            />
          </div>
        )}
      </div>
    </div>
  )
}

/**
 * Hover detail card for timeline spans — shared by the live Timeline strip,
 * the static TraceTimeline and the waterfall. A `FactCard` rendered `fixed`
 * at the cursor (clamped to the viewport edges) and `pointer-events-none`
 * so it never steals the hover. It is the "tooltip" for a bar whose label
 * is clipped: the full name, the worker, the timing.
 *
 * The card carries the bar's own colour — the worker hue (alert on error)
 * tints the kind-icon chip and fills the share-of-trace meter — so the
 * card and the bar under the pointer read as one thing.
 */

import {
  Fact,
  FactCard,
  FactCardHeader,
  FactCardStatus,
  FactList,
  FactMeter,
  type FactTone,
} from '@/components/ui/FactCard'
import {
  formatDuration,
  formatRelative,
  formatTimestamp,
} from '../../lib/traceUtils'
import { effectiveEnd, type TimelineSpan } from './layout'
import { KIND_ICONS, resolveColor } from './spanVisuals'

const CARD_MAX_WIDTH = 300
const CARD_CLEARANCE_Y = 170

function toneOf(status: TimelineSpan['status'], running: boolean): FactTone {
  if (status === 'error') return 'alert'
  if (running || status === 'pending') return 'pending'
  if (status === 'ok') return 'ok'
  return 'unset'
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
  const tone = toneOf(status, running)
  const statusLabel = running ? 'running' : status
  const accent = resolveColor(span)

  return (
    <div
      className="pointer-events-none fixed z-50"
      role="tooltip"
      style={{
        left: Math.min(x + 12, window.innerWidth - CARD_MAX_WIDTH - 8),
        top: Math.min(y + 12, window.innerHeight - CARD_CLEARANCE_Y),
      }}
    >
      <FactCard
        accent={accent}
        className="min-w-[220px]"
        style={{ maxWidth: CARD_MAX_WIDTH }}
      >
        <FactCardHeader
          icon={KIND_ICONS[span.kind ?? 'zap']}
          title={span.label ?? span.id}
          mono
        />
        <FactCardStatus tone={tone} label={statusLabel} meta={span.meta} />
        <FactList>
          <Fact label="Duration" mono align="end">
            {formatDuration(duration)}
          </Fact>
          <Fact label="Start" mono align="end">
            {relativeStart
              ? formatRelative(span.startTime)
              : formatTimestamp(span.startTime)}
          </Fact>
          {tracePercent != null ? (
            <Fact label="Of trace" mono align="end">
              {tracePercent.toFixed(1)}%
            </Fact>
          ) : null}
        </FactList>
        {tracePercent != null ? (
          <FactMeter
            value={tracePercent}
            label="share of the trace"
            pending={running}
          />
        ) : null}
      </FactCard>
    </div>
  )
}

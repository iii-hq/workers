/**
 * The TracesV2 masthead: the live Timeline wearing the page-header chrome.
 * Replaces the old `$ traces` + h1 header block — the strip IS the title
 * row now. One bar per trace (max 4 lanes), sliding through a 60s window;
 * hovering a bar shows the trace's details, clicking it opens the trace.
 *
 * A trace row is its ROOT span, which for queue-triggered traces is the
 * instant "publish" moment — alone it renders as a dot at the trace's
 * birth. The optional `activity` map (last span-close activity per trace,
 * from the engine `trace` trigger) corrects that: traces with fresh
 * activity render as LIVE bars growing along the now-edge, and settle at
 * their last activity once quiet. The strip re-evaluates liveness on a
 * coarse tick while anything is live, so settled bars park the view again.
 *
 * Chrome sits in a header row above the visualization (solid `bg-bg`,
 * bounded by 1px rules): the eyebrow + paused badge on the left, the
 * system / pause / refresh actions on the right.
 */

import { Eye, EyeOff, Pause, Play, RefreshCw } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { cn } from '@/lib/utils'
import type { TraceActivityMap } from '../../hooks/useTraceActivity'
import type { TraceListItem } from '../../hooks/useTraceData'
import { traceListToTimelineSpans } from '../../lib/timelineSpans'
import { Timeline } from './Timeline'

export interface TimelineStripProps {
  traces: readonly TraceListItem[]
  isPaused: boolean
  showSystem: boolean
  isLoading?: boolean
  onTogglePause: () => void
  onToggleSystem: () => void
  onRefresh: () => void
  /** clicking a trace bar opens that trace's detail */
  onTraceClick?: (traceId: string) => void
  selectedTraceId?: string | null
  /** per-trace last span-close activity (wall-clock ms) — keeps bars live */
  activity?: TraceActivityMap
  /** visible window, ms (default 60s) */
  windowMs?: number
  className?: string
}

const PRUNE_SLACK_MS = 15_000
/** cadence of the liveness re-evaluation while any bar is live */
const LIVENESS_TICK_MS = 500

export function TimelineStrip({
  traces,
  isPaused,
  showSystem,
  isLoading,
  onTogglePause,
  onToggleSystem,
  onRefresh,
  onTraceClick,
  selectedTraceId,
  activity,
  windowMs = 60_000,
  className,
}: TimelineStripProps) {
  // Liveness evaluation instant. Only re-ticked while something is live —
  // an idle strip renders exactly as often as its data changes. A stale
  // `now` between ticks can only UNDER-decay (fresh activity still reads
  // as live), never invent liveness.
  const [now, setNow] = useState(() => Date.now())

  const spans = useMemo(() => {
    const all = traceListToTimelineSpans(
      traces,
      activity ? { activity, now } : undefined,
    )
    // Keep one window behind the NEWEST effective end so lane assignment
    // stays cheap at the 500-row list ceiling. The cutoff keys off the
    // data, not the wall clock: when the engine is idle the Timeline parks
    // at the last span's end, and a wall-clock cutoff would filter that
    // history out from under the frozen view. Live spans count as `now`.
    let newestEnd = Number.NEGATIVE_INFINITY
    for (const s of all) {
      const end = s.endTime ?? Math.max(now, s.startTime)
      if (end > newestEnd) newestEnd = end
    }
    if (newestEnd === Number.NEGATIVE_INFINITY) return all
    const cutoff = newestEnd - windowMs - PRUNE_SLACK_MS
    return all.filter((s) => (s.endTime ?? newestEnd) >= cutoff)
  }, [traces, activity, now, windowMs])

  // While any bar is live, tick the evaluation clock so quiet traces settle
  // (live → fixed end) and the view can park again.
  const anyLive = useMemo(() => spans.some((s) => s.endTime == null), [spans])
  useEffect(() => {
    if (!anyLive) return
    const id = setInterval(() => setNow(Date.now()), LIVENESS_TICK_MS)
    return () => clearInterval(id)
  }, [anyLive])

  return (
    <div
      className={cn(
        'flex h-36 flex-shrink-0 flex-col border-b border-rule overflow-hidden',
        className,
      )}
    >
      <div className="flex shrink-0 items-center justify-between border-b border-rule bg-bg">
        <div className="flex items-center gap-3 px-3 py-2">
          <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
            <span className="text-accent">$</span>
            <span className="text-ink ml-2">traces</span>
          </div>
          {isPaused ? (
            <Badge variant="warn">
              <Pause className="w-3 h-3" />
              paused
            </Badge>
          ) : (
            <span className="flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-ghost">
              <span
                aria-hidden
                className="inline-block size-1.5 rounded-full bg-accent pulse-dot"
              />
              live
            </span>
          )}
        </div>

        <div className="flex items-center gap-1 px-2 py-1">
          <Button
            variant={showSystem ? 'pill' : 'ghost'}
            size="sm"
            onClick={onToggleSystem}
          >
            {showSystem ? (
              <Eye className="w-3.5 h-3.5" />
            ) : (
              <EyeOff className="w-3.5 h-3.5" />
            )}
            <span className={cn(showSystem ? '' : 'line-through opacity-60')}>
              system
            </span>
          </Button>
          <Button
            variant={isPaused ? 'pill' : 'ghost'}
            size="sm"
            onClick={onTogglePause}
          >
            {isPaused ? (
              <Play className="w-3.5 h-3.5" />
            ) : (
              <Pause className="w-3.5 h-3.5" />
            )}
            <span>{isPaused ? 'resume' : 'pause'}</span>
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={onRefresh}
            disabled={isLoading}
          >
            <RefreshCw
              className={cn('w-3.5 h-3.5', isLoading && 'animate-spin')}
            />
            <span>refresh</span>
          </Button>
        </div>
      </div>

      <Timeline
        className="min-h-0 flex-1"
        spans={spans}
        windowMs={windowMs}
        maxLanes={4}
        onSpanClick={onTraceClick ? (span) => onTraceClick(span.id) : undefined}
        selectedSpanId={selectedTraceId ?? undefined}
      />
    </div>
  )
}

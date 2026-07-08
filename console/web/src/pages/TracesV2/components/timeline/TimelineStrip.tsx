/**
 * The TracesV2 masthead: the live Timeline wearing the page-header chrome.
 * Replaces the old `$ traces` + h1 header block — the strip IS the title
 * row now. One bar per SPAN across all traces (max 4 lanes, overflow fans
 * into chip stacks), sliding through a 60s window; hovering a bar shows the
 * span's details, clicking it opens its owning trace.
 *
 * Bars come straight from the all-spans feed (`useAllSpans`: one seed read
 * + the engine's `iii:devtools:all-spans` stream): a pending span renders
 * as a LIVE bar growing along the now-edge and settles when its close frame
 * arrives, so liveness is span-accurate — no trace-level correction needed.
 * Engine routing wrappers are skipped (see `storedSpansToTimelineSpans`).
 *
 * Which bars are VISIBLE is the same hidden span-group / worker selection
 * the detail views use (`lib/spanFilters.ts`, one shared `spanFilter`
 * instance per page) — the funnel menu in the header row lists the strip's
 * current function families and workers, and hiding one here hides it in
 * the waterfall too. On an all-spans strip that menu is the volume control:
 * hide the bookkeeping families and the strip reads as real work.
 *
 * Chrome sits in a header row above the visualization (solid `bg-bg`,
 * bounded by 1px rules): the eyebrow + paused badge on the left, the
 * span-filter funnel on the right.
 */

import { Pause } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { cn } from '@/lib/utils'
import type { StoredSpan } from '../../api/traces'
import type { SpanFilterControls } from '../../lib/spanFilters'
import { storedSpansToTimelineSpans } from '../../lib/timelineSpans'
import type { TimelineSpan } from './layout'
import { SpanFilterMenu } from './SpanFilterMenu'
import { deriveSpanGroups } from './spanVisibility'
import { Timeline } from './Timeline'

export interface TimelineStripProps {
  /** every span the strip may show (`useAllSpans` seed + live stream) */
  spans: readonly StoredSpan[]
  isPaused: boolean
  /** clicking a bar opens its owning trace's detail */
  onTraceClick?: (traceId: string) => void
  selectedTraceId?: string | null
  /** the page's shared span-filter selection (`useSpanFilterSelection`);
   *  the funnel menu is hidden when omitted and every bar shows */
  spanFilter?: SpanFilterControls
  /** visible window, ms (default 60s) */
  windowMs?: number
  className?: string
}

const PRUNE_SLACK_MS = 15_000
/** cadence of the liveness re-evaluation while any bar is live */
const LIVENESS_TICK_MS = 500

export function TimelineStrip({
  spans: storedSpans,
  isPaused,
  onTraceClick,
  selectedTraceId,
  spanFilter,
  windowMs = 60_000,
  className,
}: TimelineStripProps) {
  // Prune-window evaluation instant. Only re-ticked while something is live —
  // an idle strip renders exactly as often as its data changes.
  const [now, setNow] = useState(() => Date.now())

  const allSpans = useMemo(() => {
    const all = storedSpansToTimelineSpans(storedSpans)
    // Keep one window behind the NEWEST effective end so lane assignment
    // stays cheap as history accumulates. The cutoff keys off the data, not
    // the wall clock: when the engine is idle the Timeline parks at the
    // last span's end, and a wall-clock cutoff would filter that history
    // out from under the frozen view. Live spans count as `now`.
    let newestEnd = Number.NEGATIVE_INFINITY
    for (const s of all) {
      const end = s.endTime ?? Math.max(now, s.startTime)
      if (end > newestEnd) newestEnd = end
    }
    if (newestEnd === Number.NEGATIVE_INFINITY) return all
    const cutoff = newestEnd - windowMs - PRUNE_SLACK_MS
    return all.filter((s) => (s.endTime ?? newestEnd) >= cutoff)
  }, [storedSpans, now, windowMs])

  // The funnel menu lists the strip's CURRENT function families / workers
  // (hidden entries included, so they can be un-hidden), sharing the hidden
  // sets with the detail views.
  const groups = useMemo(
    () => deriveSpanGroups(allSpans, (s: TimelineSpan) => s.groupKey),
    [allSpans],
  )
  const workerGroups = useMemo(
    () => deriveSpanGroups(allSpans, (s: TimelineSpan) => s.workerKey),
    [allSpans],
  )

  const spans = useMemo(() => {
    if (
      !spanFilter ||
      (spanFilter.hiddenGroups.size === 0 &&
        spanFilter.hiddenWorkers.size === 0)
    ) {
      return allSpans
    }
    return allSpans.filter(
      (s) =>
        !(s.groupKey != null && spanFilter.hiddenGroups.has(s.groupKey)) &&
        !(s.workerKey != null && spanFilter.hiddenWorkers.has(s.workerKey)),
    )
  }, [allSpans, spanFilter])

  // While any bar is live, tick the evaluation clock so the prune window
  // keeps advancing with the now-edge; once everything settles the strip
  // stops ticking and the view parks.
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

        {spanFilter && (
          <div className="flex items-center px-2 py-1">
            <SpanFilterMenu
              groups={groups}
              workerGroups={workerGroups}
              hiddenKeys={spanFilter.hiddenGroups}
              hiddenWorkerKeys={spanFilter.hiddenWorkers}
              hiddenSpanCount={allSpans.length - spans.length}
              onToggle={spanFilter.toggleGroup}
              onToggleWorker={spanFilter.toggleWorker}
              onClear={spanFilter.clear}
            />
          </div>
        )}
      </div>

      <Timeline
        className="min-h-0 flex-1"
        spans={spans}
        windowMs={windowMs}
        maxLanes={4}
        onSpanClick={
          onTraceClick
            ? (span) => onTraceClick(span.traceId ?? span.id)
            : undefined
        }
        selectedSpanId={selectedTraceId ?? undefined}
      />
    </div>
  )
}

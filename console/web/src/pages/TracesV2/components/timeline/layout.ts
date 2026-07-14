/**
 * The shared span model + pure helpers for the timeline visualizations.
 * `TimelineSpan` is the input shape of every timeline surface (the masthead
 * `TimelineStrip`, the detail `TraceTimeline`, the hover card, and the
 * waterfall's filter plumbing). Each surface packs its own geometry — the
 * strip's hierarchical line layout lives in `TimelineStrip.tsx`,
 * `TraceTimeline` packs flame-graph rows — what they share is the span
 * shape, the live-end convention, and the window/tick math.
 */

export type TimelineSpanKind = 'zap' | 'sparkle' | 'flame' | 'lambda'

export interface TimelineSpan {
  id: string
  /** owning trace, when it differs from `id` — click and selection
   *  resolve through it when present */
  traceId?: string
  /** the span that triggered this one — the nearest NON-routing ancestor
   *  (engine wrappers are skipped at mapping time). May reference a span
   *  that isn't in the current set (not arrived / pruned): consumers must
   *  treat unresolvable parents as roots. Drives the strip's hierarchy. */
  parentId?: string
  /** shared span-filter keys (`lib/spanFilters.ts` selection): the owning
   *  function id and worker name — bars matching a hidden entry are
   *  filtered out by the strip before layout */
  groupKey?: string
  workerKey?: string
  /** internal-span family (`iii.tag.hidden` — call-site tagged plumbing);
   *  bars carrying one live in the funnel's separate "internal" section
   *  and are hidden unless their family was explicitly shown */
  internalKey?: string
  /** epoch ms */
  startTime: number
  /** epoch ms; null/undefined while the span is still running */
  endTime?: number | null
  /** icon shown at the left of a detail-view bar; defaults to 'zap' */
  kind?: TimelineSpanKind
  /** bar color override; defaults to a schematic ink shade (alert on error) */
  color?: string
  status?: 'ok' | 'error' | 'pending' | 'unset'
  label?: string
  /** hover-card subtitle (worker name, trace id, …) */
  meta?: string
}

/** detail-view bar geometry (`TraceTimeline`); the strip draws its own
 *  3px lines */
export const BAR_HEIGHT = 16
export const MIN_BAR_WIDTH = 18

export function effectiveEnd(span: TimelineSpan, now: number): number {
  return span.endTime ?? now
}

/**
 * Wall-clock tick boundaries covering the window, plus one boundary past
 * each edge so ticks slide in from off-screen instead of popping.
 */
export function computeTicks(
  now: number,
  windowMs: number,
  tickMs: number,
): number[] {
  const first = Math.floor((now - windowMs) / tickMs) * tickMs
  const last = Math.ceil((now + tickMs) / tickMs) * tickMs
  const ticks: number[] = []
  for (let t = first; t <= last; t += tickMs) ticks.push(t)
  return ticks
}

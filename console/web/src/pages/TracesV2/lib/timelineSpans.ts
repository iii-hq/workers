/**
 * Pure mappers from the trace domain models onto `TimelineSpan` — the input
 * shape of the timeline visualizations.
 *
 * Two sources feed a timeline:
 * - the flat trace LIST (`TraceListItem[]`) feeds the live top strip: one
 *   bar per trace, live-growing while the trace is pending;
 * - a trace DETAIL (`WaterfallData`) feeds the static per-trace timeline:
 *   one bar per span, with ms offsets synthesized from the waterfall
 *   percentages so the window is exactly [0, total_duration_ms].
 *
 * The list rows alone under-describe a trace: a row is the trace's ROOT
 * span, exported when it CLOSES — for queue-triggered traces that is the
 * instant "publish" moment, while the real work continues in child spans
 * for seconds after. Without correction every such trace renders as a dot
 * pinned at its start. `TraceLiveness` (per-trace last span-close activity
 * from the engine's `trace` trigger) fills the gap: recent activity beyond
 * the root's own end keeps the bar LIVE and growing; once the trace goes
 * quiet the bar settles at the last activity time (≈ the real end, within
 * one engine coalesce window).
 */

import type {
  TimelineSpan,
  TimelineSpanKind,
} from '../components/timeline/layout'
import type { TraceActivityMap } from '../hooks/useTraceActivity'
import type { TraceListItem } from '../hooks/useTraceData'
import { formatSpanLabel } from './spanLabel'
import { getWorkerColor } from './traceColors'
import type { VisualizationSpan, WaterfallData } from './traceTransform'
import { getWorkerName } from './traceUtils'

/**
 * Icon vocabulary:
 * - `zap`      — inbound work: server roots / enqueue topics
 * - `lambda`   — worker function invocations
 * - `sparkle`  — outbound client calls (llm, db, http)
 * - `flame`    — queue consumers/producers
 */
function kindForListItem(item: TraceListItem): TimelineSpanKind {
  if (item.topic) return 'zap'
  if (item.functionId) return 'lambda'
  return 'sparkle'
}

function kindForOtelSpan(span: VisualizationSpan): TimelineSpanKind {
  switch (span.kind?.toLowerCase()) {
    case 'server':
      return 'zap'
    case 'client':
      return 'sparkle'
    case 'consumer':
    case 'producer':
      return 'flame'
    default:
      return 'lambda'
  }
}

/**
 * Activity newer than this (vs. `now`) keeps a trace's bar live and growing;
 * older activity settles the bar at the last activity time. Sized to ride
 * out multi-second gaps between span closes (an LLM call between two tool
 * spans) without parking the bar mid-trace.
 */
export const TRACE_ACTIVITY_IDLE_MS = 2_000

/**
 * Activity within this of the row's own end is the row's own close echoing
 * back through the trigger (engine coalesce ≤300ms + delivery), not evidence
 * of work beyond the root. Without this dead-band every single-span trace
 * would go "live" for a moment just because its own arrival fired the
 * activity trigger.
 */
export const TRACE_ACTIVITY_ECHO_MS = 1_000

export interface TraceLiveness {
  /** per-trace wall-clock ms of the most recent span-close activity */
  activity: TraceActivityMap
  /** evaluation instant — a parameter so the mapping stays pure/testable */
  now: number
}

/**
 * Last-activity timestamp for a trace, but only once it clears the echo
 * dead-band past the row's own end — i.e. evidence of work beyond the root,
 * not the root's own arrival echoing back through the trigger. `null` when
 * there's no such evidence.
 */
function activityBeyondRoot(
  rootEnd: number,
  liveness: TraceLiveness,
  traceId: string,
): number | null {
  const lastActivity = liveness.activity.get(traceId)
  if (
    lastActivity == null ||
    lastActivity - rootEnd <= TRACE_ACTIVITY_ECHO_MS
  ) {
    return null
  }
  return lastActivity
}

/**
 * Whether a trace row should read as still doing work: its root span hasn't
 * closed yet, or it has, but the engine's per-trace activity signal shows a
 * span closing recently enough (beyond the root's own end, within the idle
 * window) that children are still running. Shared by the timeline strip
 * (keeps the bar live/growing) and the trace list (pulses the status dot) so
 * both surfaces agree on one definition of "live".
 */
export function isTraceLive(
  item: Pick<TraceListItem, 'status' | 'endTime' | 'traceId'>,
  liveness: TraceLiveness,
): boolean {
  if (item.status === 'pending' || item.endTime == null) return true
  const lastActivity = activityBeyondRoot(item.endTime, liveness, item.traceId)
  return (
    lastActivity != null && liveness.now - lastActivity < TRACE_ACTIVITY_IDLE_MS
  )
}

/**
 * One timeline bar per trace row.
 *
 * With `liveness`, a row whose trace shows span-close activity beyond its
 * own end is corrected from "instant root dot" to the trace's real extent:
 * `endTime: null` (live, growing) while activity is fresh, settled at the
 * last activity once the trace goes quiet. Live bars report `pending`
 * status (unless the row already errored) so hover/aria say "running".
 */
export function traceListToTimelineSpans(
  items: readonly TraceListItem[],
  liveness?: TraceLiveness,
): TimelineSpan[] {
  const spans: TimelineSpan[] = []
  for (const item of items) {
    let running = item.status === 'pending' || item.endTime == null
    let end = running ? null : (item.endTime ?? null)

    if (!running && liveness) {
      const lastActivity = activityBeyondRoot(end ?? 0, liveness, item.traceId)
      if (lastActivity != null) {
        if (liveness.now - lastActivity < TRACE_ACTIVITY_IDLE_MS) {
          running = true
          end = null
        } else {
          end = Math.max(end ?? 0, lastActivity)
        }
      }
    }

    spans.push({
      id: item.traceId,
      startTime: item.startTime,
      endTime: end,
      status: running && item.status !== 'error' ? 'pending' : item.status,
      kind: kindForListItem(item),
      label: item.functionId ?? item.topic ?? item.rootOperation,
      meta: `${item.workers.join(', ')} · ${item.traceId.slice(0, 8)}`,
    })
  }
  return spans
}

export interface TraceDetailTimeline {
  /** bars with startTime as an OFFSET from the trace start (ms) */
  spans: TimelineSpan[]
  /** the fixed window the static timeline should render, ms */
  totalDurationMs: number
  /** back-map for hover/click: timeline span id → source span */
  byId: Map<string, VisualizationSpan>
}

/**
 * One timeline bar per span of a trace. Offsets are synthesized from the
 * waterfall percentages, so `startTime` is relative to the trace start
 * (0 .. total_duration_ms) rather than a wall-clock epoch.
 */
export function waterfallToTimelineSpans(
  data: WaterfallData,
): TraceDetailTimeline {
  const total = data.total_duration_ms
  const byId = new Map<string, VisualizationSpan>()
  const spans: TimelineSpan[] = data.spans.map((span) => {
    byId.set(span.span_id, span)
    const start = (span.start_percent / 100) * total
    return {
      id: span.span_id,
      startTime: start,
      // Offsets here are trace-relative, so a live span keeps its numeric
      // elapsed-so-far end (recomputed on each rebuild) rather than the
      // wall-clock `endTime: null` convention the list strip uses.
      endTime: start + span.duration_ms,
      status: span.pending && span.status !== 'error' ? 'pending' : span.status,
      kind: kindForOtelSpan(span),
      color: getWorkerColor(getWorkerName(span)),
      // Verb prefixes (`execute `, `call `, ...) are stripped for display —
      // the bar/hover label reads as the function, not the SDK span name.
      label: formatSpanLabel(span),
      meta: getWorkerName(span),
    }
  })
  return { spans, totalDurationMs: total, byId }
}

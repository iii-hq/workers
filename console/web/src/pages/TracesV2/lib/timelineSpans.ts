/**
 * Pure mappers from the trace domain models onto `TimelineSpan` — the input
 * shape of the timeline visualizations.
 *
 * Two sources feed a timeline:
 * - flat stored spans (`StoredSpan[]`, from the all-spans seed + stream)
 *   feed the live top strip: one bar per SPAN across all traces, live while
 *   the span is pending;
 * - a trace DETAIL (`WaterfallData`) feeds the static per-trace timeline:
 *   one bar per span, with ms offsets synthesized from the waterfall
 *   percentages so the window is exactly [0, total_duration_ms].
 *
 * `isTraceLive` (per-trace last span-close activity from the engine's
 * `trace` trigger) also lives here: the trace LIST's rows are root spans
 * that close instantly for queue-triggered traces, so the rows' pulsing
 * status dot needs the activity signal to know the trace is still working.
 */

import type { StoredSpan } from '../api/traces'
import type {
  TimelineSpan,
  TimelineSpanKind,
} from '../components/timeline/layout'
import type { TraceActivityMap } from '../hooks/useTraceActivity'
import type { TraceListItem } from '../hooks/useTraceData'
import { explicitFunctionId } from './functionCallFromSpan'
import {
  inheritedTags,
  internalFamilyOf,
  isEngineRoutingSpan,
  resolveSpanLabel,
  tagKindOf,
  tagRootKind,
} from './spanLabel'
import { getWorkerColor } from './traceColors'
import { normalizeSpanAttributes } from './traceListItem'
import {
  isPendingSpan,
  toMs,
  type VisualizationSpan,
  type WaterfallData,
} from './traceTransform'
import { getWorkerName } from './traceUtils'

/**
 * Icon vocabulary:
 * - `zap`      — inbound work: server roots / enqueue topics
 * - `lambda`   — worker function invocations
 * - `sparkle`  — outbound client calls (llm, db, http)
 * - `flame`    — queue consumers/producers
 *
 * A producer can steer this classification directly via the `iii.tag.kind`
 * baggage tag (see `spanLabel.ts#tagKindOf` and
 * workers/console/docs/timeline-span-tags.md): `harness.turn`/
 * `harness.subagent` read as `lambda` (a function invocation), and
 * `queue.process` reads as `flame` (the existing "queue consumers/producers"
 * bucket) — checked before falling back to the raw OTel `SpanKind`.
 */
function kindForOtelSpan(
  span: Pick<VisualizationSpan, 'name' | 'service_name'> & {
    kind?: string
    attributes?: Record<string, unknown>
  },
): TimelineSpanKind {
  const tag = tagKindOf(span)
  if (tag === 'queue.process') return 'flame'
  if (tag?.startsWith('harness.')) return 'lambda'
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

/** The waterfall's span-group identity, mirroring
 *  `traceTimelineFilters.ts` so the strip's funnel entries line up with the
 *  detail views': explicit identity (attrs or `execute <fn>` name) first,
 *  then tag ROOTS under their own span name (a producer's scope span like
 *  `harness::turn step` is a first-class segment, not machinery of the
 *  function whose baggage it inherits), then the baggage-stamped
 *  `iii.function.id`, then the span name. */
function storedSpanGroupKey(
  span: StoredSpan,
  attrs: Record<string, unknown>,
  inheritedKind: string | undefined,
): string {
  const explicit = explicitFunctionId({ name: span.name, attributes: attrs })
  if (explicit) return explicit
  if (tagRootKind(attrs, inheritedKind)) return span.name
  const baggage = attrs['iii.function.id']
  if (typeof baggage === 'string' && baggage !== '') return baggage
  return span.name
}

interface ShapedStoredSpan {
  span: StoredSpan
  attrs: Record<string, unknown>
  shaped: {
    name: string
    service_name?: string
    attributes: Record<string, unknown>
  }
  routing: boolean
}

/**
 * The span that TRIGGERED this one, hopping over skipped routing wrappers:
 * walk the parent chain until it leaves the routing layer. An id that isn't
 * in the feed at all is kept as-is — the parent may simply not have arrived
 * yet (self-heals on the next mapping pass), and layouts already treat
 * unresolvable parents as roots.
 */
function resolveTriggerParent(
  span: StoredSpan,
  byId: ReadonlyMap<string, ShapedStoredSpan>,
): string | undefined {
  let pid = span.parent_span_id
  const seen = new Set<string>()
  while (pid && !seen.has(pid)) {
    const entry = byId.get(pid)
    if (!entry?.routing) return pid
    seen.add(pid)
    pid = entry.span.parent_span_id
  }
  return pid || undefined
}

/**
 * One timeline bar per stored span — the masthead's all-spans view.
 *
 * Engine ROUTING wrappers (`handle_invocation X` / `call X`) are skipped:
 * every invocation would otherwise stack three near-identical bars around
 * the worker's own `execute` span, and the detail views collapse those
 * wrappers too. Each bar's `parentId` is the nearest NON-routing ancestor
 * (the hierarchy stays connected across the skipped wrappers — this is the
 * V2 strip's "what triggered what" edge). Labels follow the waterfall's
 * rules (producer `iii.tag.display_name` override, else the verb-stripped
 * span name), a pending span is a LIVE bar (`endTime: null`, grows along
 * the now-edge), and every bar carries the shared filter keys so the funnel
 * menu can hide the same function families the waterfall hides.
 */
export function storedSpansToTimelineSpans(
  spans: readonly StoredSpan[],
): TimelineSpan[] {
  // Pass 1: shape + classify everything (routing wrappers included) so
  // parent chains can be walked across the spans this function skips.
  // Keyed by span_id — the feed already dedupes (`useAllSpans`).
  const byId = new Map<string, ShapedStoredSpan>()
  for (const span of spans) {
    const attrs = normalizeSpanAttributes(span.attributes)
    const shaped = {
      name: span.name,
      service_name: span.service_name,
      attributes: attrs,
    }
    byId.set(span.span_id, {
      span,
      attrs,
      shaped,
      routing: isEngineRoutingSpan(shaped),
    })
  }

  // Pass 2: emit bars for the non-routing spans, in feed order. Tag
  // identity (labels, tag-root grouping) compares against the tags
  // INHERITED from the ancestry, walked over pass 1's index.
  const lookupCarrier = (id: string) => {
    const entry = byId.get(id)
    return entry
      ? { attributes: entry.attrs, parent_span_id: entry.span.parent_span_id }
      : undefined
  }
  const out: TimelineSpan[] = []
  for (const { span, attrs, shaped, routing } of byId.values()) {
    if (routing) continue
    const pending = isPendingSpan(span)
    const worker = getWorkerName(shaped)
    const inherited = inheritedTags(span.parent_span_id, lookupCarrier)
    out.push({
      id: span.span_id,
      traceId: span.trace_id,
      parentId: resolveTriggerParent(span, byId),
      startTime: toMs(span.start_time_unix_nano),
      endTime: pending ? null : toMs(span.end_time_unix_nano),
      status:
        span.status.toLowerCase() === 'error'
          ? 'error'
          : pending
            ? 'pending'
            : 'ok',
      kind: kindForOtelSpan({ ...shaped, kind: span.kind }),
      label: resolveSpanLabel(shaped, inherited.displayName),
      meta: `${worker} · ${span.trace_id.slice(0, 8)}`,
      groupKey: storedSpanGroupKey(span, attrs, inherited.kind),
      workerKey: worker,
      internalKey: internalFamilyOf(attrs) ?? undefined,
    })
  }
  return out
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
  // Filled BEFORE the mapping pass: label resolution walks the ancestry
  // (display-name echo suppression), and feed order does not guarantee
  // parents precede children.
  const byId = new Map<string, VisualizationSpan>()
  for (const span of data.spans) byId.set(span.span_id, span)
  const lookupById = (id: string) => byId.get(id)
  const spans: TimelineSpan[] = data.spans.map((span) => {
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
      // the bar/hover label reads as the function, not the SDK span name —
      // unless a producer set an `iii.tag.display_name` override (applied
      // only where the name is new, not on baggage echoes).
      label: resolveSpanLabel(
        span,
        inheritedTags(span.parent_span_id, lookupById).displayName,
      ),
      meta: getWorkerName(span),
    }
  })
  return { spans, totalDurationMs: total, byId }
}

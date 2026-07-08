/**
 * Live span streams for the devtools Traces view — the real-time APPEND feed
 * that replaces the old `traces-live` (trace-trigger → debounce → refetch)
 * model. The engine observability worker pushes span data onto two ephemeral
 * iii streams via `stream::send`; the browser subscribes with scoped
 * `type:'stream'` triggers and merges pushed spans into state, the same
 * "engine pushes data, client appends" pattern as `lib/sessions/events.ts`.
 *
 *   - `iii:devtools:trace-rows` / group `all`     → root rows for the LIST
 *     (one global firehose; every list view joins the single `all` group).
 *   - `iii:devtools:trace-spans` / group `<traceId>` → every span of one trace
 *     for the DETAIL waterfall (joined only for the selected trace).
 *
 * A third, non-stream feed rides the engine's `type:'trace'` trigger
 * (`startTraceActivityFeed`): `{ trace_ids }` for every ~300ms window in which
 * any non-internal span of those traces CLOSED. The rows stream only carries
 * ROOT spans — for queue-triggered traces the root is the instant publish
 * span, so the row alone says nothing about the seconds of child work that
 * follow. The activity feed is that missing liveness signal: the timeline
 * strip uses it to keep a trace's bar growing while its spans are still
 * closing and to settle the bar once the trace goes quiet.
 *
 * The in-memory trace store stays the source of truth: each surface does ONE
 * seed read, then appends from its stream. A dropped broadcast frame is not
 * fatal — a single re-seed on reconnect self-heals (the caller wires that).
 *
 * Handlers are named with the `iii::` prefix (`is_iii_builtin_function_id`), so
 * the spans produced by DELIVERING a frame are `iii.function.kind=internal` and
 * are excluded by the engine's stream loop-break (`is_trace_stream_delivery`),
 * so deliveries never re-enter the trace feed. The trace trigger has its own
 * loop-break: spans produced by delivering to a registered trace-trigger
 * function are dropped before they can re-fire it.
 */

import type { IiiClient } from '@/lib/iii-client'
import type { StoredSpan, TracesFilterParams } from '@/pages/Traces/api/traces'

/** iii:: prefix → engine-internal → delivery spans hidden + stream loop-break. */
const TRACE_ROWS_FN = 'iii::console::trace_rows'
const TRACE_SPANS_FN = 'iii::console::trace_spans'
const ALL_SPANS_FN = 'iii::console::all_spans'
const TRACE_ACTIVITY_FN = 'iii::console::trace_activity'
/** Stream names the observability worker pushes onto (mirror the engine consts). */
const TRACE_ROWS_STREAM = 'iii:devtools:trace-rows'
const TRACE_SPANS_STREAM = 'iii:devtools:trace-spans'
const ALL_SPANS_STREAM = 'iii:devtools:all-spans'
/** The single group every list subscriber joins (the list is a global firehose). */
const TRACE_ROWS_GROUP = 'all'
/** The engine observability worker's span-activity trigger type. */
const TRACE_TRIGGER_TYPE = 'trace'

/** Dev-only stream diagnostics. Silent in production builds. */
function dlog(msg: string, data?: unknown): void {
  if (import.meta.env?.DEV) {
    console.debug(`[traces-stream] ${msg}`, data ?? '')
  }
}

/**
 * Pull the `{ spans: [...] }` payload out of a raw stream frame.
 *
 * A `stream::send` Event frame serializes as
 * `{ streamName, groupId, event: { type: 'event', event: { type, data } } }`
 * — the payload is nested one level deeper than the `Create`/`Update` shape
 * (`{ event: { type, data } }`) that `set`/`update` produce. We read the Event
 * shape first and fall back to the flat-`data` shape so the extractor is
 * resilient to either producer.
 */
export function extractStreamSpans(frame: unknown): StoredSpan[] {
  if (!frame || typeof frame !== 'object') return []
  const obj = frame as Record<string, unknown>

  const outer =
    obj.event && typeof obj.event === 'object'
      ? (obj.event as Record<string, unknown>)
      : null
  if (!outer) return []

  // Event variant: outer = { type: 'event', event: { type, data } }.
  // Create/Update variant: outer = { type: 'create', data }.
  const inner =
    outer.event && typeof outer.event === 'object'
      ? (outer.event as Record<string, unknown>)
      : outer
  const data = 'data' in inner ? inner.data : null
  if (!data || typeof data !== 'object') return []

  const spans = (data as Record<string, unknown>).spans
  return Array.isArray(spans) ? (spans as StoredSpan[]) : []
}

/**
 * Merge a batch of streamed root spans into the existing list of trace rows,
 * keyed by `trace_id` (one row per trace). The newest representative wins:
 * a root span is preferred over a non-root, and among equals the later start
 * time wins. The result is sorted newest-first and capped — a live session
 * would otherwise grow the list without bound.
 *
 * Pure and immutable: returns a fresh array; never mutates the inputs.
 */
export function mergeTraceListSpans(
  existing: ReadonlyArray<StoredSpan>,
  incoming: ReadonlyArray<StoredSpan>,
  cap: number,
): StoredSpan[] {
  const byTrace = new Map<string, StoredSpan>()
  for (const span of existing) byTrace.set(span.trace_id, span)
  for (const span of incoming) {
    const current = byTrace.get(span.trace_id)
    if (!current) {
      byTrace.set(span.trace_id, span)
      continue
    }
    // Streamed spans never carry `trace_tags` (only `traces::list` merges
    // them) — a replacement must not erase tags the seeded row already has.
    const withTags = (s: StoredSpan): StoredSpan =>
      s.trace_tags || !current.trace_tags
        ? s
        : { ...s, trace_tags: current.trace_tags }
    const spanIsRoot = !span.parent_span_id
    const currentIsRoot = !current.parent_span_id
    if (spanIsRoot && !currentIsRoot) {
      byTrace.set(span.trace_id, withTags(span))
    } else if (
      spanIsRoot === currentIsRoot &&
      span.start_time_unix_nano >= current.start_time_unix_nano
    ) {
      byTrace.set(span.trace_id, withTags(span))
    }
  }
  const spans = [...byTrace.values()].sort(
    (a, b) => b.start_time_unix_nano - a.start_time_unix_nano,
  )
  return spans.length > cap ? spans.slice(0, cap) : spans
}

/**
 * Whether the list can be live-APPENDED from the stream, vs. needing a refetch.
 *
 * Append is only correct for the default "newest traces, unfiltered" view:
 * - No CONTENT filter (worker/name/status/duration/time/attributes/search) —
 *   a streamed row might not match it, so a filtered view must refetch.
 * - The default `start_time` / `desc` sort — append keeps the list newest-first,
 *   which only matches that ordering (a duration/asc sort + limit would be
 *   violated by prepending new rows). `sort_by`/`sort_order` are always present
 *   (the filter builder emits the defaults), so they're compared, not counted.
 */
export function isAppendableTraceList(
  filterParams: TracesFilterParams,
  search: string,
): boolean {
  if (search) return false
  const fp = filterParams
  if (fp.service_name || fp.name || fp.status) return false
  if (fp.attributes && fp.attributes.length > 0) return false
  if (fp.min_duration_ms != null || fp.max_duration_ms != null) return false
  if (fp.start_time != null || fp.end_time != null) return false
  if (fp.search_all_spans) return false
  if (fp.sort_by && fp.sort_by !== 'start_time') return false
  if (fp.sort_order && fp.sort_order !== 'desc') return false
  return true
}

/**
 * Subscribe the LIST to the global `trace-rows` stream. `onSpans` receives each
 * pushed batch of root spans (already extracted). Returns a cleanup that
 * unregisters the handler and the trigger.
 */
export function startTraceListStream(
  client: Pick<IiiClient, 'browserId' | 'on' | 'registerTrigger'>,
  onSpans: (spans: StoredSpan[]) => void,
): () => void {
  const off = client.on(TRACE_ROWS_FN, (frame: unknown) => {
    const spans = extractStreamSpans(frame)
    if (spans.length > 0) onSpans(spans)
  })

  // `on()` registers under `<fn>::<browserId>`; the trigger must target that id.
  const functionId = `${TRACE_ROWS_FN}::${client.browserId}`
  const offTrigger = client.registerTrigger({
    type: 'stream',
    function_id: functionId,
    config: { stream_name: TRACE_ROWS_STREAM, group_id: TRACE_ROWS_GROUP },
  })
  dlog('trace-rows stream subscribed', { functionId })

  return () => {
    off()
    try {
      offTrigger()
    } catch {
      // SDK already disposed; nothing to do.
    }
  }
}

/**
 * Subscribe the DETAIL to one trace's `trace-spans` stream (scoped by
 * `group_id = traceId`, so only this trace's span activity arrives). `onSpans`
 * receives each pushed batch of spans for the trace. Returns a cleanup that
 * unregisters the handler and the trigger — call it before subscribing to a
 * different trace.
 */
export function startTraceSpansStream(
  client: Pick<IiiClient, 'browserId' | 'on' | 'registerTrigger'>,
  traceId: string,
  onSpans: (spans: StoredSpan[]) => void,
): () => void {
  const off = client.on(TRACE_SPANS_FN, (frame: unknown) => {
    const spans = extractStreamSpans(frame)
    // The trigger is group-scoped, but guard against mis-delivery defensively.
    const scoped = spans.filter((s) => s.trace_id === traceId)
    if (scoped.length > 0) onSpans(scoped)
  })

  const functionId = `${TRACE_SPANS_FN}::${client.browserId}`
  const offTrigger = client.registerTrigger({
    type: 'stream',
    function_id: functionId,
    config: { stream_name: TRACE_SPANS_STREAM, group_id: traceId },
  })
  dlog('trace-spans stream subscribed', { functionId, traceId })

  return () => {
    off()
    try {
      offTrigger()
    } catch {
      // SDK already disposed; nothing to do.
    }
  }
}

/**
 * Subscribe to the global ALL-spans stream: every non-internal span of every
 * trace, pushed by the engine each coalesce window — the masthead strip's
 * per-span live feed. Same frame shape and lifecycle as the rows stream; one
 * global `all` group.
 */
export function startAllSpansFeed(
  client: Pick<IiiClient, 'browserId' | 'on' | 'registerTrigger'>,
  onSpans: (spans: StoredSpan[]) => void,
): () => void {
  const off = client.on(ALL_SPANS_FN, (frame: unknown) => {
    const spans = extractStreamSpans(frame)
    if (spans.length > 0) onSpans(spans)
  })

  const functionId = `${ALL_SPANS_FN}::${client.browserId}`
  const offTrigger = client.registerTrigger({
    type: 'stream',
    function_id: functionId,
    config: { stream_name: ALL_SPANS_STREAM, group_id: TRACE_ROWS_GROUP },
  })
  dlog('all-spans stream subscribed', { functionId })

  return () => {
    off()
    try {
      offTrigger()
    } catch {
      // SDK already disposed; nothing to do.
    }
  }
}

/**
 * Pull the `trace_ids` out of a `type:'trace'` trigger payload. The engine
 * delivers `{ trace_ids: string[] }` directly (a plain `engine.call`, not a
 * stream frame). Non-string entries and malformed payloads yield `[]`.
 */
export function extractTraceActivityIds(payload: unknown): string[] {
  if (!payload || typeof payload !== 'object') return []
  const ids = (payload as Record<string, unknown>).trace_ids
  if (!Array.isArray(ids)) return []
  return ids.filter((id): id is string => typeof id === 'string')
}

/**
 * Subscribe to the engine's global span activity: `onTraceIds` receives the
 * batch of trace ids whose (non-internal) spans landed in each ~300ms
 * coalesce window — on span START (live pending snapshots, when the engine's
 * `live_spans` is on) as well as on close. This is the liveness signal the
 * rows stream lacks — a row arrives once, while activity keeps firing for as
 * long as the trace is doing work.
 *
 * Registered exactly like the old `traces-live` model's trigger, minus the
 * refetch: the payload itself (just ids) is the data. Returns a cleanup that
 * unregisters the handler and the trigger.
 */
export function startTraceActivityFeed(
  client: Pick<IiiClient, 'browserId' | 'on' | 'registerTrigger'>,
  onTraceIds: (traceIds: string[]) => void,
): () => void {
  const off = client.on(TRACE_ACTIVITY_FN, (payload: unknown) => {
    const ids = extractTraceActivityIds(payload)
    if (ids.length > 0) onTraceIds(ids)
  })

  const functionId = `${TRACE_ACTIVITY_FN}::${client.browserId}`
  const offTrigger = client.registerTrigger({
    type: TRACE_TRIGGER_TYPE,
    function_id: functionId,
    config: {},
  })
  dlog('trace activity trigger subscribed', { functionId })

  return () => {
    off()
    try {
      offTrigger()
    } catch {
      // SDK already disposed; nothing to do.
    }
  }
}

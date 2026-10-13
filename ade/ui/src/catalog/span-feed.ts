/**
 * The catalog's live span feed: notify-then-query over the engine's `trace`
 * trigger, the same model the Traces masthead uses
 * (`ade/web/src/lib/traces-activity.ts`, `useAllSpans`).
 *
 * The engine's observability worker coalesces span activity into one
 * `{ trace_ids }` tick per ~300ms window, fired on span start and close for
 * every non-internal span. The tick carries ids only; this feed re-reads the
 * touched traces with `engine::traces::spans` and hands the page the
 * `execute <function_id>` spans it has not delivered yet. The engine's span
 * store stays the single source of truth and retention owner, and an idle
 * engine produces zero traffic.
 *
 * Bounds: at most one engine read per `minIntervalMs`, one read in flight,
 * <= MAX_TRACE_IDS ids and <= SPANS_LIMIT spans per read, and a seen-span map
 * capped at MAX_SEEN. More ids than fit between two reads, a reconnect, or a
 * tab returning to visible switch the next read to a bounded recovery path:
 * recent trace summaries since the last read, then the same span read.
 *
 * The handler id carries the `iii::` prefix, so its per-tick invocations are
 * span-suppressed; with the engine excluding its own delivery and internal
 * `engine::*` spans, the reads a tick causes cannot feed the next tick.
 *
 * This module is framework-free (no React) so the logic is unit tested in
 * `ade/web/src/lib/catalog-span-feed.test.ts`.
 */

import type { ExtensionIii } from '@iii-dev/console-ui'

/** One executed call, read back from its `execute <function_id>` span. */
export interface SpanEvent {
  functionId: string
  worker: string
  durationMs: number
  ok: boolean
  atMs: number
}

export type SpanFeedBus = Pick<
  ExtensionIii,
  | 'browserId'
  | 'on'
  | 'registerTrigger'
  | 'trigger'
  | 'addConnectionStateListener'
>

/** The engine observability worker's coalesced span-activity trigger type. */
export const TRACE_TRIGGER_TYPE = 'trace'
/** Most trace ids one read asks for; more switch the read to recovery. */
export const MAX_TRACE_IDS = 64
/** Spans per read: the strip shows a handful, the map keeps the newest. */
export const SPANS_LIMIT = 200
/** Span ids remembered for de-duplication across re-reads. */
export const MAX_SEEN = 2_000
/** Clock slack applied to every `start_time` lower bound. */
const SLACK_MS = 1_000
const READ_TIMEOUT_MS = 15_000

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

function rowsOf(value: unknown, key: string): unknown[] {
  if (Array.isArray(value)) return value
  if (isRecord(value) && Array.isArray(value[key]))
    return value[key] as unknown[]
  return []
}

/** `[key, value]` pairs (the stored-span wire shape) or a plain object. */
function attribute(span: Record<string, unknown>, key: string): unknown {
  const attrs = span.attributes
  if (Array.isArray(attrs)) {
    for (const pair of attrs) {
      if (Array.isArray(pair) && pair[0] === key) return pair[1]
    }
    return undefined
  }
  return isRecord(attrs) ? attrs[key] : undefined
}

/**
 * The engine's own machinery spans: internal AND parentless. Parented
 * internal spans are builtin calls made inside a real trace and stay.
 * Mirrors `isContextFreeInternalSpan` in TracesV2 `useAllSpans`.
 */
export function isContextFreeInternal(span: Record<string, unknown>): boolean {
  if (str(span.parent_span_id)) return false
  if (attribute(span, 'iii.function.kind') === 'internal') return true
  const fn = attribute(span, 'function_id')
  return typeof fn === 'string' && fn.startsWith('engine::')
}

/** A stored span read back as a catalog call, with its identity. */
export interface FeedSpan {
  spanId: string
  final: boolean
  event: SpanEvent
}

/**
 * Stored spans (`engine::traces::spans` rows) to catalog calls. Only
 * execution spans (`execute <function_id>`) count: caller-side `call …`
 * spans would double-count the same invocation.
 */
export function feedSpansFrom(rows: readonly unknown[]): FeedSpan[] {
  const out: FeedSpan[] = []
  for (const span of rows) {
    if (!isRecord(span)) continue
    const name = str(span.name) ?? ''
    if (!name.startsWith('execute ')) continue
    if (isContextFreeInternal(span)) continue
    const spanId = str(span.span_id)
    if (!spanId) continue
    const start = Number(span.start_time_unix_nano)
    if (!Number.isFinite(start) || start <= 0) continue
    // In-flight spans carry a null/0 end; Number(null) is 0, which would read
    // as a negative duration. Only a real end after the start counts.
    const end = Number(span.end_time_unix_nano)
    const final = Number.isFinite(end) && end > start
    out.push({
      spanId,
      final,
      event: {
        functionId: name.slice('execute '.length),
        worker: str(span.service_name) ?? 'unknown',
        durationMs: final ? (end - start) / 1e6 : 0,
        ok: str(span.status) !== 'error',
        atMs: start / 1e6,
      },
    })
  }
  return out
}

/**
 * De-duplicates re-reads: a span passes once while pending and once more
 * when it turns final (the same two deliveries the page already merges by
 * `functionId@atMs`), never again. Bounded; oldest ids are forgotten first.
 */
export function createSeenSpans(max = MAX_SEEN) {
  const seen = new Map<string, boolean>()
  return {
    fresh(spans: readonly FeedSpan[]): SpanEvent[] {
      const out: SpanEvent[] = []
      for (const span of spans) {
        const held = seen.get(span.spanId)
        if (held === true || (held === false && !span.final)) continue
        seen.delete(span.spanId)
        seen.set(span.spanId, span.final)
        out.push(span.event)
      }
      while (seen.size > max) {
        const oldest = seen.keys().next().value
        if (oldest === undefined) break
        seen.delete(oldest)
      }
      return out
    },
    get size() {
      return seen.size
    },
  }
}

/** Pull `trace_ids` out of a `trace` tick; malformed payloads yield `[]`. */
export function traceIdsOf(payload: unknown): string[] {
  if (!isRecord(payload) || !Array.isArray(payload.trace_ids)) return []
  return payload.trace_ids.filter((id): id is string => typeof id === 'string')
}

export interface SpanFeedOptions {
  /** Distinct handler id under the `iii::` prefix (per page instance). */
  handlerId: string
  debounceMs?: number
  minIntervalMs?: number
  /** Defer reads (ticks keep accumulating); defaults to tab-hidden. */
  isPaused?: () => boolean
  now?: () => number
}

/**
 * Start the feed. `onSpans` receives each batch of not-yet-delivered calls.
 * Returns a cleanup that unregisters the trigger, handler and listeners.
 * A bus without the `trace` trigger type degrades to manual refresh.
 */
export function startSpanFeed(
  bus: SpanFeedBus,
  onSpans: (spans: SpanEvent[]) => void,
  options: SpanFeedOptions,
): () => void {
  const debounceMs = options.debounceMs ?? 300
  const minIntervalMs = options.minIntervalMs ?? 1_000
  const now = options.now ?? Date.now
  const isPaused =
    options.isPaused ??
    (() =>
      typeof document !== 'undefined' && document.visibilityState === 'hidden')
  const openedAt = now()
  const seen = createSeenSpans()

  let disposed = false
  let pendingIds = new Set<string>()
  let recover = false
  let watermark = openedAt
  let inFlight = false
  let trailing = false
  let lastStart = Number.NEGATIVE_INFINITY
  let timer: ReturnType<typeof setTimeout> | null = null

  const spansRead = async (traceIds: string[]): Promise<void> => {
    if (traceIds.length === 0) return
    const out = await bus.trigger(
      'engine::traces::spans',
      {
        trace_ids: traceIds,
        search_all_spans: true,
        include_internal: true,
        include_events: false,
        start_time: Math.max(0, Math.floor(openedAt - SLACK_MS)),
        sort_by: 'start_time',
        sort_order: 'desc',
        limit: SPANS_LIMIT,
      },
      { timeoutMs: READ_TIMEOUT_MS },
    )
    if (disposed) return
    const fresh = seen.fresh(feedSpansFrom(rowsOf(out, 'spans')))
    if (fresh.length > 0) onSpans(fresh)
  }

  /** Bounded catch-up: recent trace summaries since the last read. */
  const recoveryIds = async (since: number): Promise<string[]> => {
    const out = await bus.trigger(
      'engine::traces::list',
      {
        include_internal: false,
        start_time: Math.max(
          0,
          Math.floor(Math.max(since, openedAt) - SLACK_MS),
        ),
        sort_by: 'start_time',
        sort_order: 'desc',
        limit: MAX_TRACE_IDS,
      },
      { timeoutMs: READ_TIMEOUT_MS },
    )
    const ids = new Set<string>()
    // Current engines answer `{ traces }`; a legacy span-shaped answer still
    // carries trace ids.
    for (const row of [...rowsOf(out, 'traces'), ...rowsOf(out, 'spans')]) {
      const id = isRecord(row) ? str(row.trace_id) : undefined
      if (id) ids.add(id)
      if (ids.size >= MAX_TRACE_IDS) break
    }
    return [...ids]
  }

  const run = async (): Promise<void> => {
    const ids = [...pendingIds]
    const recovering = recover
    pendingIds = new Set()
    recover = false
    const since = watermark
    watermark = now()
    try {
      await spansRead(recovering ? await recoveryIds(since) : ids)
    } catch {
      // Traces unavailable or slow: the next tick or recovery self-heals,
      // and the page keeps its manual refresh.
    }
  }

  // A pending timer is kept, not pushed back: ticks arrive every ~300ms
  // under load, and a resetting debounce would starve the read for as long
  // as the engine stays busy. `replace` is only for the interval floor.
  const schedule = (delay: number, replace = false) => {
    if (disposed) return
    if (timer !== null) {
      if (!replace) return
      clearTimeout(timer)
    }
    timer = setTimeout(() => {
      timer = null
      void attempt()
    }, delay)
  }

  const attempt = async (): Promise<void> => {
    if (disposed || isPaused()) return
    if (pendingIds.size === 0 && !recover) return
    if (inFlight) {
      trailing = true
      return
    }
    const wait = lastStart + minIntervalMs - now()
    if (wait > 0) {
      schedule(wait, true)
      return
    }
    inFlight = true
    lastStart = now()
    try {
      await run()
    } finally {
      inFlight = false
      if (trailing || pendingIds.size > 0 || recover) {
        trailing = false
        schedule(debounceMs)
      }
    }
  }

  const request = (opts: { recover?: boolean } = {}) => {
    if (opts.recover) recover = true
    schedule(debounceMs)
  }

  const offHandler = bus.on(options.handlerId, (payload: unknown) => {
    for (const id of traceIdsOf(payload)) {
      if (pendingIds.size >= MAX_TRACE_IDS) {
        recover = true
        break
      }
      pendingIds.add(id)
    }
    request()
  })

  let offTrigger: (() => void) | undefined
  try {
    offTrigger = bus.registerTrigger({
      type: TRACE_TRIGGER_TYPE,
      function_id: `${options.handlerId}::${bus.browserId}`,
      config: {},
    })
  } catch {
    // No observability worker on this engine: the page degrades to manual
    // refresh.
  }

  let dropped = false
  const offConnection = bus.addConnectionStateListener((state: unknown) => {
    // Only a real drop counts: the first `connecting` -> `connected` of a
    // fresh client is not a reconnect and needs no catch-up read.
    if (state === 'connected') {
      if (dropped) request({ recover: true })
      dropped = false
    } else if (
      state === 'disconnected' ||
      state === 'reconnecting' ||
      state === 'failed'
    ) {
      dropped = true
    }
  })

  let offVisibility: (() => void) | undefined
  if (typeof document !== 'undefined' && options.isPaused === undefined) {
    // Ticks that landed while hidden were kept (bounded) in `pendingIds`, or
    // flipped `recover` on overflow; read them now instead of re-reading
    // on every tab switch.
    const onVisible = () => {
      if (document.visibilityState !== 'visible') return
      if (pendingIds.size > 0 || recover) request()
    }
    document.addEventListener('visibilitychange', onVisible)
    offVisibility = () =>
      document.removeEventListener('visibilitychange', onVisible)
  }

  return () => {
    disposed = true
    if (timer !== null) clearTimeout(timer)
    offVisibility?.()
    offConnection()
    try {
      offTrigger?.()
    } catch {
      // Client already disposed; nothing to unregister.
    }
    offHandler()
  }
}

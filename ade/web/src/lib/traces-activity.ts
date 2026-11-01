/**
 * Trace activity feed for the devtools Traces view — the notify-then-query
 * model that replaces the removed span streams (`iii:devtools:trace-rows` /
 * `trace-spans` / `all-spans`).
 *
 * The engine's observability worker coalesces span activity into one
 * `{ trace_ids }` event per ~300ms window and calls
 * `observability::on-span` with it — on span START (live pending
 * snapshots) as well as on close, for every non-internal span. The payload
 * carries ids only, never span data: each surface keeps doing its own seeded
 * read and re-runs that query when an event lands, so the engine remains the
 * single owner of filter semantics (list filters, tag merging, tree
 * correction) and an idle engine produces zero traffic. A dropped event is
 * not fatal — the next event or the caller's reconnect/visibility re-seed
 * self-heals.
 *
 * The handler is named with the `iii::` prefix (`is_iii_builtin_function_id`)
 * and the engine's span source additionally excludes observer-delivery and
 * internal spans (by attribute or span name) before they can re-arm the
 * window, so reacting to an event by querying can never feed the next one.
 */

import type { IiiClient } from '@/lib/iii-client'

/** iii:: prefix → engine-internal → delivery spans hidden from the feed. */
const TRACE_ACTIVITY_FN = 'iii::console::trace_activity'
/** The engine's span-activity source, hosted in `default`. */
const ON_SPAN_FN = 'observability::on-span'

/** Dev-only diagnostics. Silent in production builds. */
function dlog(msg: string, data?: unknown): void {
  if (import.meta.env?.DEV) {
    console.debug(`[traces-activity] ${msg}`, data ?? '')
  }
}

/**
 * Pull the `trace_ids` out of an `observability::on-span` event. The
 * observer receives `{ trace_ids: string[] }` directly (the event the source
 * returned, not a stream frame). Non-string entries and malformed payloads
 * yield `[]`.
 */
export function extractTraceActivityIds(payload: unknown): string[] {
  if (!payload || typeof payload !== 'object') return []
  const ids = (payload as Record<string, unknown>).trace_ids
  if (!Array.isArray(ids)) return []
  return ids.filter((id): id is string => typeof id === 'string')
}

/**
 * Subscribe to the engine's global span activity: `onTraceIds` receives the
 * batch of trace ids whose (non-internal) spans landed in each coalesce
 * window. Returns a cleanup that drops the handler and disposes the observer.
 */
export function startTraceActivityFeed(
  client: Pick<IiiClient, 'browserId' | 'on' | 'observe'>,
  onTraceIds: (traceIds: string[]) => void,
): () => void {
  const off = client.on(TRACE_ACTIVITY_FN, (payload: unknown) => {
    const ids = extractTraceActivityIds(payload)
    if (ids.length > 0) onTraceIds(ids)
  })

  // `on()` registers under `<fn>::<browserId>`; the observer targets that id.
  const target = `${TRACE_ACTIVITY_FN}::${client.browserId}`
  const offObserver = client.observe(ON_SPAN_FN, target)
  dlog('trace activity observer added', { target })

  return () => {
    off()
    try {
      offObserver()
    } catch {
      // SDK already disposed; nothing to do.
    }
  }
}

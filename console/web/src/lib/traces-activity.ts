/**
 * Trace activity feed for the devtools Traces view — the notify-then-query
 * model that replaces the removed span streams (`iii:devtools:trace-rows` /
 * `trace-spans` / `all-spans`).
 *
 * The engine's observability worker coalesces span activity into one
 * `{ trace_ids }` tick per ~300ms window and fires it through the `type:
 * 'trace'` trigger — on span START (live pending snapshots) as well as on
 * close, for every non-internal span. The payload carries ids only, never
 * span data: each surface keeps doing its own seeded read and re-runs that
 * query when a tick lands, so the engine remains the single owner of filter
 * semantics (list filters, tag merging, tree correction) and an idle engine
 * produces zero traffic. A dropped tick is not fatal — the next tick or the
 * caller's reconnect/visibility re-seed self-heals.
 *
 * The handler is named with the `iii::` prefix (`is_iii_builtin_function_id`)
 * and the engine's trace subscriber additionally excludes trigger-delivery
 * and internal spans (by attribute or span name) before they can re-arm the
 * window, so reacting to a tick by querying can never feed the next tick.
 */

import type { IiiClient } from '@/lib/iii-client'

/** iii:: prefix → engine-internal → delivery spans hidden from the feed. */
const TRACE_ACTIVITY_FN = 'iii::console::trace_activity'
/** The engine observability worker's span-activity trigger type. */
const TRACE_TRIGGER_TYPE = 'trace'

/** Dev-only diagnostics. Silent in production builds. */
function dlog(msg: string, data?: unknown): void {
  if (import.meta.env?.DEV) {
    console.debug(`[traces-activity] ${msg}`, data ?? '')
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
 * batch of trace ids whose (non-internal) spans landed in each coalesce
 * window. Returns a cleanup that unregisters the handler and the trigger.
 */
export function startTraceActivityFeed(
  client: Pick<IiiClient, 'browserId' | 'on' | 'registerTrigger'>,
  onTraceIds: (traceIds: string[]) => void,
): () => void {
  const off = client.on(TRACE_ACTIVITY_FN, (payload: unknown) => {
    const ids = extractTraceActivityIds(payload)
    if (ids.length > 0) onTraceIds(ids)
  })

  // `on()` registers under `<fn>::<browserId>`; the trigger must target that id.
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

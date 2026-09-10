/**
 * Live feed of ALL spans across all traces for the masthead strip. The seed
 * first reads compact recent trace summaries, then fetches complete spans only
 * for enough trace IDs to fill the strip. This avoids the expensive global
 * full-span scan while preserving parented internal spans from real traces.
 * Live appends then come from the engine's `iii:devtools:all-spans` stream,
 * keyed by `span_id` so a pending snapshot is replaced in place by its final
 * close frame (never the other way around).
 *
 * Retention is bounded twice: entries older than the strip's usable history
 * are pruned as frames arrive, and a hard cap keeps a busy engine from
 * growing the map without limit (oldest effective-end evicted first).
 * Paused / hidden-tab frames are dropped, matching the rows stream; a
 * reconnect, unpause, or tab-visible re-seeds once, REPLACING the map —
 * the engine store is the source of truth, and merging would immortalize
 * a stale pending span whose close frame was lost across an engine
 * restart.
 *
 * Engines without the all-spans stream simply never deliver a frame: the
 * strip then shows the seed's spans and refreshes on reconnects only.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import { startTraceActivityFeed } from '@/lib/traces-activity'
import {
  fetchTraceSpans,
  fetchTraces,
  type StoredSpan,
  type TraceSummary,
} from '../api/traces'
import { createRefetchCoalescer } from '../lib/refetchCoalescer'
import { isPendingSpan, toMs } from '../lib/traceTransform'

/** Forget spans that ended this long ago (strip window + wide slack). */
const RETENTION_MS = 120_000
/** Seed read size — bounds a busy engine's history, not the live feed. */
const SEED_LIMIT = 500
/**
 * Maximum compact summaries examined to find the seed's trace IDs. The
 * summary RPC prices by the spans it must load to aggregate, so this bounds
 * the engine's work per reseed: 500 summaries of ~40-span turns meant ~20k
 * spans decoded per tick — measured live under four concurrent turns at
 * ~8s per call, the single heaviest query on the connection (MOT-4621). The
 * strip only needs enough recent traces to fill `SEED_LIMIT` spans, which
 * 120 covers unless every recent trace is a single span.
 */
const SEED_TRACE_LIMIT = 120
/** Debounce over activity ticks before re-seeding the strip. */
const ACTIVITY_RESEED_DEBOUNCE_MS = 300
/** Floor between two consecutive activity-driven reseed STARTS: the seed is
 *  two engine reads (summaries + spans), so a busy engine gets at most one
 *  pair per second, one pair in flight (see `createRefetchCoalescer`). */
const ACTIVITY_RESEED_MIN_INTERVAL_MS = 1_000
/** Hard ceiling on retained spans; oldest effective-end evicted first. */
const MAX_SPANS = 1_500
/** Let focus/visibility churn settle before the expensive seed read. */
const VISIBILITY_RESEED_DELAY_MS = 1_000

function effectiveEndMs(span: StoredSpan, now: number): number {
  return isPendingSpan(span) ? now : toMs(span.end_time_unix_nano)
}

/** Replace-by-span_id merge; a final span never regresses to pending. */
function mergeSpans(
  prev: ReadonlyMap<string, StoredSpan>,
  incoming: readonly StoredSpan[],
  now: number,
): Map<string, StoredSpan> {
  const next = new Map(prev)
  for (const [id, span] of prev) {
    if (now - effectiveEndMs(span, now) > RETENTION_MS) next.delete(id)
  }
  for (const span of incoming) {
    const current = next.get(span.span_id)
    if (current && !isPendingSpan(current) && isPendingSpan(span)) continue
    next.set(span.span_id, span)
  }
  if (next.size > MAX_SPANS) {
    const byAge = [...next.entries()].sort(
      (a, b) => effectiveEndMs(a[1], now) - effectiveEndMs(b[1], now),
    )
    for (const [id] of byAge.slice(0, next.size - MAX_SPANS)) next.delete(id)
  }
  return next
}

/**
 * The engine's live all-spans feed excludes exactly one internal class: the
 * engine's own machinery spans — internal (`iii.function.kind: internal` /
 * `engine::*`) AND parentless (context-free). Parented internal spans are
 * built-in calls made inside a real trace (a turn's `configuration::list`)
 * and DO stream. The seed must mirror that split, so it fetches with
 * `include_internal: true` and re-applies the machinery exclusion here —
 * otherwise builtin bars would flip in and out across a reseed.
 * Mirrors `is_context_free_internal_span` in
 * `iii/engine/src/workers/observability/mod.rs`.
 */
export function isContextFreeInternalSpan(span: StoredSpan): boolean {
  if (span.parent_span_id) return false
  return span.attributes.some(
    ([k, v]) =>
      (k === 'iii.function.kind' && v === 'internal') ||
      (k === 'function_id' &&
        typeof v === 'string' &&
        v.startsWith('engine::')),
  )
}

/** Select newest traces until their non-internal span counts fill the seed. */
export function selectSeedTraceIds(traces: readonly TraceSummary[]): string[] {
  const traceIds: string[] = []
  let spanCount = 0

  for (const trace of traces) {
    traceIds.push(trace.trace_id)
    spanCount += Math.max(1, trace.span_count)
    if (spanCount >= SEED_LIMIT) break
  }

  return traceIds
}

/**
 * Build a bounded recent-span seed without asking the engine to page every
 * stored span. The summary RPC is trace-first and compact; the detail RPC then
 * uses the archive's trace-id index and filters the same retention window the
 * client applies below.
 */
export async function fetchLiveSpanSeed(
  now = Date.now(),
): Promise<StoredSpan[]> {
  const summaries = await fetchTraces({
    include_internal: false,
    sort_by: 'start_time',
    sort_order: 'desc',
    limit: SEED_TRACE_LIMIT,
  })
  if (summaries.legacyContract) {
    const legacyResult = await fetchTraceSpans({
      search_all_spans: true,
      include_internal: true,
      sort_by: 'start_time',
      sort_order: 'desc',
      limit: SEED_LIMIT,
    })
    return legacyResult.spans
  }

  const traceIds = selectSeedTraceIds(summaries.traces)
  if (traceIds.length === 0) return []

  const result = await fetchTraceSpans({
    trace_ids: traceIds,
    search_all_spans: true,
    include_internal: true,
    start_time: now - RETENTION_MS,
    sort_by: 'start_time',
    sort_order: 'desc',
    limit: SEED_LIMIT,
  })
  return result.spans
}

export function useAllSpans(isPaused: boolean): readonly StoredSpan[] {
  const [spans, setSpans] = useState<ReadonlyMap<string, StoredSpan>>(new Map())
  const seedInFlightRef = useRef<Promise<void> | null>(null)

  const isPausedRef = useRef(isPaused)
  useEffect(() => {
    isPausedRef.current = isPaused
  }, [isPaused])

  // Seed read — run on mount, and re-run on reconnect and on unpause (the
  // stream dropped frames while away). REPLACES the map; see the module
  // docstring.
  const seed = useCallback((): Promise<void> => {
    if (seedInFlightRef.current) return seedInFlightRef.current

    const request = (async () => {
      try {
        const seedSpans = await fetchLiveSpanSeed()
        setSpans(
          mergeSpans(
            new Map(),
            seedSpans.filter((s) => !isContextFreeInternalSpan(s)),
            Date.now(),
          ),
        )
      } catch {
        // Traces unavailable (memory exporter off, transient error) — the
        // strip simply renders empty until data arrives.
      }
    })().finally(() => {
      if (seedInFlightRef.current === request) seedInFlightRef.current = null
    })
    seedInFlightRef.current = request
    return request
  }, [])
  const seedRef = useRef(seed)
  useEffect(() => {
    seedRef.current = seed
    void seed()
  }, [seed])

  const wasPausedRef = useRef(isPaused)
  useEffect(() => {
    if (wasPausedRef.current && !isPaused) void seedRef.current()
    wasPausedRef.current = isPaused
  }, [isPaused])

  useEffect(() => {
    let stop: (() => void) | undefined
    let disposed = false

    const isHidden = () =>
      typeof document !== 'undefined' && document.visibilityState === 'hidden'

    void (async () => {
      const client = await getIiiClient()
      if (disposed) return
      // Notify-then-query: each activity tick re-seeds the strip's recent
      // window (REPLACE semantics, same reads as the initial seed). One seed
      // in flight at a time; a tick that lands mid-seed queues one trailing
      // reseed, so the strip always settles on the engine's latest state.
      const reseed = createRefetchCoalescer({
        run: () => seedRef.current(),
        debounceMs: ACTIVITY_RESEED_DEBOUNCE_MS,
        minIntervalMs: ACTIVITY_RESEED_MIN_INTERVAL_MS,
        shouldRun: () => !disposed && !isPausedRef.current && !isHidden(),
      })
      const offFeed = startTraceActivityFeed(client, () => {
        if (isPausedRef.current || isHidden()) return
        reseed.request()
      })
      const offConn = client.addConnectionStateListener((state) => {
        if (state === 'connected' && !isPausedRef.current) {
          void seedRef.current()
        }
      })
      // Hidden-tab frames are dropped above, so the map has a hole after a
      // tab switch — re-seed on return, mirroring the list's recovery in
      // `useTraceData`. (REPLACE semantics, see the module docstring.)
      let offVisibility: (() => void) | undefined
      if (typeof document !== 'undefined') {
        let visibilitySeedTimer: ReturnType<typeof setTimeout> | undefined
        const onVisible = () => {
          if (visibilitySeedTimer !== undefined) {
            clearTimeout(visibilitySeedTimer)
            visibilitySeedTimer = undefined
          }
          if (document.visibilityState === 'visible' && !isPausedRef.current) {
            visibilitySeedTimer = setTimeout(() => {
              visibilitySeedTimer = undefined
              if (!isPausedRef.current) void seedRef.current()
            }, VISIBILITY_RESEED_DELAY_MS)
          }
        }
        document.addEventListener('visibilitychange', onVisible)
        offVisibility = () => {
          document.removeEventListener('visibilitychange', onVisible)
          if (visibilitySeedTimer !== undefined) {
            clearTimeout(visibilitySeedTimer)
          }
        }
      }
      stop = () => {
        offFeed()
        offConn()
        offVisibility?.()
        reseed.dispose()
      }
    })()

    return () => {
      disposed = true
      stop?.()
    }
  }, [])

  return useMemo(() => [...spans.values()], [spans])
}

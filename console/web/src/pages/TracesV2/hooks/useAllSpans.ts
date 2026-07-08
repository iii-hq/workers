/**
 * Live feed of ALL spans across all traces for the masthead strip: one seed
 * read (`engine::traces::list` with `search_all_spans` and no name filter —
 * which returns every stored span, newest first) then live appends from the
 * engine's `iii:devtools:all-spans` stream, keyed by `span_id` so a pending
 * live snapshot is replaced in place by its final close frame (never the
 * other way around).
 *
 * Retention is bounded twice: entries older than the strip's usable history
 * are pruned as frames arrive, and a hard cap keeps a busy engine from
 * growing the map without limit (oldest effective-end evicted first).
 * Paused / hidden-tab frames are dropped, matching the rows stream; a
 * reconnect or unpause re-seeds once, REPLACING the map — the engine store
 * is the source of truth, and merging would immortalize a stale pending
 * span whose close frame was lost across an engine restart.
 *
 * Engines without the all-spans stream simply never deliver a frame: the
 * strip then shows the seed's spans and refreshes on reconnects only.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import { startAllSpansFeed } from '@/lib/traces-stream'
import { fetchTraces, type StoredSpan } from '../api/traces'
import { isPendingSpan, toMs } from '../lib/traceTransform'

/** Forget spans that ended this long ago (strip window + wide slack). */
const RETENTION_MS = 120_000
/** Seed read size — bounds a busy engine's history, not the live feed. */
const SEED_LIMIT = 500
/** Hard ceiling on retained spans; oldest effective-end evicted first. */
const MAX_SPANS = 1_500

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

export function useAllSpans(
  isPaused: boolean,
  showSystem: boolean,
): readonly StoredSpan[] {
  const [spans, setSpans] = useState<ReadonlyMap<string, StoredSpan>>(new Map())

  const isPausedRef = useRef(isPaused)
  useEffect(() => {
    isPausedRef.current = isPaused
  }, [isPaused])

  // Seed read — run on mount / showSystem flips, and re-run on reconnect and
  // on unpause (the stream dropped frames while away). REPLACES the map; see
  // the module docstring.
  const seed = useCallback(async () => {
    try {
      const res = await fetchTraces({
        search_all_spans: true,
        include_internal: showSystem,
        sort_by: 'start_time',
        sort_order: 'desc',
        limit: SEED_LIMIT,
      })
      setSpans(mergeSpans(new Map(), res.spans, Date.now()))
    } catch {
      // Traces unavailable (memory exporter off, transient error) — the
      // strip simply renders empty until data arrives.
    }
  }, [showSystem])
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
      const offFeed = startAllSpansFeed(client, (incoming) => {
        if (isPausedRef.current || isHidden()) return
        setSpans((prev) => mergeSpans(prev, incoming, Date.now()))
      })
      const offConn = client.addConnectionStateListener((state) => {
        if (state === 'connected' && !isPausedRef.current) {
          void seedRef.current()
        }
      })
      stop = () => {
        offFeed()
        offConn()
      }
    })()

    return () => {
      disposed = true
      stop?.()
    }
  }, [])

  return useMemo(() => [...spans.values()], [spans])
}

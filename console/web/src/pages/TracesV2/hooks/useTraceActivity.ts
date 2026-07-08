/**
 * Per-trace liveness for the timeline strip, fed by the engine's `trace`
 * trigger (`startTraceActivityFeed`).
 *
 * The trace LIST only ever carries ROOT spans, so for a queue-triggered
 * trace the row is the instant "publish" moment and says nothing about the
 * seconds of child work that follow. This hook records, per trace id, the
 * wall-clock time of the most recent span activity. The strip combines that
 * with the rows: activity newer than the row's own end means the trace is
 * still doing work — render it as a live, growing bar; once activity stops,
 * settle the bar at the last activity time (≈ the trace's real end, within
 * one coalesce window).
 *
 * With the engine's `live_spans` on (the local-dev default), activity fires
 * on span START as well as close — pending snapshots land in the store the
 * moment work begins, so bars go live at trace start. On engines without
 * live spans the signal degrades to spans CLOSING only: a trace sitting
 * inside one long span shows no activity until that span closes, and its bar
 * extends in steps rather than growing continuously.
 *
 * Paused / hidden-tab batches are dropped, matching the rows stream: entries
 * then age out naturally (the strip's idle threshold settles the bars).
 */

import { useEffect, useRef, useState } from 'react'
import { getIiiClient } from '@/lib/iii-client'
import { startTraceActivityFeed } from '@/lib/traces-stream'

/** forget traces with no activity for this long (strip window + wide slack) */
const ACTIVITY_RETENTION_MS = 180_000

export type TraceActivityMap = ReadonlyMap<string, number>

export function useTraceActivity(isPaused: boolean): TraceActivityMap {
  const [activity, setActivity] = useState<TraceActivityMap>(new Map())

  const isPausedRef = useRef(isPaused)
  useEffect(() => {
    isPausedRef.current = isPaused
  }, [isPaused])

  useEffect(() => {
    let stop: (() => void) | undefined
    let disposed = false

    const isHidden = () =>
      typeof document !== 'undefined' && document.visibilityState === 'hidden'

    void (async () => {
      const client = await getIiiClient()
      if (disposed) return
      stop = startTraceActivityFeed(client, (traceIds) => {
        if (isPausedRef.current || isHidden()) return
        const now = Date.now()
        setActivity((prev) => {
          const next = new Map(prev)
          for (const [id, t] of prev) {
            if (now - t > ACTIVITY_RETENTION_MS) next.delete(id)
          }
          for (const id of traceIds) next.set(id, now)
          return next
        })
      })
    })()

    return () => {
      disposed = true
      stop?.()
    }
  }, [])

  return activity
}

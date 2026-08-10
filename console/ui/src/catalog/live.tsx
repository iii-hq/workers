/**
 * The page-level live layer: what is running RIGHT NOW, visible without
 * selecting anything.
 *
 * `useLiveActivity` folds the all-spans stream into two things a page can
 * render from directly: a rolling feed of the most recent calls (the
 * now-strip) and a per-function "last call" map (row pulses and the live
 * meta line). One subscription per page, shared by every row.
 *
 * The strip is the page's one bold element. Everything it shows is a real
 * execution the engine just recorded — during a harness turn it reads as
 * the agent thinking out loud.
 */

import { type Host, StatusDot } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { formatDuration } from './ActivityFeed'
import { type SpanEvent, useSpanFeed } from './engine'

const FEED_LENGTH = 18
const PULSE_MS = 1400

export interface LiveActivity {
  /** Newest first, capped at FEED_LENGTH. */
  feed: readonly SpanEvent[]
  /** Latest call per function id. */
  lastCall: ReadonlyMap<string, SpanEvent>
  /** Function ids whose pulse animation is currently running. */
  pulsing: ReadonlySet<string>
}

export function useLiveActivity(host: Host): LiveActivity {
  const [feed, setFeed] = useState<readonly SpanEvent[]>([])
  const [lastCall, setLastCall] = useState<ReadonlyMap<string, SpanEvent>>(
    new Map(),
  )
  const [pulsing, setPulsing] = useState<ReadonlySet<string>>(new Set())
  const timers = useRef<Map<string, number>>(new Map())

  useSpanFeed(
    host,
    useCallback((spans: SpanEvent[]) => {
      const newest = [...spans].sort((a, b) => b.atMs - a.atMs)
      setFeed((prev) => {
        // A span can arrive twice: in-flight (no end yet) and completed.
        // Same identity, so the completed version replaces the running one
        // instead of stacking next to it.
        const merged = new Map<string, SpanEvent>()
        for (const span of [...newest, ...prev]) {
          const key = `${span.functionId}@${span.atMs}`
          const held = merged.get(key)
          if (!held || (held.durationMs === 0 && span.durationMs > 0)) {
            merged.set(key, span)
          }
        }
        return [...merged.values()]
          .sort((a, b) => b.atMs - a.atMs)
          .slice(0, FEED_LENGTH)
      })
      setLastCall((prev) => {
        const next = new Map(prev)
        for (const span of spans) {
          const held = next.get(span.functionId)
          if (!held || span.atMs >= held.atMs) next.set(span.functionId, span)
        }
        return next
      })
      setPulsing((prev) => {
        const next = new Set(prev)
        for (const span of spans) next.add(span.functionId)
        return next
      })
      for (const span of spans) {
        const existing = timers.current.get(span.functionId)
        if (existing !== undefined) window.clearTimeout(existing)
        timers.current.set(
          span.functionId,
          window.setTimeout(() => {
            timers.current.delete(span.functionId)
            setPulsing((prev) => {
              const next = new Set(prev)
              next.delete(span.functionId)
              return next
            })
          }, PULSE_MS),
        )
      }
    }, []),
  )

  useEffect(
    () => () => {
      for (const timer of timers.current.values()) window.clearTimeout(timer)
    },
    [],
  )

  return { feed, lastCall, pulsing }
}

/** "3s ago" for the live meta line; empty under a second so fresh rows read as now. */
export function agoLabel(atMs: number, nowMs: number): string {
  const seconds = Math.floor((nowMs - atMs) / 1000)
  if (seconds < 1) return 'now'
  if (seconds < 60) return `${seconds}s ago`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  return `${Math.floor(minutes / 60)}h ago`
}

/**
 * The strip under the page head: recent function calls, newest on the left.
 * Clicking one jumps to that function.
 */
export function NowStrip({
  activity,
  onSelect,
}: {
  activity: LiveActivity
  onSelect?: (functionId: string) => void
}) {
  // A ticking clock would re-render the whole page each second; the strip
  // re-renders on arrival anyway, so relative times refresh with traffic.
  const shown = activity.feed.slice(0, 6)

  if (shown.length === 0) {
    return (
      <div className="console-catalog-nowstrip" data-empty="true">
        <span className="console-catalog-nowstrip-label">recent calls</span>
        <span className="quiet">no calls recorded since this page opened</span>
      </div>
    )
  }

  const now = Date.now()
  return (
    <div className="console-catalog-nowstrip">
      <span className="console-catalog-nowstrip-label">recent calls</span>
      <div className="console-catalog-nowstrip-track">
        {shown.map((span) => (
          <button
            key={`${span.functionId}-${span.atMs}`}
            type="button"
            className="console-catalog-nowentry"
            onClick={() => onSelect?.(span.functionId)}
            title={`${span.worker} · ${agoLabel(span.atMs, now)}`}
          >
            <StatusDot tone={span.ok ? 'accent' : 'alert'} />
            <span className="fn">{span.functionId}</span>
            <span className="dur">
              {span.durationMs > 0
                ? formatDuration(span.durationMs)
                : 'running'}
            </span>
          </button>
        ))}
      </div>
    </div>
  )
}

/** The quiet live meta a row shows once its function has been seen running. */
export function LastCallMeta({ span }: { span: SpanEvent | undefined }) {
  if (!span) return null
  return (
    <span className="console-catalog-lastcall" data-ok={span.ok}>
      {span.ok ? '' : 'failed · '}
      {span.durationMs > 0 ? `${formatDuration(span.durationMs)} · ` : ''}
      {agoLabel(span.atMs, Date.now())}
    </span>
  )
}

/**
 * The github page (#/ext/github): a live ACTIVITY feed of what the agent does
 * with the github worker. It binds a tab-scoped subscription to the worker's
 * `github::called` trigger type and appends one row per call — no owner/name
 * to type, no polling: the worker pushes each event as it finishes.
 *
 * Each row shows the function id (badge), a one-line arg echo, an ok/error dot
 * with the call duration, and a relative timestamp; clicking a row expands the
 * ACTUAL result the worker budgeted into the event, rendered by its `kind`
 * (list / object / text / diff / outcome — see result-views.tsx and the
 * `preview()` budgeter in github/src/events.rs). A live/paused toggle stops
 * appending new events, and clear empties the (bounded, newest-first) list.
 *
 * The host only mounts this page while the github worker is connected, so there
 * is no presence gate; this page invokes nothing — it is a passive observer of
 * the bus.
 */

import { Badge, Button, EmptyState, ErrorBoundary, type Host, StatusDot } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { type CalledEvent, useGithubCalled } from './events'
import { formatRelative } from './format'
import { Activity, type IconProps } from './icons'
import { ResultView } from './result-views'

/** Per-tab handler id — the `iii::` prefix keeps per-event invocations
 *  span-suppressed and out of the trace feed (host.iii.on namespaces it
 *  `::<browserId>`, which the trigger's function_id must match). */
const EVENTS_FN = 'iii::github-ui::called'
/** Newest-first cap so a long-running session can't grow the list unbounded. */
const MAX_ENTRIES = 200

interface Entry extends CalledEvent {
  /** Stable react key + expansion id. */
  key: string
  /** Arrival time (ms) for the relative timestamp. */
  receivedAt: number
}

const ActivityIcon = (p: IconProps) => <Activity size={28} {...p} />

/**
 * Register ONE tab-scoped binding to `github::called` for the page's lifetime
 * and collect events into a bounded, newest-first list. Paused drops incoming
 * events (checked via a ref so the subscription never re-registers). The
 * binding is unregistered on unmount — hot reload disposes it with the page.
 */
function useCalledFeed(host: Host) {
  const [entries, setEntries] = useState<Entry[]>([])
  const [paused, setPaused] = useState(false)
  const pausedRef = useRef(paused)
  pausedRef.current = paused
  const seq = useRef(0)

  useGithubCalled(host, EVENTS_FN, (event) => {
    if (pausedRef.current) return
    if (!event || typeof event.function_id !== 'string') return
    const entry: Entry = {
      function_id: event.function_id,
      args_summary: event.args_summary ?? '',
      repo: event.repo ?? null,
      ok: Boolean(event.ok),
      duration_ms: Number(event.duration_ms) || 0,
      result_summary: event.result_summary ?? '',
      kind: typeof event.kind === 'string' ? event.kind : 'object',
      result_preview: event.result_preview ?? null,
      timestamp: event.timestamp ?? '',
      key: `${Date.now()}-${seq.current++}`,
      receivedAt: Date.now(),
    }
    setEntries((prev) => [entry, ...prev].slice(0, MAX_ENTRIES))
  })

  const clear = useCallback(() => setEntries([]), [])
  return { entries, clear, paused, setPaused }
}

export function ActivityFeed({ host }: { host: Host }) {
  const { entries, clear, paused, setPaused } = useCalledFeed(host)
  const [expanded, setExpanded] = useState<string | null>(null)

  // Re-render on a slow tick so "Ns ago" stays fresh without per-row timers.
  const [, forceTick] = useState(0)
  useEffect(() => {
    const id = window.setInterval(() => forceTick((n) => n + 1), 15000)
    return () => window.clearInterval(id)
  }, [])
  const now = Date.now()

  return (
    <div className="gh-page">
      <div className="gh-head">
        <div>
          <div className="gh-title">github activity</div>
          <div className="gh-sub">
            {entries.length
              ? `${entries.length} recent call${entries.length === 1 ? '' : 's'}`
              : 'live feed of github worker calls'}
          </div>
        </div>
        <div className="gh-head-actions">
          <Button variant="ghost" size="sm" aria-pressed={!paused} onClick={() => setPaused((p) => !p)}>
            <StatusDot tone={paused ? 'warn' : 'accent'} pulse={!paused} aria-hidden />
            {paused ? 'paused' : 'live'}
          </Button>
          <Button variant="ghost" size="sm" disabled={entries.length === 0} onClick={clear}>
            clear
          </Button>
        </div>
      </div>

      {entries.length === 0 ? (
        <EmptyState
          icon={ActivityIcon}
          title="waiting for github activity"
          description="trigger a github function and it shows here — pull requests, issues, runs, releases, searches, and any gh command the agent runs."
        />
      ) : (
        <table className="gh-feed">
          <tbody>
            {entries.map((entry) => (
              <ActivityRow
                key={entry.key}
                entry={entry}
                now={now}
                expanded={expanded === entry.key}
                onToggle={() => setExpanded((cur) => (cur === entry.key ? null : entry.key))}
              />
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}

function ActivityRow({
  entry,
  now,
  expanded,
  onToggle,
}: {
  entry: Entry
  now: number
  expanded: boolean
  onToggle: () => void
}) {
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onToggle()
    }
  }
  return (
    <>
      <tr
        className={`gh-feed-row${expanded ? ' expanded' : ''}`}
        role="button"
        tabIndex={0}
        aria-expanded={expanded}
        onClick={onToggle}
        onKeyDown={onKeyDown}
      >
        <td className="gh-feed-fn">
          <Badge>{entry.function_id}</Badge>
        </td>
        <td className="gh-feed-args">
          {entry.repo ? <span className="gh-feed-repo">{entry.repo}</span> : null}
          <span className="gh-feed-argtext">{entry.args_summary || '—'}</span>
        </td>
        <td className="gh-feed-status">
          <StatusDot tone={entry.ok ? 'accent' : 'alert'} aria-hidden />
          <span className={entry.ok ? 'gh-ok' : 'gh-err'}>{entry.ok ? 'ok' : 'error'}</span>
          <span className="gh-feed-dur">{formatDuration(entry.duration_ms)}</span>
        </td>
        <td className="gh-feed-time" title={entry.timestamp || undefined}>
          {formatRelative((now - entry.receivedAt) / 1000)}
        </td>
      </tr>
      {expanded ? (
        <tr className="gh-feed-detail-row">
          <td colSpan={4}>
            <div className="gh-feed-detail">
              {entry.result_summary ? <div className="gh-feed-summary">{entry.result_summary}</div> : null}
              <ErrorBoundary fallback={() => <div className="gh-rv-empty">could not render this result</div>}>
                <ResultView kind={entry.kind} preview={entry.result_preview} ok={entry.ok} />
              </ErrorBoundary>
            </div>
          </td>
        </tr>
      ) : null}
    </>
  )
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)}s`
}

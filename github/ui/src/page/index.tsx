/**
 * The ACTIVITY view of the github page: a live feed of what the agent does
 * with the github worker. It binds a tab-scoped subscription to the worker's
 * `github::called` trigger type and appends one row per call — no owner/name
 * to type, no polling: the worker pushes each event as it finishes.
 *
 * Each row shows the function id (badge), a one-line arg echo, an ok/error
 * dot with the call duration, and a relative timestamp; clicking a row
 * expands the ACTUAL result the worker budgeted into the event, rendered by
 * its `kind` (list / object / text / diff / outcome — see result-views.tsx
 * and the `preview()` budgeter in github/src/events.rs). A live/paused
 * toggle in the view's toolbar stops appending new events, and clear
 * empties the (bounded, newest-first) list.
 *
 * Layout adapts to the width the view HAS (useContainerNarrow — a
 * ResizeObserver on its own root, never a viewport media query): wide keeps
 * the four aligned columns; narrow regroups each row onto two lines with
 * touch-sized targets and drops the relative timestamp.
 *
 * The host only mounts this page while the github worker is connected, so
 * there is no presence gate; this view invokes nothing — it is a passive
 * observer of the bus.
 */

import { Badge, Button, ErrorBoundary, type Host, StatusDot } from '@iii-dev/console-ui'
import { type KeyboardEvent, useCallback, useEffect, useRef, useState } from 'react'
import { type CalledEvent, useGithubCalled } from './events'
import { formatRelative } from './format'
import { Activity } from './icons'
import { NARROW_BELOW, useContainerNarrow } from './narrow'
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

/**
 * Register ONE tab-scoped binding to `github::called` for the view's lifetime
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
  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)

  // Re-render on a slow tick so "Ns ago" stays fresh without per-row timers.
  const [, forceTick] = useState(0)
  useEffect(() => {
    const id = window.setInterval(() => forceTick((n) => n + 1), 15000)
    return () => window.clearInterval(id)
  }, [])
  const now = Date.now()

  return (
    <div className={`gh-ui-view${narrow ? ' narrow' : ''}`} ref={rootRef}>
      <div className="gh-ui-toolbar">
        <span className="gh-ui-toolbar-note">
          {entries.length
            ? `${entries.length} recent call${entries.length === 1 ? '' : 's'}`
            : 'live feed of github worker calls'}
        </span>
        <span className="spacer" />
        <Button variant="ghost" size="sm" aria-pressed={!paused} onClick={() => setPaused((p) => !p)}>
          <StatusDot tone={paused ? 'warn' : 'accent'} pulse={!paused} aria-hidden />
          {paused ? 'paused' : 'live'}
        </Button>
        <Button variant="ghost" size="sm" disabled={entries.length === 0} onClick={clear}>
          clear
        </Button>
      </div>

      {entries.length === 0 ? (
        <div className="gh-ui-hero">
          <Activity className="gh-ui-hero-icon" />
          <h2 className="gh-ui-hero-title">Waiting for github activity</h2>
          <p className="gh-ui-hero-body">
            Trigger a github function and it shows here — pull requests, issues, runs, releases, searches, and any gh
            command the agent runs.
          </p>
        </div>
      ) : (
        <div className="gh-ui-feed" role="list">
          {entries.map((entry) => (
            <ActivityRow
              key={entry.key}
              entry={entry}
              now={now}
              expanded={expanded === entry.key}
              onToggle={() => setExpanded((cur) => (cur === entry.key ? null : entry.key))}
            />
          ))}
        </div>
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
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onToggle()
    }
  }
  return (
    <div className="gh-ui-feed-item" role="listitem">
      <div
        className={`gh-ui-feed-row${expanded ? ' expanded' : ''}`}
        role="button"
        tabIndex={0}
        aria-expanded={expanded}
        onClick={onToggle}
        onKeyDown={onKeyDown}
      >
        <span className="gh-ui-feed-fn">
          <Badge>{entry.function_id}</Badge>
        </span>
        <span className="gh-ui-feed-args">
          {entry.repo ? <span className="gh-ui-feed-repo">{entry.repo}</span> : null}
          <span className="gh-ui-feed-argtext">{entry.args_summary || '—'}</span>
        </span>
        <span className="gh-ui-feed-status">
          <StatusDot tone={entry.ok ? 'accent' : 'alert'} aria-hidden />
          <span className={entry.ok ? 'gh-ui-ok' : 'gh-ui-err'}>{entry.ok ? 'ok' : 'error'}</span>
          <span className="gh-ui-feed-dur">{formatDuration(entry.duration_ms)}</span>
        </span>
        <span className="gh-ui-feed-time" title={entry.timestamp || undefined}>
          {formatRelative((now - entry.receivedAt) / 1000)}
        </span>
      </div>
      {expanded ? (
        <div className="gh-ui-feed-detail">
          {entry.result_summary ? <div className="gh-ui-feed-summary">{entry.result_summary}</div> : null}
          <ErrorBoundary fallback={() => <div className="gh-ui-rv-empty">could not render this result</div>}>
            <ResultView kind={entry.kind} preview={entry.result_preview} ok={entry.ok} />
          </ErrorBoundary>
        </div>
      ) : null}
    </div>
  )
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`
  return `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)}s`
}

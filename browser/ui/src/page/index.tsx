import {
  Button,
  EmptyState,
  type Host,
  StatusDot,
  StatusPanel,
} from '@iii-dev/console-ui'
import { useEffect, useMemo, useState } from 'react'
import { errorMessage, startBrowserSession } from '../lib/browser'
import { cn } from '../lib/cn'
import { AlertCircle, Globe, Plus, RefreshCw } from '../lib/icons'
import { SessionRail } from './SessionRail'
import { SessionView } from './SessionView'
import { useBrowserSessionsLive } from './useBrowserSessionsLive'

/**
 * The browser page (#/ext/browser): the browser worker's live surface — a
 * session rail, a screencast-fed viewport, and console/network feeds for the
 * selected session, so a user can watch what an agent is doing in a Chromium
 * session and pick elements into the clipboard.
 *
 * The host only mounts this page while the browser worker is connected, so
 * there is no presence gate here (worker disconnect disposes the script and
 * drops the nav entry). Session selection is page-local state — the injected
 * page owns its own view, and the host owns the `#/ext/browser` route.
 */

export function BrowserPage({ host }: { host: Host }) {
  const { sessions, loading, error, live, refresh } = useBrowserSessionsLive(
    host,
    true,
  )

  const [selectedId, setSelectedId] = useState<string | null>(null)
  const selected = useMemo(
    () => sessions.find((s) => s.session_id === selectedId) ?? null,
    [sessions, selectedId],
  )

  // Auto-select the first session when nothing (or a stopped session) is
  // selected, and clear a selection whose session is gone.
  useEffect(() => {
    if (loading) return
    if (selectedId && sessions.some((s) => s.session_id === selectedId)) return
    const next = sessions[0]?.session_id ?? null
    if (next !== selectedId) setSelectedId(next)
  }, [loading, sessions, selectedId])

  const [starting, setStarting] = useState(false)
  const [startError, setStartError] = useState<string | null>(null)
  const handleNewSession = async () => {
    if (starting) return
    setStarting(true)
    try {
      const started = await startBrowserSession(host.iii)
      setStartError(null)
      refresh()
      if (started) setSelectedId(started.session_id)
    } catch (err) {
      setStartError(errorMessage(err))
    } finally {
      setStarting(false)
    }
  }

  const countLabel = loading ? '...' : String(sessions.length)

  return (
    <main className="br-ui-page" aria-label="browser">
      <header className="br-ui-page-head">
        <div>
          <h1 className="br-ui-page-title">browser</h1>
          <p className="br-ui-page-sub">{countLabel} sessions</p>
        </div>
        <div className="br-ui-page-actions">
          <span
            className="br-ui-live"
            title={
              live
                ? 'updates arrive on the browser session triggers'
                : 'live bindings unavailable; refreshing on a timer'
            }
          >
            <StatusDot tone={live ? 'accent' : 'ink'} pulse={live} />
            {live ? 'live' : 'polling'}
          </span>
          <Button variant="ghost" size="sm" onClick={refresh} disabled={loading}>
            <RefreshCw size={14} className={cn(loading && 'br-ui-spin')} aria-hidden />
            refresh
          </Button>
          <Button
            variant="primary"
            size="sm"
            onClick={() => void handleNewSession()}
            disabled={starting}
          >
            <Plus size={14} aria-hidden />
            {starting ? 'starting...' : 'new session'}
          </Button>
        </div>
      </header>

      {error ? (
        <div className="br-ui-page-body">
          <StatusPanel
            variant="alert"
            icon={<AlertCircle size={18} />}
            headline="failed to load browser sessions"
            detail={error}
          />
        </div>
      ) : !loading && sessions.length === 0 ? (
        <div className="br-ui-page-body">
          {startError ? (
            <StatusPanel
              variant="alert"
              icon={<AlertCircle size={18} />}
              headline="could not start a session"
              detail={startError}
            />
          ) : null}
          <EmptyState
            icon={Globe}
            title="no browser sessions"
            description="sessions started by agents appear here automatically. start one yourself with the new session button above, or ask an agent to call browser::sessions::start"
          />
        </div>
      ) : (
        <div className="br-ui-split">
          <aside className="br-ui-aside">
            {startError ? (
              <p className="br-ui-aside-err">{startError}</p>
            ) : null}
            <SessionRail
              sessions={sessions}
              selectedId={selectedId}
              onSelect={setSelectedId}
            />
          </aside>
          {selected ? (
            <SessionView
              key={selected.session_id}
              host={host}
              session={selected}
              enabled
              onSessionsRefresh={refresh}
              onStopped={() => setSelectedId(null)}
            />
          ) : (
            <div className="br-ui-page-placeholder">
              <p>select a session</p>
            </div>
          )}
        </div>
      )}
    </main>
  )
}

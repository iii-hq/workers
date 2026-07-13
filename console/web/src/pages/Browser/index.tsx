import { AlertCircle, Globe, Plus, RefreshCw } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { EmptyState } from '@/components/ui/EmptyState'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import {
  isBrowserAvailable,
  useBrowserStatus,
} from '@/hooks/use-browser-status'
import { useBrowserSessionRoute } from '@/hooks/use-hash-route'
import { startBrowserSession } from '@/lib/browser'
import { useConversationsCtx } from '@/lib/conversations-context'
import { cn } from '@/lib/utils'
import { SessionRail } from './components/SessionRail'
import { SessionView } from './components/SessionView'
import { useBrowserSessionsLive } from './hooks/useBrowserSessionsLive'

/**
 * Live browser page: session rail -> polled viewport -> console/network
 * feeds, so a user can watch what an agent is doing in a Chromium session
 * and pick elements straight into chat. Data is `browser::sessions::list`,
 * refreshed on the session lifecycle trigger types (poll fallback while
 * bindings are unavailable). The nav entry is gated on worker presence; a
 * direct hash hit with the worker absent lands on the install notice below.
 */

export function Browser() {
  const { backend } = useConversationsCtx()
  const status = useBrowserStatus(backend.id === 'real')
  const available = isBrowserAvailable(status)

  const { sessions, loading, error, live, refresh } =
    useBrowserSessionsLive(available)

  const [selectedId, setSelectedId] = useBrowserSessionRoute()
  const selected = useMemo(
    () => sessions.find((s) => s.session_id === selectedId) ?? null,
    [sessions, selectedId],
  )

  // Auto-select the first session when nothing (or a stopped session) is
  // selected, and clear a deep link whose session is gone.
  useEffect(() => {
    if (!available || loading) return
    if (selectedId && sessions.some((s) => s.session_id === selectedId)) return
    const next = sessions[0]?.session_id ?? null
    if (next !== selectedId) setSelectedId(next)
  }, [available, loading, sessions, selectedId, setSelectedId])

  const [starting, setStarting] = useState(false)
  const [startError, setStartError] = useState<string | null>(null)
  const handleNewSession = async () => {
    if (starting) return
    setStarting(true)
    try {
      const started = await startBrowserSession()
      setStartError(null)
      refresh()
      if (started) setSelectedId(started.session_id)
    } catch (err) {
      setStartError(err instanceof Error ? err.message : String(err))
    } finally {
      setStarting(false)
    }
  }

  const countLabel = loading ? '...' : String(sessions.length)

  return (
    <main
      className="flex-1 flex flex-col min-h-0 overflow-hidden"
      aria-label="browser"
    >
      <header className="shrink-0 px-4 sm:px-6 lg:px-8 py-4 border-b border-rule flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="font-mono text-[16px] font-semibold tracking-[-0.01em] text-ink lowercase">
            browser
          </h1>
          <p className="font-mono text-[12px] text-ink-faint mt-0.5 lowercase">
            {available ? `${countLabel} sessions` : 'worker not connected'}
          </p>
        </div>
        {available ? (
          <div className="flex items-center gap-3">
            <span
              className="flex items-center gap-1.5 font-mono text-[11px] lowercase text-ink-faint"
              title={
                live
                  ? 'updates arrive on the browser session triggers'
                  : 'live bindings unavailable; refreshing on a timer'
              }
            >
              <StatusDot tone={live ? 'accent' : 'ink'} pulse={live} />
              {live ? 'live' : 'polling'}
            </span>
            <Button
              variant="ghost"
              size="sm"
              onClick={refresh}
              disabled={loading}
              className="gap-1.5"
            >
              <RefreshCw
                className={cn('w-3.5 h-3.5', loading && 'animate-spin')}
                aria-hidden
              />
              refresh
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={() => void handleNewSession()}
              disabled={starting}
              className="gap-1.5"
            >
              <Plus className="w-3.5 h-3.5" aria-hidden />
              {starting ? 'starting...' : 'new session'}
            </Button>
          </div>
        ) : null}
      </header>

      {!available ? (
        <div className="flex-1 overflow-auto px-4 sm:px-6 lg:px-8 py-4">
          {status.loading ? (
            <p className="font-mono text-[12px] lowercase text-ink-ghost">
              checking for the browser worker...
            </p>
          ) : (
            <StatusPanel
              variant="info"
              icon={<AlertCircle className="w-full h-full" />}
              headline="browser worker not installed"
              detail="this page needs the optional browser worker. run: iii worker add browser"
            />
          )}
        </div>
      ) : error ? (
        <div className="flex-1 overflow-auto px-4 sm:px-6 lg:px-8 py-4">
          <StatusPanel
            variant="alert"
            icon={<AlertCircle className="w-full h-full" />}
            headline="failed to load browser sessions"
            detail={error}
          />
        </div>
      ) : !loading && sessions.length === 0 ? (
        <div className="flex-1 overflow-auto px-4 sm:px-6 lg:px-8 py-4">
          {startError ? (
            <StatusPanel
              variant="alert"
              icon={<AlertCircle className="w-full h-full" />}
              headline="could not start a session"
              detail={startError}
              className="mb-4"
            />
          ) : null}
          <EmptyState
            icon={Globe}
            title="no browser sessions"
            description="sessions started by agents appear here automatically. start one yourself with the new session button above, or ask an agent to call browser::sessions::start"
          />
        </div>
      ) : (
        <div className="flex-1 flex min-h-0">
          <aside className="w-72 shrink-0 border-r border-rule overflow-y-auto">
            {startError ? (
              <p className="px-3 py-2 border-b border-rule-2 font-mono text-[11px] text-alert">
                {startError}
              </p>
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
              session={selected}
              enabled={available}
              onSessionsRefresh={refresh}
              onStopped={() => setSelectedId(null)}
            />
          ) : (
            <div className="flex-1 flex items-center justify-center">
              <p className="font-mono text-[12px] lowercase text-ink-ghost">
                select a session
              </p>
            </div>
          )}
        </div>
      )}
    </main>
  )
}

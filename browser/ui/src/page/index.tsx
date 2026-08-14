/**
 * The browser page (#/ext/browser): the standard page chrome (PageShell/
 * PageHeader from @iii-dev/console-ui) over a session rail and the selected
 * session's workspace — a screencast-fed live viewport with the console and
 * network feeds — so a user can watch what an agent is doing in a Chromium
 * session, drive the page directly, and pick elements into the clipboard.
 *
 * The host only mounts this page while the browser worker is connected, so
 * there is no presence gate here (worker disconnect disposes the script and
 * drops the nav entry). Session selection is page-local state — the injected
 * page owns its own view, and the host owns the `#/ext/browser` route.
 *
 * Layout adapts to the width the page HAS (a ResizeObserver on its own body
 * row, not a viewport media query — the console can host it in panes of any
 * size). Wide: the rail (start control + session list) is a fixed navigation
 * column beside the session workspace. Under NARROW_BELOW px it becomes a
 * drill-in flow: the session list fills the width, and opening a session
 * swaps it for the full-width workspace with a ← back button. The screencast
 * subscription only runs while the viewport is actually visible (see
 * SessionView), so a narrow pane parked on the list streams nothing.
 */

import { Button, type Host, PageHeader, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
import type { ComponentType } from 'react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { errorMessage, readBrowserDoctor, startBrowserSession } from '../lib/browser'
import { Plus } from '../lib/icons'
import { GlobeIcon, LivePill, RefreshButton, useContainerNarrow } from '../lib/widgets'
import { SessionRail } from './SessionRail'
import { SessionView } from './SessionView'
import { useBrowserSessionsLive } from './useBrowserSessionsLive'

/** Container width (px) below which the page collapses to the drill-in
 * session-list ⇄ workspace flow. */
const NARROW_BELOW = 720

export function BrowserPage({
  host,
  panelSide = 'left',
  tabId = '',
  onRequestClose,
}: { host: Host } & Partial<PageRenderProps>) {
  const [configOpen, setConfigOpen] = useState(false)
  const ConfigurationDialog = host.components.WorkerConfigurationDialog as
    | ComponentType<{
        configurationId: string | null
        onClose: () => void
      }>
    | undefined
  const { sessions, loading, error, live, refresh } = useBrowserSessionsLive(host, true)
  const [chromiumVersion, setChromiumVersion] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void readBrowserDoctor(host.iii)
      .then((doctor) => {
        if (!cancelled) setChromiumVersion(doctor?.chromium_version ?? null)
      })
      .catch(() => {
        // Version is useful context, not a requirement for operating a session.
      })
    return () => {
      cancelled = true
    }
  }, [host])

  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)
  // Narrow-mode drill-in: true once a session was explicitly opened (row
  // click or a start), false after ← back. Wide mode renders both panes
  // regardless, so the flag is harmless there.
  const [drilled, setDrilled] = useState(false)

  // A session selected the moment it starts is not in the list yet; hold it
  // until the refresh lands so the selection does not bounce to the first
  // session and then away again.
  const pendingIdRef = useRef<string | null>(null)

  // Auto-select the first session when nothing (or a stopped session) is
  // selected, and clear a selection whose session is gone.
  useEffect(() => {
    if (loading) return
    setSelectedId((current) => {
      if (current && sessions.some((s) => s.session_id === current)) {
        pendingIdRef.current = null
        return current
      }
      if (current && current === pendingIdRef.current) return current
      pendingIdRef.current = null
      return sessions[0]?.session_id ?? null
    })
  }, [loading, sessions])

  const selected = useMemo(() => sessions.find((s) => s.session_id === selectedId) ?? null, [sessions, selectedId])

  // The drilled-into session can die underneath us (stopped from chat or
  // another tab): drill back out to the list rather than silently showing
  // whichever session the selection fell back to. A session still waiting
  // to appear in the list (pendingIdRef) is not dead — keep the workspace.
  useEffect(() => {
    if (!drilled || selectedId === null) return
    const alive = sessions.some((s) => s.session_id === selectedId)
    if (!alive && pendingIdRef.current !== selectedId) setDrilled(false)
  }, [sessions, drilled, selectedId])

  const [starting, setStarting] = useState(false)
  const [startError, setStartError] = useState<string | null>(null)
  const handleNewSession = async () => {
    if (starting) return
    setStarting(true)
    try {
      const started = await startBrowserSession(host.iii)
      setStartError(null)
      refresh()
      if (started) {
        pendingIdRef.current = started.session_id
        setSelectedId(started.session_id)
        setDrilled(true)
      }
    } catch (err) {
      setStartError(errorMessage(err))
    } finally {
      setStarting(false)
    }
  }

  const openSession = useCallback((sessionId: string) => {
    setSelectedId(sessionId)
    setDrilled(true)
  }, [])

  // Narrow: one pane at a time — the rail or the opened session workspace.
  const stageVisible = !narrow || (drilled && selected !== null)
  const railVisible = !narrow || !stageVisible

  return (
    <PageShell className="br-ui-shell">
      <PageHeader
        icon={<GlobeIcon />}
        title="Browser"
        description="live Chromium sessions you can watch and drive"
        actions={
          <div className="br-ui-header-actions">
            <LivePill live={live} />
            {ConfigurationDialog ? (
              <Button variant="ghost" size="sm" onClick={() => setConfigOpen(true)}>
                Configure
              </Button>
            ) : null}
          </div>
        }
        onClose={onRequestClose}
      />

      {error ? (
        <div className="br-ui-banner alert" role="alert">
          <span>
            The session list could not be loaded.
            <span className="detail">{error}</span>
          </span>
          <button type="button" className="br-ui-linkish" onClick={refresh}>
            retry
          </button>
        </div>
      ) : null}

      <div className={`br-ui-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`} ref={rootRef}>
        {railVisible ? (
          <aside className="br-ui-rail" aria-label="session list">
            <div className="br-ui-rail-top">
              <div className="br-ui-rail-intro">
                <div className="br-ui-rail-intro-copy">
                  <h2>Sessions</h2>
                  <p>Active local and attached browsers.</p>
                </div>
                <Button variant="primary" size="sm" onClick={() => void handleNewSession()} disabled={starting}>
                  <Plus size={14} aria-hidden />
                  {starting ? 'Starting...' : 'New session'}
                </Button>
              </div>
              {startError ? <p className="br-ui-rail-err">{startError}</p> : null}
            </div>
            <header className="br-ui-col-head">
              <span className="label">active now</span>
              <span className="spacer" />
              {loading && sessions.length === 0 ? null : <span className="count">{sessions.length}</span>}
              <RefreshButton onClick={refresh} label="refresh sessions" disabled={loading} spinning={loading} />
            </header>
            <div className="br-ui-rail-scroll">
              <SessionRail sessions={sessions} selectedId={selectedId} loading={loading} onSelect={openSession} />
            </div>
          </aside>
        ) : null}

        {stageVisible ? (
          selected ? (
            <SessionView
              // Remount per session so drafts, pick mode, and type buffers
              // never leak across sessions.
              key={selected.session_id}
              host={host}
              session={selected}
              chromiumVersion={chromiumVersion}
              enabled
              narrow={narrow}
              tabId={tabId}
              onBack={() => setDrilled(false)}
              onSessionsRefresh={refresh}
              onStopped={() => {
                setSelectedId(null)
                setDrilled(false)
              }}
            />
          ) : (
            <section className="br-ui-stage" aria-label="session workspace">
              <div className="br-ui-hero">
                <GlobeIcon className="br-ui-hero-icon" />
                <h2 className="br-ui-hero-title">No browser sessions</h2>
                <p className="br-ui-hero-body">
                  Sessions started by agents appear here automatically. Start one from the session rail, or ask an agent
                  to call <code>browser::sessions::start</code>.
                </p>
              </div>
            </section>
          )
        ) : null}
      </div>
      {ConfigurationDialog ? (
        <ConfigurationDialog configurationId={configOpen ? 'browser' : null} onClose={() => setConfigOpen(false)} />
      ) : null}
    </PageShell>
  )
}

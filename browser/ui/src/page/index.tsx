/**
 * The browser page (#/ext/browser): the standard page chrome (PageShell/
 * PageHeader/PageSidebar from @iii-dev/console-ui) over a session rail and the
 * selected session's workspace — a screencast-fed live viewport with the
 * console and network feeds — so a user can watch what an agent is doing in a
 * Chromium session, drive the page directly, and pick elements into the
 * clipboard.
 *
 * The host only mounts this page while the browser worker is connected, so
 * there is no presence gate here (worker disconnect disposes the script and
 * drops the nav entry). Session selection is page-local state — the injected
 * page owns its own view, and the host owns the `#/ext/browser` route.
 *
 * Layout adapts to the width the page HAS (a ResizeObserver on its own body
 * row, not a viewport media query — the console can host it in panes of any
 * size). Wide: the rail (start control + session list) is a collapsible
 * navigation column beside the session workspace. Under NARROW_BELOW px it
 * becomes a drill-in flow: the session list fills the width, and opening a
 * session swaps it for the full-width workspace with a ← back button. The
 * screencast subscription only runs while the viewport is actually visible
 * (see SessionView), so a narrow pane parked on the list streams nothing.
 */

import {
  Button,
  type Host,
  PageHeader,
  type PageRenderProps,
  PageShell,
  PageSidebar,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  errorMessage,
  readBrowserDoctor,
  startBrowserSession,
} from '../lib/browser'
import { Plus } from '../lib/icons'
import {
  GlobeIcon,
  LivePill,
  RefreshButton,
  useContainerNarrow,
} from '../lib/widgets'
import { SavedSetsDialog } from './SavedSetsDialog'
import { SessionRail } from './SessionRail'
import { type SessionActions, SessionView } from './SessionView'
import { useBrowserSessionsLive } from './useBrowserSessionsLive'

/** Container width (px) below which the page collapses to the drill-in
 * session-list ⇄ workspace flow. */
const NARROW_BELOW = 720

export function BrowserPage({
  host,
  panelSide = 'left',
  tabId = '',
  onRequestClose,
  panelContext,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  const { sessions, loading, error, live, refresh } = useBrowserSessionsLive(
    host,
    true,
  )
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

  const selected = useMemo(
    () => sessions.find((s) => s.session_id === selectedId) ?? null,
    [sessions, selectedId],
  )

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
  const handleNewSession = useCallback(async () => {
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
  }, [host, refresh, starting])

  const openSession = useCallback((sessionId: string) => {
    setSelectedId(sessionId)
    setDrilled(true)
  }, [])

  // Per-session verbs (stop / annotate / focus url) live inside SessionView;
  // this ref lets page-level commands reach the mounted session's handlers
  // without lifting their state up.
  const sessionActionsRef = useRef<SessionActions | null>(null)
  const [savedSets, setSavedSets] = useState<{
    open: boolean
    key: string | null
  }>({ open: false, key: null })
  const openSavedSets = useCallback(
    (key: string | null = null) => setSavedSets({ open: true, key }),
    [],
  )

  // A palette "sessions" row (or any other host.panels.open caller) selects
  // a session by id through the standard panelContext channel.
  const appliedContextRef = useRef(0)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    appliedContextRef.current = panelContext.id
    const context = panelContext.context
    const sessionId =
      context && typeof context === 'object' && !Array.isArray(context)
        ? (context as Record<string, unknown>).sessionId
        : null
    if (typeof sessionId === 'string' && sessionId) openSession(sessionId)
    const type =
      context && typeof context === 'object' && !Array.isArray(context)
        ? (context as Record<string, unknown>).type
        : null
    const setKey =
      context && typeof context === 'object' && !Array.isArray(context)
        ? (context as Record<string, unknown>).key
        : null
    if (type === 'saved-set' && typeof setKey === 'string')
      openSavedSets(setKey)
  }, [panelContext, openSession, openSavedSets])

  useEffect(
    () =>
      commands?.register([
        {
          id: 'new-session',
          title: 'New session',
          detail: 'Start a browser session',
          keywords: ['start', 'chromium'],
          run: () => void handleNewSession(),
        },
        {
          id: 'stop-session',
          title: 'Stop session',
          detail: 'Stop the selected browser session',
          keywords: ['close', 'end'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.stop(),
        },
        {
          id: 'annotate',
          title: 'Annotate the view',
          detail: 'Freeze the live view and pin elements with notes',
          keywords: [
            'pin',
            'inspect',
            'pick',
            'element',
            'comment',
            'markup',
            'feedback',
            'screenshot',
          ],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.toggleAnnotate(),
        },
        {
          id: 'send-annotations',
          title: 'Send annotations to the chat',
          detail: 'Attach each pin to the conversation, plus the whole view',
          keywords: ['annotations', 'chat', 'share', 'send'],
          shortcut: 'Mod+Enter',
          firesWhileTyping: true,
          enabled: () =>
            (sessionActionsRef.current?.annotationCount() ?? 0) > 0,
          run: () => sessionActionsRef.current?.sendAnnotations(),
        },
        {
          id: 'download-annotations',
          title: 'Download the annotated picture',
          detail: 'Save the frozen view with its pins as a PNG',
          keywords: ['annotations', 'save', 'png', 'export'],
          enabled: () =>
            (sessionActionsRef.current?.annotationCount() ?? 0) > 0,
          run: () => sessionActionsRef.current?.downloadAnnotations(),
        },
        {
          id: 'find-in-page',
          title: 'Find in page',
          detail: 'Search the page text; Enter steps, Escape closes',
          keywords: ['search', 'text', 'match'],
          shortcut: 'Mod+F',
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.findInPage(),
        },
        {
          id: 'zoom-in',
          title: 'Zoom in',
          keywords: ['zoom', 'bigger', 'scale'],
          shortcut: 'Mod+=',
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.zoom('in'),
        },
        {
          id: 'zoom-out',
          title: 'Zoom out',
          keywords: ['zoom', 'smaller', 'scale'],
          shortcut: 'Mod+-',
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.zoom('out'),
        },
        {
          id: 'zoom-reset',
          title: 'Reset zoom',
          detail: 'Back to 100 %',
          keywords: ['zoom', 'actual', 'size'],
          shortcut: 'Mod+0',
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.zoom('reset'),
        },
        {
          id: 'screenshot',
          title: 'Take a screenshot',
          detail: 'Save the page as a JPEG',
          keywords: ['capture', 'image', 'download'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.takeScreenshot(),
        },
        {
          id: 'screenshot-to-chat',
          title: 'Screenshot to chat',
          detail: 'Attach the page as an image to the conversation',
          keywords: ['capture', 'image', 'attach', 'send'],
          enabled: () =>
            selected !== null &&
            sessionActionsRef.current !== null &&
            typeof host.chat?.compose === 'function',
          run: () => sessionActionsRef.current?.screenshotToChat(),
        },
        {
          id: 'print-pdf',
          title: 'Print to PDF',
          detail: 'Save the page as a PDF',
          keywords: ['print', 'pdf', 'save', 'export'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.printToPdf(),
        },
        {
          id: 'clear-browsing-data',
          title: 'Clear browsing data',
          detail: 'Cookies, cache and storage for this session',
          keywords: ['clear', 'cookies', 'cache', 'storage', 'reset'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.clearData(),
        },
        {
          id: 'device-toolbar',
          title: 'Toggle device toolbar',
          detail: 'Pin the viewport to a device size',
          keywords: ['device', 'responsive', 'mobile', 'viewport', 'width'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.toggleDeviceToolbar(),
        },
        {
          id: 'import-cookies',
          title: 'Import cookies',
          detail: 'Load a JSON or Netscape cookie file into the session',
          keywords: ['cookies', 'import', 'auth', 'session'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.importCookies(),
        },
        {
          id: 'copy-cookies',
          title: 'Copy cookies',
          detail: 'Copy the session cookies as JSON',
          keywords: ['cookies', 'export', 'copy'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.copyCookies(),
        },
        {
          id: 'save-annotations',
          title: 'Save the annotations',
          detail: 'Keep this set; reopen or send it later from Saved annotations',
          keywords: ['annotations', 'save', 'keep'],
          enabled: () =>
            selected !== null &&
            sessionActionsRef.current !== null &&
            (sessionActionsRef.current?.annotationCount() ?? 0) > 0,
          run: () => sessionActionsRef.current?.saveSet(),
        },
        {
          id: 'saved-annotations',
          title: 'Saved annotations',
          detail: 'Sets saved from any session; send or download them again',
          keywords: ['annotations', 'saved', 'sets', 'share'],
          run: () => openSavedSets(),
        },
        {
          id: 'browser-diagnostics',
          title: 'Browser diagnostics',
          detail: 'What Chromium the worker launches and what it allows',
          keywords: ['doctor', 'diagnostics', 'environment', 'chromium'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.showDiagnostics(),
        },
        {
          id: 'clear-annotations',
          title: 'Clear the annotations',
          keywords: ['annotations', 'remove', 'reset'],
          enabled: () =>
            (sessionActionsRef.current?.annotationCount() ?? 0) > 0,
          run: () => sessionActionsRef.current?.clearAnnotations(),
        },
        {
          id: 'focus-url',
          title: 'Focus the address bar',
          detail: 'Type a url to navigate the selected session',
          keywords: ['address', 'navigate', 'url'],
          enabled: () => selected !== null && sessionActionsRef.current !== null,
          run: () => sessionActionsRef.current?.focusUrl(),
        },
      ]),
    [commands, handleNewSession, selected, openSavedSets],
  )

  // Narrow: one pane at a time — the rail or the opened session workspace.
  const stageVisible = !narrow || (drilled && selected !== null)
  const railVisible = !narrow || !stageVisible

  return (
    <PageShell className="br-ui-shell">
      <PageHeader
        icon={<GlobeIcon />}
        title="Browser"
        description="Live Chromium sessions you can watch and drive"
        actions={
          <div className="br-ui-header-actions">
            <LivePill live={live} />
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

      <div
        className={`br-ui-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`}
        ref={rootRef}
      >
        {railVisible ? (
          <PageSidebar
            label="session list"
            side={panelSide}
            collapsible
            storageKey="browser:sessions"
            defaultWidth={300}
            narrow={narrow}
            className="br-ui-rail"
            header={
              <div className="br-ui-rail-intro">
                <div className="br-ui-rail-intro-copy">
                  <h2>Sessions</h2>
                  <p>Active local and attached browsers.</p>
                </div>
                <Button
                  variant="primary"
                  size="sm"
                  onClick={() => void handleNewSession()}
                  disabled={starting}
                  data-autofocus=""
                >
                  <Plus size={16} aria-hidden />
                  {starting ? 'Starting...' : 'New session'}
                </Button>
              </div>
            }
            collapsedActions={
              <>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => void handleNewSession()}
                  disabled={starting}
                  aria-label={
                    starting
                      ? 'starting browser session'
                      : 'new browser session'
                  }
                  title={
                    starting
                      ? 'starting browser session'
                      : 'new browser session'
                  }
                >
                  <Plus size={16} aria-hidden />
                </Button>
                <RefreshButton
                  onClick={refresh}
                  label="refresh sessions"
                  disabled={loading}
                  spinning={loading}
                />
              </>
            }
          >
            {startError ? <p className="br-ui-rail-err">{startError}</p> : null}
            <header className="br-ui-col-head">
              <span className="label">Active now</span>
              <span className="spacer" />
              {loading && sessions.length === 0 ? null : (
                <span className="count">{sessions.length}</span>
              )}
              <RefreshButton
                onClick={refresh}
                label="refresh sessions"
                disabled={loading}
                spinning={loading}
              />
            </header>
            <div className="br-ui-rail-scroll">
              <SessionRail
                sessions={sessions}
                selectedId={selectedId}
                loading={loading}
                onSelect={openSession}
              />
            </div>
          </PageSidebar>
        ) : null}

        {stageVisible ? (
          selected ? (
            <SessionView
              // Remount per session so drafts, pick mode, and type buffers
              // never leak across sessions.
              key={selected.session_id}
              host={host}
              onOpenSavedSets={openSavedSets}
              session={selected}
              chromiumVersion={chromiumVersion}
              enabled
              narrow={narrow}
              tabId={tabId}
              actionsRef={sessionActionsRef}
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
                  Sessions started by agents appear here automatically. Start
                  one from the session rail, or ask an agent to call{' '}
                  <code>browser::sessions::start</code>.
                </p>
              </div>
            </section>
          )
        ) : null}
      </div>
      <SavedSetsDialog
        host={host}
        open={savedSets.open}
        onOpenChange={(next) =>
          setSavedSets((s) => ({ open: next, key: next ? s.key : null }))
        }
        initialKey={savedSets.key}
      />
    </PageShell>
  )
}

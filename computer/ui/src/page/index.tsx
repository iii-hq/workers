/**
 * The `computer` page: the standard page chrome (PageShell/PageHeader
 * from @iii-dev/console-ui) over a session rail and a screencast-fed live
 * desktop. The viewport forwards every click, scroll and keystroke back as
 * `computer::act`, so the page is a working desktop rather than a screenshot
 * gallery. Deliberately NO page-level keyboard shortcuts: while the surface
 * is focused, keys belong to the desktop (shift+esc is the one way out,
 * handled inside the viewport itself).
 *
 * Layout adapts to the width the page HAS (a ResizeObserver on its own body
 * row, not a viewport media query — the console can host it in panes of any
 * size). Wide: the rail (session list, + for a new desktop) is a collapsible
 * navigation column beside the desktop workspace, which shows the open
 * desktop or, with none open, the launcher. Under 720px (useContainerNarrow)
 * it becomes a drill-in flow: the session list fills the width, and opening a
 * session (or +) swaps it for the full-width stage with a ← back button. The
 * screencast subscription only runs while the viewport is actually visible,
 * so a narrow pane parked on the list streams nothing.
 */

import {
  Button,
  Eyebrow,
  type Host,
  IconButton,
  PageHeader,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  StatusBar,
  StatusDot,
  StatusPanel,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, useWorkerLive } from '@iii-dev/console-ui/hooks'
import { ChevronLeft, Monitor, Plus } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  type ActPayload,
  act,
  type ComputerDisplay,
  type ComputerSessionInfo,
  LIFECYCLE_TRIGGERS,
  listDisplays,
  listSessions,
  type StartSessionInput,
  startSession,
  stopSession,
} from '../lib/computer'
import { describeEndpoint, formatAge } from '../lib/format'
import { claimOnPage } from '../overlay/store'
import { Launcher, type LauncherHandle } from './Launcher'
import { SessionRail } from './SessionRail'
import { useLiveFrames } from './useLiveFrames'
import { Viewport } from './Viewport'

/** Poll cadence while the lifecycle trigger bindings are unavailable (SDK
 * hiccup, races around worker restart); skipped while the tab is hidden. */
const SESSIONS_POLL_MS = 10_000
const NO_SESSIONS: ComputerSessionInfo[] = []

export function ComputerPage({
  host,
  panelSide = 'left',
  onRequestClose,
  panelContext,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  // `computer::sessions::list`, re-read on session-started / session-stopped.
  const { data, loading, error, live, refresh } = useWorkerLive({
    iii: host.iii,
    triggers: LIFECYCLE_TRIGGERS,
    fetch: () => listSessions(host.iii),
    pollMs: SESSIONS_POLL_MS,
    handlerId: 'iii::computer-ui::lifecycle',
  })
  const sessions = data ?? NO_SESSIONS
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [displays, setDisplays] = useState<ComputerDisplay[]>([])
  const [starting, setStarting] = useState(false)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const { ref: rootRef, narrow } = useContainerNarrow()
  // Narrow-mode drill-in: true once a session was explicitly opened (row
  // click or a start), false after ← back. Wide mode renders both panes
  // regardless, so the flag is harmless there.
  const [drilled, setDrilled] = useState(false)
  // The stage shows the launcher instead of the selected desktop (+, or no
  // desktop open at all).
  const [launching, setLaunching] = useState(false)

  // A session selected the moment it starts is not in the list yet; hold it
  // until the refresh lands so the selection does not bounce back to the old
  // session and then away again.
  const pendingIdRef = useRef<string | null>(null)

  // Selection follows the session list: keep the current pick while it lives,
  // otherwise fall back to the newest session.
  useEffect(() => {
    setSelectedId((current) => {
      const live = current && sessions.some((s) => s.session_id === current)
      if (live) {
        pendingIdRef.current = null
        return current
      }
      if (current && current === pendingIdRef.current) return current
      pendingIdRef.current = null
      return sessions.length > 0
        ? sessions[sessions.length - 1].session_id
        : null
    })
  }, [sessions])

  // The drilled-into session can die underneath us (stopped from chat or
  // another tab): drill back out to the list rather than silently showing
  // whichever session the selection fell back to. A session still waiting
  // to appear in the list (pendingIdRef) is not dead — keep the viewport.
  useEffect(() => {
    if (!drilled || launching || selectedId === null) return
    const alive = sessions.some((s) => s.session_id === selectedId)
    if (!alive && pendingIdRef.current !== selectedId) setDrilled(false)
  }, [sessions, drilled, launching, selectedId])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const found = await listDisplays(host.iii).catch(
        () => [] as ComputerDisplay[],
      )
      if (!cancelled) setDisplays(found)
    })()
    return () => {
      cancelled = true
    }
  }, [host])

  const selected = useMemo(
    () => sessions.find((s) => s.session_id === selectedId) ?? null,
    [sessions, selectedId],
  )

  // The stage holds the launcher until a desktop is open, or while + asks
  // for a new one.
  const showLauncher = launching || selected === null
  // Narrow: one pane at a time — the rail (list) or the stage.
  const stageVisible = !narrow || (drilled && (launching || selected !== null))
  const railVisible = !narrow || !stageVisible

  const {
    frame,
    loading: frameLoading,
    error: frameError,
  } = useLiveFrames(host, selectedId, stageVisible && !showLauncher)
  // The desktop shown here gets no corner preview in this window, even when
  // its own viewport input fires computer::activity.
  useEffect(() => {
    if (stageVisible && !showLauncher && selectedId)
      return claimOnPage(selectedId)
  }, [stageVisible, showLauncher, selectedId])

  const runAct = useCallback(
    (payload: ActPayload) => {
      if (!selectedId) return
      void act(host.iii, selectedId, payload).catch((err) => {
        setActionError(errorMessage(err))
      })
    },
    [host, selectedId],
  )

  const openSession = useCallback((sessionId: string) => {
    setSelectedId(sessionId)
    setLaunching(false)
    setDrilled(true)
  }, [])

  const openLauncher = useCallback(() => {
    setLaunching(true)
    setDrilled(true)
  }, [])

  const handleStart = async (input: StartSessionInput) => {
    setStarting(true)
    setActionError(null)
    try {
      const started = await startSession(host.iii, input)
      pendingIdRef.current = started.session_id
      setSelectedId(started.session_id)
      setLaunching(false)
      setDrilled(true)
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setStarting(false)
      refresh()
    }
  }

  // One banner: whatever the last action said, else whatever the list said.
  const problem = actionError ?? error

  const handleStop = async (sessionId: string) => {
    setBusyId(sessionId)
    try {
      await stopSession(host.iii, sessionId)
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusyId(null)
      refresh()
    }
  }

  // The launcher owns its own mode/fields; a page-level command submits
  // whatever the user currently has set through this handle, or opens it.
  const launcherRef = useRef<LauncherHandle>(null)

  // A palette "computer-sessions" row (or any other host.panels.open caller)
  // selects a session by id through the standard panelContext channel.
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
  }, [panelContext, openSession])

  useEffect(
    () =>
      commands?.register([
        {
          id: 'start-session',
          title: 'Start session',
          detail:
            'Start a desktop with the launcher settings, or open the launcher',
          keywords: ['new', 'desktop'],
          enabled: () => !starting,
          run: () =>
            launcherRef.current ? launcherRef.current.submit() : openLauncher(),
        },
        {
          id: 'stop-session',
          title: 'Stop session',
          detail: 'Stop the selected desktop session',
          keywords: ['close', 'end'],
          enabled: () => selected !== null && busyId === null,
          run: () => {
            if (selected && busyId === null)
              void handleStop(selected.session_id)
          },
        },
      ]),
    [commands, starting, selected, busyId, openLauncher],
  )

  return (
    <PageShell className="cp-ui-shell">
      <PageHeader
        icon={<Monitor size={16} aria-hidden />}
        title="Computer"
        description="Live desktops you can watch and drive"
        actions={
          <span
            className="cp-ui-live"
            title={
              live
                ? 'live — subscribed to the session lifecycle triggers; the rail updates as sessions start and stop'
                : 'polling — lifecycle trigger bindings unavailable; the session list refreshes every 10s'
            }
          >
            <StatusDot tone={live ? 'ok' : 'ink'} pulse={live} aria-hidden />
            {live ? 'live' : 'polling'}
          </span>
        }
        onClose={onRequestClose}
      />

      {problem ? (
        <StatusPanel
          variant="alert"
          role="alert"
          className="cp-ui-problem"
          headline={problem}
          action={
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                setActionError(null)
                refresh()
              }}
            >
              Dismiss
            </Button>
          }
        />
      ) : null}

      <div
        className={`cp-ui-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`}
        ref={rootRef}
      >
        {railVisible ? (
          <PageSidebar
            label="sessions"
            side={panelSide}
            collapsible
            storageKey="computer:sessions"
            defaultWidth={280}
            narrow={narrow}
            className="cp-ui-rail"
            header={
              <div className="cp-ui-col-head">
                <Eyebrow as="span">sessions</Eyebrow>
                {loading && sessions.length === 0 ? null : (
                  <span className="count">{sessions.length}</span>
                )}
                <IconButton label="New desktop" onClick={openLauncher}>
                  <Plus size={16} aria-hidden />
                </IconButton>
              </div>
            }
          >
            <div className="cp-ui-rail-scroll">
              <SessionRail
                sessions={sessions}
                selectedId={showLauncher ? null : selectedId}
                loading={loading}
                busyId={busyId}
                onSelect={openSession}
                onStop={(id) => void handleStop(id)}
                onNew={narrow ? openLauncher : undefined}
              />
            </div>
          </PageSidebar>
        ) : null}

        {stageVisible ? (
          <section className="cp-ui-stage" aria-label="desktop workspace">
            {!showLauncher && selected ? (
              <>
                <header className="cp-ui-doc-head">
                  {narrow ? (
                    <IconButton
                      label="back to session list"
                      onClick={() => setDrilled(false)}
                    >
                      <ChevronLeft size={16} aria-hidden />
                    </IconButton>
                  ) : null}
                  <div className="cp-ui-doc-identity">
                    <span
                      className="cp-ui-doc-name"
                      title={selected.session_id}
                    >
                      <span className="txt">{selected.session_id}</span>
                    </span>
                    {!narrow ? (
                      <span
                        className="cp-ui-doc-crumb"
                        title={selected.endpoint}
                      >
                        {describeEndpoint(selected.endpoint)} · {selected.os}
                      </span>
                    ) : null}
                  </div>
                </header>
                <div className="cp-ui-stage-body">
                  <Viewport
                    frame={frame}
                    loading={frameLoading}
                    error={frameError}
                    interactive
                    onClickAt={(x, y, button) =>
                      runAct({
                        action: button === 'right' ? 'right_click' : 'click',
                        x,
                        y,
                      })
                    }
                    onDoubleClickAt={(x, y) =>
                      runAct({ action: 'double_click', x, y })
                    }
                    onScrollAt={(x, y, notches) =>
                      runAct({ action: 'scroll', x, y, scroll_y: notches })
                    }
                    onTextInput={(text) => runAct({ action: 'type', text })}
                    onPressKeys={(keys) => runAct({ action: 'press', keys })}
                  />
                </div>
                <StatusBar
                  as="footer"
                  className="cp-ui-status"
                  end={
                    <>
                      <span className="cp-ui-hint">
                        Click the desktop to control it
                      </span>
                      <span className="cp-ui-hint">Shift+Esc releases it</span>
                    </>
                  }
                >
                  <span className="cp-ui-fact">
                    {selected.screen.width}x{selected.screen.height}
                  </span>
                  <span className="cp-ui-fact">
                    {formatAge(selected.last_used_ms)}
                  </span>
                </StatusBar>
              </>
            ) : (
              <>
                {narrow && sessions.length > 0 ? (
                  <header className="cp-ui-doc-head">
                    <IconButton
                      label="back to session list"
                      onClick={() => {
                        setLaunching(false)
                        setDrilled(false)
                      }}
                    >
                      <ChevronLeft size={16} aria-hidden />
                    </IconButton>
                    <span className="cp-ui-doc-name">
                      <span className="txt">New desktop</span>
                    </span>
                  </header>
                ) : null}
                <div className="cp-ui-stage-launch">
                  <Launcher
                    ref={launcherRef}
                    host={host}
                    displays={displays}
                    starting={starting}
                    onStart={(input) => void handleStart(input)}
                  />
                </div>
              </>
            )}
          </section>
        ) : null}
      </div>
    </PageShell>
  )
}

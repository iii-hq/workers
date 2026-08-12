/**
 * The `#/ext/computer` page: the standard page chrome (PageShell/PageHeader
 * from @iii-dev/console-ui) over a session rail and a screencast-fed live
 * desktop. The viewport forwards every click, scroll and keystroke back as
 * `computer::act`, so the page is a working desktop rather than a screenshot
 * gallery. Deliberately NO page-level keyboard shortcuts: while the surface
 * is focused, keys belong to the desktop (shift+esc is the one way out,
 * handled inside the viewport itself).
 *
 * Layout adapts to the width the page HAS (a ResizeObserver on its own body
 * row, not a viewport media query — the console can host it in panes of any
 * size). Wide: the rail (start form + session list) is a fixed navigation
 * column beside the desktop workspace. Under NARROW_BELOW px it becomes a
 * drill-in flow: the session list fills the width, and opening a session
 * swaps it for the full-width viewport with a ← back button. The screencast
 * subscription only runs while the viewport is actually visible, so a
 * narrow pane parked on the list streams nothing.
 */

import {
  type Host,
  PageHeader,
  type PageRenderProps,
  PageShell,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  type ActPayload,
  act,
  type ComputerDisplay,
  listDisplays,
  type StartSessionInput,
  startSession,
  stopSession,
} from '../lib/computer'
import { errorMessage } from '../lib/errors'
import { formatAge, shortEndpoint } from '../lib/format'
import { BackButton, LivePill, MonitorIcon } from '../lib/widgets'
import { SessionRail } from './SessionRail'
import { StartSessionForm } from './StartSessionForm'
import { useLiveFrames } from './useLiveFrames'
import { useSessionsLive } from './useSessionsLive'
import { Viewport } from './Viewport'

/** Container width (px) below which the page collapses to the drill-in
 * session-list ⇄ viewport flow. */
const NARROW_BELOW = 720

/** Observe the page body's own width. Returns a callback ref to put on the
 * body row plus whether it is currently narrower than `threshold` —
 * container-driven, so the same page adapts inside any pane the console
 * gives it. Measures synchronously on mount to avoid a wide-mode flash;
 * zero widths (display:none) are ignored so a hidden page keeps its last
 * real layout. */
function useContainerNarrow(threshold: number): [(node: HTMLDivElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)
  const refCb = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return
      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < threshold)
      const observer = new ResizeObserver((entries) => {
        const next = entries[0]?.contentRect.width
        if (typeof next === 'number' && next > 0) setNarrow(next < threshold)
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [threshold],
  )
  return [refCb, narrow]
}

export function ComputerPage({
  host,
  panelSide = 'left',
  onRequestClose,
}: { host: Host } & Partial<PageRenderProps>) {
  const { sessions, loading, error, live, refresh } = useSessionsLive(
    host,
    true,
  )
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [displays, setDisplays] = useState<ComputerDisplay[]>([])
  const [starting, setStarting] = useState(false)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)
  // Narrow-mode drill-in: true once a session was explicitly opened (row
  // click or a start), false after ← back. Wide mode renders both panes
  // regardless, so the flag is harmless there.
  const [drilled, setDrilled] = useState(false)

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
    if (!drilled || selectedId === null) return
    const alive = sessions.some((s) => s.session_id === selectedId)
    if (!alive && pendingIdRef.current !== selectedId) setDrilled(false)
  }, [sessions, drilled, selectedId])

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

  // Narrow: one pane at a time — the rail (start form + list) or the
  // opened desktop.
  const stageVisible = !narrow || (drilled && selected !== null)
  const railVisible = !narrow || !stageVisible

  const {
    frame,
    loading: frameLoading,
    error: frameError,
  } = useLiveFrames(host, selectedId, stageVisible)

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
    setDrilled(true)
  }, [])

  const handleStart = async (input: StartSessionInput) => {
    setStarting(true)
    setActionError(null)
    try {
      const started = await startSession(host.iii, input)
      pendingIdRef.current = started.session_id
      setSelectedId(started.session_id)
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

  return (
    <PageShell className="cp-ui-shell">
      <PageHeader
        icon={<MonitorIcon />}
        title="computer"
        description="live desktops you can watch and drive"
        actions={<LivePill live={live} />}
        onClose={onRequestClose}
      />

      {problem ? (
        <div className="cp-ui-banner alert" role="alert">
          <span>{problem}</span>
          <button
            type="button"
            className="cp-ui-linkish"
            onClick={() => {
              setActionError(null)
              refresh()
            }}
          >
            dismiss
          </button>
        </div>
      ) : null}

      <div
        className={`cp-ui-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`}
        ref={rootRef}
      >
        {railVisible ? (
          <aside className="cp-ui-rail" aria-label="session list">
            <div className="cp-ui-rail-top">
              <div className="cp-ui-rail-caption">start a session</div>
              <StartSessionForm
                displays={displays}
                starting={starting}
                onStart={(input) => void handleStart(input)}
              />
            </div>
            <header className="cp-ui-col-head">
              <span className="label">sessions</span>
              <span className="spacer" />
              {loading && sessions.length === 0 ? null : (
                <span className="count">{sessions.length}</span>
              )}
            </header>
            <div className="cp-ui-rail-scroll">
              <SessionRail
                sessions={sessions}
                selectedId={selectedId}
                loading={loading}
                busyId={busyId}
                onSelect={openSession}
                onStop={(id) => void handleStop(id)}
              />
            </div>
          </aside>
        ) : null}

        {stageVisible ? (
          <section className="cp-ui-stage" aria-label="desktop workspace">
            {selected ? (
              <>
                <header className="cp-ui-doc-head">
                  {narrow ? (
                    <BackButton
                      onClick={() => setDrilled(false)}
                      label="back to session list"
                    />
                  ) : null}
                  <div className="cp-ui-doc-identity">
                    <span
                      className="cp-ui-doc-name"
                      title={selected.session_id}
                    >
                      <span className="txt">{selected.session_id}</span>
                    </span>
                    {!narrow ? (
                      <span className="cp-ui-doc-crumb">
                        {shortEndpoint(selected.endpoint)} · {selected.os}
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
                <footer className="cp-ui-statusbar">
                  <span className="fact">
                    {selected.screen.width}x{selected.screen.height}
                  </span>
                  <span className="fact">
                    {formatAge(selected.last_used_ms)}
                  </span>
                  <span className="spacer" />
                  <span className="fact hint">
                    click to focus — clicks, scroll, typing and shortcuts
                    forward as act
                  </span>
                  <span className="fact hint">shift+esc leaves the surface</span>
                </footer>
              </>
            ) : (
              <div className="cp-ui-hero">
                <MonitorIcon className="cp-ui-hero-icon" />
                <h2 className="cp-ui-hero-title">no desktop yet</h2>
                <p className="cp-ui-hero-body">
                  start a session to drive this machine, a sandboxed desktop,
                  or a remote one.
                </p>
              </div>
            )}
          </section>
        ) : null}
      </div>
    </PageShell>
  )
}

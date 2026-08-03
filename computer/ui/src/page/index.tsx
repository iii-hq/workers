import { Badge, Button, EmptyState, type Host } from '@iii-dev/console-ui'
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
import { shortEndpoint } from '../lib/format'
import { SessionRail } from './SessionRail'
import { StartSessionForm } from './StartSessionForm'
import { useLiveFrames } from './useLiveFrames'
import { useSessionsLive } from './useSessionsLive'
import { Viewport } from './Viewport'

/**
 * The `#/ext/computer` page: session rail on the left, live desktop on the
 * right. The viewport is fed by the screencast stream and forwards every
 * click, scroll and keystroke back as `computer::act`, so the page is a
 * working desktop rather than a screenshot gallery.
 */

export function ComputerPage({ host }: { host: Host }) {
  const { sessions, loading, error, live, refresh } = useSessionsLive(
    host,
    true,
  )
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [displays, setDisplays] = useState<ComputerDisplay[]>([])
  const [starting, setStarting] = useState(false)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

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

  const {
    frame,
    loading: frameLoading,
    error: frameError,
  } = useLiveFrames(host, selectedId, true)

  const runAct = useCallback(
    (payload: ActPayload) => {
      if (!selectedId) return
      void act(host.iii, selectedId, payload).catch((err) => {
        setActionError(errorMessage(err))
      })
    },
    [host, selectedId],
  )

  const handleStart = async (input: StartSessionInput) => {
    setStarting(true)
    setActionError(null)
    try {
      const started = await startSession(host.iii, input)
      pendingIdRef.current = started.session_id
      setSelectedId(started.session_id)
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setStarting(false)
      refresh()
    }
  }

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
    <div className="cp-ui-page">
      <header className="cp-ui-page-head">
        <div className="cp-ui-page-title">
          <span className="cp-ui-page-name">computer</span>
          <Badge variant={live ? 'accent' : 'default'} className="cp-ui-pill">
            {live ? 'live' : 'polling'}
          </Badge>
          {selected ? (
            <span className="cp-ui-page-sub">
              {selected.session_id} · {shortEndpoint(selected.endpoint)} ·{' '}
              {selected.screen.width}x{selected.screen.height}
            </span>
          ) : null}
        </div>
        <StartSessionForm
          displays={displays}
          starting={starting}
          onStart={(input) => void handleStart(input)}
        />
      </header>

      {error || actionError ? (
        <p className="cp-ui-page-error">
          {actionError ?? error}
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setActionError(null)
              refresh()
            }}
          >
            dismiss
          </Button>
        </p>
      ) : null}

      <div className="cp-ui-page-body">
        <aside className="cp-ui-rail">
          <SessionRail
            sessions={sessions}
            selectedId={selectedId}
            loading={loading}
            busyId={busyId}
            onSelect={setSelectedId}
            onStop={(id) => void handleStop(id)}
          />
        </aside>
        <section className="cp-ui-stage">
          {selected ? (
            <>
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
              <p className="cp-ui-stage-hint">
                click the desktop to focus it: clicks, scroll, typing and
                shortcuts forward as <code className="cp-ui-code">act</code>
              </p>
            </>
          ) : (
            <EmptyState
              title="no desktop yet"
              description="start a session to drive this machine, a sandboxed desktop, or a remote one."
            />
          )}
        </section>
      </div>
    </div>
  )
}

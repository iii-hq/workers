/**
 * Everything for one selected session — the workspace beside (or, when
 * narrow, instead of) the rail: a document header carrying the session's
 * identity plus the pick-to-clipboard and stop controls, the URL bar, the
 * screencast-fed viewport letterboxed in the workspace, the console /
 * network feeds, and a status bar with the input-forwarding hints.
 *
 * Wide: the viewport takes the workspace and the feeds dock under it behind
 * a console | network segmented control. Narrow: a viewport | console |
 * network segmented control shows one pane at a time, and the screencast
 * subscription only runs while the viewport segment is actually visible.
 *
 * Pick-to-chat became pick-to-clipboard: the injected UI has no composer
 * slot (host.composer is unimplemented), so a picked element's summary is
 * copied to the clipboard for the user to paste into chat.
 *
 * The page remounts this component per session (React key), so all state
 * here — url draft, pick mode, type buffer, pane choices — is session-local.
 */

import { Button, type Host, Input, SegmentedControl } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  BROWSER_PICKED_TRIGGER,
  type BrowserClickOptions,
  type BrowserPickedEvent,
  type BrowserSessionInfo,
  clickBrowserAt,
  controlBrowserHistory,
  errorMessage,
  formatPickedElement,
  hintBrowserPick,
  navigateBrowser,
  parsePickedEvent,
  pickedSelector,
  pressBrowserKey,
  resolveBrowserPick,
  scrollBrowserAt,
  startBrowserPick,
  stopBrowserPick,
  stopBrowserSession,
  typeBrowserText,
} from '../lib/browser'
import { cn } from '../lib/cn'
import { useBrowserSessionEvent } from '../lib/events'
import { formatMtime } from '../lib/format'
import { Crosshair, ExternalLink, Globe, RefreshCw, X } from '../lib/icons'
import { BackButton, ChevronLeftIcon } from '../lib/widgets'
import { ConsolePanel } from './ConsolePanel'
import { NetworkPanel } from './NetworkPanel'
import { useLiveFrames } from './useLiveFrames'
import { Viewport } from './Viewport'

const PICKED_FN = 'iii::browser-ui::picked'
const TYPE_FLUSH_MS = 200

type FeedPane = 'console' | 'network'
type NarrowPane = 'viewport' | FeedPane

const FEED_PANES: readonly FeedPane[] = ['console', 'network']
const NARROW_PANES: readonly NarrowPane[] = ['viewport', 'console', 'network']

function readStored(key: string): string | null {
  try {
    return window.localStorage.getItem(key)
  } catch {
    return null
  }
}

function writeStored(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value)
  } catch {
    /* private mode / quota — persistence is best-effort */
  }
}

/** Same page-title derivation as the rail rows. */
function hostOf(url: string): string {
  try {
    const parsed = new URL(url)
    return parsed.host || url
  } catch {
    return url
  }
}

interface SessionViewProps {
  host: Host
  session: BrowserSessionInfo
  chromiumVersion: string | null
  enabled: boolean
  narrow: boolean
  /** Stable workspace-tab id — namespaces persisted UI state. */
  tabId: string
  onBack: () => void
  onSessionsRefresh: () => void
  onStopped: () => void
}

export function SessionView({
  host,
  session,
  chromiumVersion,
  enabled,
  narrow,
  tabId,
  onBack,
  onSessionsRefresh,
  onStopped,
}: SessionViewProps) {
  const sessionId = session.session_id
  const [picking, setPicking] = useState(false)

  // Narrow-mode segment (viewport | console | network); session-local, so a
  // drill-in always lands on the viewport.
  const [narrowPane, setNarrowPane] = useState<NarrowPane>('viewport')
  // Wide-mode feeds dock (console | network), persisted per workspace tab.
  const dockStoreKey = `browser-ui:${tabId || 'page'}:dock`
  const [dockPane, setDockPaneState] = useState<FeedPane>(() =>
    readStored(dockStoreKey) === 'network' ? 'network' : 'console',
  )
  const dockCollapsedStoreKey = `browser-ui:${tabId || 'page'}:dock-collapsed`
  const [dockCollapsed, setDockCollapsedState] = useState(
    () => readStored(dockCollapsedStoreKey) === 'true',
  )
  const setDockPane = (pane: FeedPane) => {
    setDockPaneState(pane)
    writeStored(dockStoreKey, pane)
    if (dockCollapsed) {
      setDockCollapsedState(false)
      writeStored(dockCollapsedStoreKey, 'false')
    }
  }
  const toggleDock = () => {
    setDockCollapsedState((current) => {
      const next = !current
      writeStored(dockCollapsedStoreKey, String(next))
      return next
    })
  }

  // The screencast subscription is gated on the viewport actually being
  // visible: wide mode always shows it, narrow only on its segment.
  const viewportShown = !narrow || narrowPane === 'viewport'
  const live = useLiveFrames(host, sessionId, enabled && viewportShown)

  const [actionError, setActionError] = useState<string | null>(null)
  const runAction = useCallback(async (action: () => Promise<void>) => {
    try {
      await action()
      setActionError(null)
    } catch (err) {
      setActionError(errorMessage(err))
    }
  }, [])

  // URL bar: mirrors the session's committed url, but never clobbers
  // in-progress typing (focus-aware sync).
  const [urlDraft, setUrlDraft] = useState(session.url)
  const urlFocusedRef = useRef(false)
  const lastSessionUrlRef = useRef(session.url)
  useEffect(() => {
    if (lastSessionUrlRef.current === session.url) return
    lastSessionUrlRef.current = session.url
    if (!urlFocusedRef.current) setUrlDraft(session.url)
  }, [session.url])
  useEffect(() => {
    setUrlDraft(session.url)
    lastSessionUrlRef.current = session.url
  }, [sessionId])

  const submitUrl = useCallback(() => {
    let url = urlDraft.trim()
    if (!url) return
    // Require a real scheme (`scheme://`) to skip the prefix; a bare
    // `host:port` like localhost:3000 is not a scheme and must get https://.
    if (!/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(url)) url = `https://${url}`
    void runAction(async () => {
      await navigateBrowser(host.iii, sessionId, url)
      onSessionsRefresh()
    })
  }, [host, urlDraft, sessionId, runAction, onSessionsRefresh])

  const handleHistory = useCallback(
    (action: 'back' | 'forward' | 'reload') => {
      void runAction(async () => {
        const result = await controlBrowserHistory(host.iii, sessionId, action)
        if (result?.url) setUrlDraft(result.url)
        onSessionsRefresh()
      })
    },
    [host, sessionId, runAction, onSessionsRefresh],
  )

  const openCurrentPage = useCallback(() => {
    let url = urlDraft.trim() || session.url
    if (url && !/^[a-zA-Z][a-zA-Z0-9+.-]*:\/\//.test(url))
      url = `https://${url}`
    if (url) window.open(url, '_blank', 'noopener,noreferrer')
  }, [session.url, urlDraft])

  // Pick-to-clipboard. The worker auto-exits inspect mode after one pick, so
  // a received event only flips local state; explicit toggles and unmounts
  // send pick::stop.
  const [lastPicked, setLastPicked] = useState<BrowserPickedEvent | null>(null)
  const pickingRef = useRef(false)
  pickingRef.current = picking

  useEffect(() => {
    setPicking(false)
    setLastPicked(null)
    setActionError(null)
    return () => {
      if (pickingRef.current) {
        void stopBrowserPick(host.iii, sessionId).catch(() => {})
      }
    }
  }, [host, sessionId])

  const togglePick = useCallback(() => {
    if (picking) {
      setPicking(false)
      void runAction(() => stopBrowserPick(host.iii, sessionId))
      return
    }
    void runAction(async () => {
      await startBrowserPick(host.iii, sessionId)
      setPicking(true)
    })
  }, [host, picking, sessionId, runAction])

  useEffect(() => {
    if (!picking) return
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      setPicking(false)
      void stopBrowserPick(host.iii, sessionId).catch(() => {})
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [host, picking, sessionId])

  useBrowserSessionEvent({
    host,
    enabled: enabled && picking,
    triggerType: BROWSER_PICKED_TRIGGER,
    sessionId,
    fnId: PICKED_FN,
    onEvent: (payload) => {
      const evt = parsePickedEvent(payload)
      if (!evt || evt.session_id !== sessionId) return
      setLastPicked(evt)
      // No composer slot in injected UI: copy the summary for the user to
      // paste into chat.
      void navigator.clipboard
        ?.writeText(formatPickedElement(evt))
        .catch(() => {})
      setPicking(false)
    },
  })

  const handleClickAt = useCallback(
    (x: number, y: number, options?: BrowserClickOptions) => {
      void runAction(() => clickBrowserAt(host.iii, sessionId, x, y, options))
    },
    [host, sessionId, runAction],
  )

  // Pick-mode click: resolve the element at the exact clicked point (the
  // worker emits browser::picked), so what the hover highlight showed is
  // what gets picked.
  const handlePickAt = useCallback(
    (x: number, y: number) => {
      void runAction(() => resolveBrowserPick(host.iii, sessionId, x, y))
    },
    [host, sessionId, runAction],
  )

  const handleScrollAt = useCallback(
    (x: number, y: number, deltaY: number) => {
      void runAction(() => scrollBrowserAt(host.iii, sessionId, x, y, deltaY))
    },
    [host, sessionId, runAction],
  )

  // Printable characters batch into one type act per idle window; a special
  // key flushes the pending text first so the page sees keystrokes in order.
  const typeBufferRef = useRef('')
  const typeTimerRef = useRef<number | undefined>(undefined)
  const takeTypeBuffer = useCallback(() => {
    window.clearTimeout(typeTimerRef.current)
    typeTimerRef.current = undefined
    const text = typeBufferRef.current
    typeBufferRef.current = ''
    return text
  }, [])

  const flushTypeBuffer = useCallback(() => {
    const text = takeTypeBuffer()
    if (!text) return
    void runAction(() => typeBrowserText(host.iii, sessionId, text))
  }, [host, takeTypeBuffer, sessionId, runAction])

  const handleTextInput = useCallback(
    (text: string) => {
      typeBufferRef.current += text
      window.clearTimeout(typeTimerRef.current)
      typeTimerRef.current = window.setTimeout(flushTypeBuffer, TYPE_FLUSH_MS)
    },
    [flushTypeBuffer],
  )

  const handlePressKey = useCallback(
    (key: string) => {
      const text = takeTypeBuffer()
      void runAction(async () => {
        if (text) await typeBrowserText(host.iii, sessionId, text)
        await pressBrowserKey(host.iii, sessionId, key)
      })
    },
    [host, takeTypeBuffer, sessionId, runAction],
  )

  // Flush any buffered text before the session changes or the component
  // unmounts, so keystrokes typed against one session are sent to that
  // session rather than dropped (or leaking into the next one).
  useEffect(() => {
    return () => {
      flushTypeBuffer()
    }
  }, [flushTypeBuffer])

  const requestHint = useCallback(
    (x: number, y: number) => hintBrowserPick(host.iii, sessionId, x, y),
    [host, sessionId],
  )

  const handleStop = useCallback(() => {
    void runAction(async () => {
      await stopBrowserSession(host.iii, sessionId)
      onSessionsRefresh()
      onStopped()
    })
  }, [host, sessionId, runAction, onSessionsRefresh, onStopped])

  const displayName =
    session.title?.trim() || hostOf(session.url) || 'about:blank'
  const feedPane: FeedPane = narrow
    ? narrowPane === 'network'
      ? 'network'
      : 'console'
    : dockPane
  const browserMajor = chromiumVersion?.match(/\d+/)?.[0]
  const browserLabel = browserMajor ? `Chromium ${browserMajor}` : null

  return (
    <section
      className="br-ui-stage"
      aria-label={`browser session ${sessionId}`}
    >
      <header className="br-ui-doc-head">
        {narrow ? (
          <BackButton onClick={onBack} label="back to session list" />
        ) : null}
        <div className="br-ui-doc-identity">
          <div className="br-ui-doc-title-row">
            <span
              className="br-ui-doc-name"
              title={`${sessionId} · ${session.url}`}
            >
              <span className="txt">{displayName}</span>
            </span>
            <span className="br-ui-doc-badge">
              {session.headless ? 'headless' : 'headful'}
            </span>
            {!narrow && browserLabel ? (
              <span className="br-ui-doc-badge">{browserLabel}</span>
            ) : null}
          </div>
          <span className="br-ui-doc-crumb">
            <span className="br-ui-doc-url">{session.url}</span>
            <span aria-hidden>·</span>
            <span className="br-ui-doc-live">
              <span className="br-ui-live-dot" aria-hidden />
              live
            </span>
            {!narrow ? (
              <>
                <span aria-hidden>·</span>
                <span>
                  started {formatMtime(Math.floor(session.created_ms / 1000))}
                </span>
              </>
            ) : null}
          </span>
        </div>
        <div className="br-ui-doc-actions">
          <button
            type="button"
            onClick={togglePick}
            aria-pressed={picking}
            title={
              picking
                ? 'pick mode on: click an element in the viewport to copy it to the clipboard (esc cancels)'
                : 'pick an element to the clipboard'
            }
            className={cn('br-ui-pick-btn', picking && 'is-on')}
          >
            <Crosshair size={16} aria-hidden />
            {picking ? 'Inspecting...' : 'Inspect'}
          </button>
          <Button
            variant="ghost"
            size="sm"
            className="br-ui-stop-btn"
            onClick={handleStop}
          >
            Stop session
          </Button>
        </div>
      </header>

      {lastPicked ? (
        <div className="br-ui-picked">
          <span className="br-ui-picked-label">Picked</span>
          <span
            className="br-ui-picked-chip"
            title={lastPicked.element.outer_html}
          >
            <span className="br-ui-picked-ref">{lastPicked.element.ref}</span>
            <span className="br-ui-truncate">
              {pickedSelector(lastPicked.element)}
            </span>
            <button
              type="button"
              onClick={() => setLastPicked(null)}
              aria-label="dismiss picked element"
              className="br-ui-picked-x"
            >
              <X size={16} aria-hidden />
            </button>
          </span>
          <span className="br-ui-picked-note">Copied to clipboard</span>
        </div>
      ) : null}

      {actionError ? (
        <div className="br-ui-banner alert" role="alert">
          <span>{actionError}</span>
          <button
            type="button"
            className="br-ui-linkish quiet"
            onClick={() => setActionError(null)}
          >
            dismiss
          </button>
        </div>
      ) : null}

      {narrow ? (
        <div className="br-ui-view-row">
          <SegmentedControl<NarrowPane>
            value={narrowPane}
            onChange={setNarrowPane}
            options={NARROW_PANES.map((pane) => ({
              value: pane,
              label:
                pane === 'console'
                  ? 'Console'
                  : pane === 'network'
                    ? 'Network'
                    : 'Viewport',
            }))}
            className="br-ui-tabs"
            aria-label="Session view"
          />
        </div>
      ) : null}

      {viewportShown ? (
        <div className="br-ui-stage-body">
          <div className="br-ui-browser-frame">
            <form
              className="br-ui-toolbar"
              onSubmit={(event) => {
                event.preventDefault()
                submitUrl()
              }}
            >
              <fieldset
                className="br-ui-history-controls"
                aria-label="browser history controls"
              >
                <button
                  type="button"
                  className="br-ui-chrome-btn"
                  onClick={() => handleHistory('back')}
                  title="back"
                  aria-label="back"
                >
                  <ChevronLeftIcon className="br-ui-chrome-icon" />
                </button>
                <button
                  type="button"
                  className="br-ui-chrome-btn"
                  onClick={() => handleHistory('forward')}
                  title="forward"
                  aria-label="forward"
                >
                  <ChevronLeftIcon className="br-ui-chrome-icon is-forward" />
                </button>
                <button
                  type="button"
                  className="br-ui-chrome-btn"
                  onClick={() => handleHistory('reload')}
                  title="reload"
                  aria-label="reload"
                >
                  <RefreshCw size={16} aria-hidden />
                </button>
              </fieldset>
              <div className="br-ui-address">
                <Globe size={16} aria-hidden className="br-ui-address-icon" />
                <Input
                  name="browser-url"
                  value={urlDraft}
                  onChange={setUrlDraft}
                  preserveCase
                  placeholder="https://localhost:3000"
                  aria-label="page url"
                  onFocus={() => {
                    urlFocusedRef.current = true
                  }}
                  onBlur={() => {
                    urlFocusedRef.current = false
                  }}
                  className="br-ui-url-input"
                />
              </div>
              <button
                type="button"
                className="br-ui-chrome-btn"
                onClick={openCurrentPage}
                title="open page in a new tab"
                aria-label="open page in a new tab"
              >
                <ExternalLink size={17} aria-hidden />
              </button>
              <button
                type="submit"
                className="br-ui-address-submit"
                tabIndex={-1}
              >
                navigate to address
              </button>
            </form>
            <Viewport
              frame={live.frame}
              loading={live.loading}
              error={live.error}
              picking={picking}
              onClickAt={handleClickAt}
              onPickAt={handlePickAt}
              onScrollAt={handleScrollAt}
              onTextInput={handleTextInput}
              onPressKey={handlePressKey}
              requestHint={requestHint}
            />
          </div>
        </div>
      ) : (
        <div className="br-ui-pane-fill">
          {feedPane === 'console' ? (
            <ConsolePanel host={host} sessionId={sessionId} enabled={enabled} />
          ) : (
            <NetworkPanel host={host} sessionId={sessionId} enabled={enabled} />
          )}
        </div>
      )}

      {!narrow ? (
        <div className={`br-ui-dock${dockCollapsed ? ' collapsed' : ''}`}>
          <div className="br-ui-dock-head">
            <SegmentedControl<FeedPane>
              value={dockPane}
              onChange={setDockPane}
              options={FEED_PANES.map((pane) => ({
                value: pane,
                label: pane === 'console' ? 'Console' : 'Network',
              }))}
              className="br-ui-tabs"
              aria-label="Session feeds"
            />
            <button
              type="button"
              className="br-ui-dock-toggle"
              aria-expanded={!dockCollapsed}
              aria-label={
                dockCollapsed ? 'show developer tools' : 'hide developer tools'
              }
              title={
                dockCollapsed ? 'show developer tools' : 'hide developer tools'
              }
              onClick={toggleDock}
            >
              {dockCollapsed ? (
                <>
                  <span>Show developer tools</span>
                  <ChevronLeftIcon className="br-ui-dock-toggle-icon" />
                </>
              ) : (
                <X size={16} aria-hidden />
              )}
            </button>
          </div>
          {!dockCollapsed ? (
            <div className="br-ui-dock-body">
              {dockPane === 'console' ? (
                <ConsolePanel
                  host={host}
                  sessionId={sessionId}
                  enabled={enabled}
                />
              ) : (
                <NetworkPanel
                  host={host}
                  sessionId={sessionId}
                  enabled={enabled}
                />
              )}
            </div>
          ) : null}
        </div>
      ) : null}

      <footer className="br-ui-statusbar">
        {live.frame ? (
          <span className="fact">
            Viewport: {live.frame.width}×{live.frame.height}
          </span>
        ) : (
          <span className="fact">Viewport: —</span>
        )}
        <span className="fact">
          {session.headless ? 'Headless' : 'Headful'}
        </span>
        {browserLabel ? <span className="fact">{browserLabel}</span> : null}
        <span className="fact live">
          <span className="br-ui-live-dot" aria-hidden />
          live
        </span>
        <span className="spacer" />
        {viewportShown ? (
          picking ? (
            <span className="fact hint">
              pick mode: click an element to copy it — esc cancels
            </span>
          ) : (
            <>
              <span className="fact hint">Click to focus</span>
              <span className="fact hint">Scroll or type to interact</span>
              <span className="fact hint">Shift+Esc to release</span>
            </>
          )
        ) : null}
      </footer>
    </section>
  )
}

import { Button, type Host, Input, Tabs, TabsContent, TabsList, TabsTrigger } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  BROWSER_PICKED_TRIGGER,
  type BrowserClickOptions,
  type BrowserPickedEvent,
  type BrowserSessionInfo,
  clickBrowserAt,
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
import { Crosshair, Square, X } from '../lib/icons'
import { ConsolePanel } from './ConsolePanel'
import { NetworkPanel } from './NetworkPanel'
import { useLiveFrames } from './useLiveFrames'
import { Viewport } from './Viewport'

/**
 * Everything for one selected session: the URL bar, pick-to-clipboard toggle,
 * stop control, the screencast-fed viewport, and the console / network tabs.
 *
 * Pick-to-chat became pick-to-clipboard: the injected UI has no composer slot
 * (host.composer is unimplemented), so a picked element's summary is copied
 * to the clipboard for the user to paste into chat.
 */

const PICKED_FN = 'iii::browser-ui::picked'
const TYPE_FLUSH_MS = 200

interface SessionViewProps {
  host: Host
  session: BrowserSessionInfo
  enabled: boolean
  onSessionsRefresh: () => void
  onStopped: () => void
}

export function SessionView({
  host,
  session,
  enabled,
  onSessionsRefresh,
  onStopped,
}: SessionViewProps) {
  const sessionId = session.session_id
  const [picking, setPicking] = useState(false)
  const live = useLiveFrames(host, sessionId, enabled)

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
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset the draft when the selected session changes, not when its url does
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

  return (
    <section className="br-ui-session" aria-label={`browser session ${sessionId}`}>
      <div className="br-ui-toolbar">
        <Input
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
          onKeyDown={(e) => {
            if (e.key === 'Enter') submitUrl()
          }}
          className="br-ui-url-input"
        />
        <Button variant="pill" size="sm" onClick={submitUrl}>
          go
        </Button>
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
          <Crosshair size={13} aria-hidden />
          {picking ? 'picking...' : 'pick element'}
        </button>
        <Button variant="ghost" size="sm" onClick={handleStop}>
          <Square size={12} aria-hidden />
          stop
        </Button>
      </div>

      {lastPicked ? (
        <div className="br-ui-picked">
          <span className="br-ui-picked-label">picked</span>
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
              <X size={12} aria-hidden />
            </button>
          </span>
          <span className="br-ui-picked-note">copied to clipboard</span>
        </div>
      ) : null}

      {actionError ? (
        <p className="br-ui-session-err">{actionError}</p>
      ) : null}

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

      <Tabs defaultValue="console" className="br-ui-tabs">
        <TabsList className="br-ui-tabs-list">
          <TabsTrigger value="console">console</TabsTrigger>
          <TabsTrigger value="network">network</TabsTrigger>
        </TabsList>
        <TabsContent value="console" className="br-ui-tabs-content">
          <ConsolePanel host={host} sessionId={sessionId} enabled={enabled} />
        </TabsContent>
        <TabsContent value="network" className="br-ui-tabs-content">
          <NetworkPanel host={host} sessionId={sessionId} enabled={enabled} />
        </TabsContent>
      </Tabs>
    </section>
  )
}

import { Crosshair, Square, X } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import { useBrowserSessionEvent } from '@/hooks/use-browser-events'
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
} from '@/lib/browser'
import { insertIntoComposer } from '@/lib/composer-insert'
import { cn } from '@/lib/utils'
import { useLiveFrames } from '../hooks/useLiveFrames'
import { ConsolePanel } from './ConsolePanel'
import { NetworkPanel } from './NetworkPanel'
import { Viewport } from './Viewport'

/**
 * Everything for one selected session: the URL bar, pick-to-chat toggle,
 * stop control, the screencast-fed viewport, and the console / network
 * tabs.
 */

const PICKED_FN = 'console::browser-picked'
const TYPE_FLUSH_MS = 200

interface SessionViewProps {
  session: BrowserSessionInfo
  enabled: boolean
  onSessionsRefresh: () => void
  onStopped: () => void
}

export function SessionView({
  session,
  enabled,
  onSessionsRefresh,
  onStopped,
}: SessionViewProps) {
  const sessionId = session.session_id
  const [picking, setPicking] = useState(false)
  const live = useLiveFrames(sessionId, enabled)

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
    if (!/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(url)) url = `https://${url}`
    void runAction(async () => {
      await navigateBrowser(sessionId, url)
      onSessionsRefresh()
    })
  }, [urlDraft, sessionId, runAction, onSessionsRefresh])

  // Pick-to-chat. The worker auto-exits inspect mode after one pick, so a
  // received event only flips local state; explicit toggles and unmounts
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
        void stopBrowserPick(sessionId).catch(() => {})
      }
    }
  }, [sessionId])

  const togglePick = useCallback(() => {
    if (picking) {
      setPicking(false)
      void runAction(() => stopBrowserPick(sessionId))
      return
    }
    void runAction(async () => {
      await startBrowserPick(sessionId)
      setPicking(true)
    })
  }, [picking, sessionId, runAction])

  useEffect(() => {
    if (!picking) return
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      setPicking(false)
      void stopBrowserPick(sessionId).catch(() => {})
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [picking, sessionId])

  useBrowserSessionEvent({
    enabled: enabled && picking,
    triggerType: BROWSER_PICKED_TRIGGER,
    sessionId,
    fnId: PICKED_FN,
    onEvent: (payload) => {
      const evt = parsePickedEvent(payload)
      if (!evt || evt.session_id !== sessionId) return
      setLastPicked(evt)
      insertIntoComposer(formatPickedElement(evt))
      setPicking(false)
    },
  })

  const handleClickAt = useCallback(
    (x: number, y: number, options?: BrowserClickOptions) => {
      void runAction(() => clickBrowserAt(sessionId, x, y, options))
    },
    [sessionId, runAction],
  )

  // Pick-mode click: resolve the element at the exact clicked point (the
  // worker emits browser::picked), so what the hover highlight showed is
  // what gets picked.
  const handlePickAt = useCallback(
    (x: number, y: number) => {
      void runAction(() => resolveBrowserPick(sessionId, x, y))
    },
    [sessionId, runAction],
  )

  const handleScrollAt = useCallback(
    (x: number, y: number, deltaY: number) => {
      void runAction(() => scrollBrowserAt(sessionId, x, y, deltaY))
    },
    [sessionId, runAction],
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
    void runAction(() => typeBrowserText(sessionId, text))
  }, [takeTypeBuffer, sessionId, runAction])

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
        if (text) await typeBrowserText(sessionId, text)
        await pressBrowserKey(sessionId, key)
      })
    },
    [takeTypeBuffer, sessionId, runAction],
  )

  useEffect(() => {
    return () => {
      window.clearTimeout(typeTimerRef.current)
    }
  }, [])

  const requestHint = useCallback(
    (x: number, y: number) => hintBrowserPick(sessionId, x, y),
    [sessionId],
  )

  const handleStop = useCallback(() => {
    void runAction(async () => {
      await stopBrowserSession(sessionId)
      onSessionsRefresh()
      onStopped()
    })
  }, [sessionId, runAction, onSessionsRefresh, onStopped])

  return (
    <section
      className="flex-1 flex flex-col min-w-0 min-h-0"
      aria-label={`browser session ${sessionId}`}
    >
      <div className="shrink-0 flex items-center gap-2 px-3 py-2 border-b border-rule">
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
          className="h-8 text-[12px] flex-1 min-w-0"
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
              ? 'pick mode on: click an element in the viewport to send it to chat (esc cancels)'
              : 'pick an element into chat'
          }
          className={cn(
            'font-mono text-[12px] lowercase h-8 px-2.5 border inline-flex items-center gap-1.5 transition-colors',
            picking
              ? 'bg-accent text-accent-fg border-accent'
              : 'bg-transparent text-ink-faint border-rule hover:text-ink hover:border-ink',
          )}
        >
          <Crosshair size={13} aria-hidden />
          {picking ? 'picking...' : 'pick element'}
        </button>
        <Button
          variant="ghost"
          size="sm"
          onClick={handleStop}
          className="gap-1.5"
        >
          <Square size={12} aria-hidden />
          stop
        </Button>
      </div>

      {lastPicked ? (
        <div className="shrink-0 flex items-center gap-2 px-3 py-1.5 border-b border-rule-2 bg-paper-2">
          <span className="font-mono text-[11px] lowercase text-ink-faint shrink-0">
            picked
          </span>
          <span
            className="inline-flex items-center gap-x-2 border border-rule bg-bg px-2 py-0.5 font-mono text-[12px] text-ink min-w-0"
            title={lastPicked.element.outer_html}
          >
            <span className="text-accent shrink-0">
              {lastPicked.element.ref}
            </span>
            <span className="truncate min-w-0">
              {pickedSelector(lastPicked.element)}
            </span>
            <button
              type="button"
              onClick={() => setLastPicked(null)}
              aria-label="dismiss picked element"
              className="text-ink-faint hover:text-accent transition-colors shrink-0"
            >
              <X size={12} aria-hidden />
            </button>
          </span>
          <span className="font-mono text-[11px] lowercase text-ink-ghost truncate">
            sent to the chat composer
          </span>
        </div>
      ) : null}

      {actionError ? (
        <p className="shrink-0 px-3 py-1.5 border-b border-rule-2 font-mono text-[12px] text-alert">
          {actionError}
        </p>
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

      <Tabs
        defaultValue="console"
        className="shrink-0 h-[38%] min-h-44 flex flex-col border-t border-rule"
      >
        <TabsList className="px-3 shrink-0">
          <TabsTrigger value="console">console</TabsTrigger>
          <TabsTrigger value="network">network</TabsTrigger>
        </TabsList>
        <TabsContent value="console" className="flex-1 min-h-0">
          <ConsolePanel sessionId={sessionId} enabled={enabled} />
        </TabsContent>
        <TabsContent value="network" className="flex-1 min-h-0">
          <NetworkPanel sessionId={sessionId} enabled={enabled} />
        </TabsContent>
      </Tabs>
    </section>
  )
}

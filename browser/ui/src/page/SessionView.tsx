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
import type { RefObject } from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  BROWSER_NAVIGATED_TRIGGER,
  BROWSER_PICKED_TRIGGER,
  type BrowserClickOptions,
  type BrowserFindAction,
  type BrowserSessionInfo,
  clickBrowserAt,
  controlBrowserHistory,
  downloadFile,
  errorMessage,
  fileFromBase64,
  findInBrowserPage,
  hintBrowserPick,
  navigateBrowser,
  parsePickedEvent,
  pinLabel,
  pressBrowserKey,
  printBrowserPageToPdf,
  resolveBrowserPick,
  screenshotFileName,
  scrollBrowserAt,
  stopBrowserSession,
  takeBrowserScreenshot,
  typeBrowserText,
  zoomBrowserPage,
} from '../lib/browser'
import { cn } from '../lib/cn'
import { useBrowserSessionEvent } from '../lib/events'
import { formatMtime } from '../lib/format'
import {
  ExternalLink,
  Globe,
  MessageSquarePlus,
  RefreshCw,
  X,
} from '../lib/icons'
import { BackButton, ChevronLeftIcon } from '../lib/widgets'
import {
  type Annotation,
  type AnnotationSet,
  addAnnotation,
  annotationFileName,
  annotationPinFileName,
  annotationsMarkdown,
  labelAnnotation,
  moveAnnotation,
  noteAnnotation,
  removeAnnotation,
  renderAnnotatedImage,
  renderAnnotationCrop,
} from './annotations'
import { ConsolePanel } from './ConsolePanel'
import { FindBar, type FindState } from './FindBar'
import { NetworkPanel } from './NetworkPanel'
import { PageMenu } from './PageMenu'
import { type LiveFrame, useLiveFrames } from './useLiveFrames'
import { Viewport } from './Viewport'

const PICKED_FN = 'iii::browser-ui::picked'
const NAVIGATED_FN = 'iii::browser-ui::navigated'
const FIND_DEBOUNCE_MS = 150
const TYPE_FLUSH_MS = 200

type FeedPane = 'console' | 'network'
type NarrowPane = 'viewport' | FeedPane

/** Verbs the page's commands reach through a ref, since they close over
 * this component's session-local state. */
export interface SessionActions {
  stop: () => void
  focusUrl: () => void
  toggleAnnotate: () => void
  annotating: () => boolean
  annotationCount: () => number
  sendAnnotations: () => void
  downloadAnnotations: () => void
  clearAnnotations: () => void
  findInPage: () => void
  zoom: (action: 'in' | 'out' | 'reset') => void
  takeScreenshot: () => void
  screenshotToChat: () => void
  printToPdf: () => void
}

const FEED_PANES: readonly FeedPane[] = ['console', 'network']
const NARROW_PANES: readonly NarrowPane[] = ['viewport', 'console', 'network']
const PANE_LABELS: Record<NarrowPane, string> = {
  viewport: 'Viewport',
  console: 'Console',
  network: 'Network',
}

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
  /** Populated with this session's stop/inspect/focus-url verbs while
   * mounted, so the page's commands can reach them. */
  actionsRef?: RefObject<SessionActions | null>
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
  actionsRef,
  onBack,
  onSessionsRefresh,
  onStopped,
}: SessionViewProps) {
  const sessionId = session.session_id

  // Narrow-mode segment (viewport | console | network); session-local, so a
  // drill-in always lands on the viewport.
  const [narrowPane, setNarrowPane] = useState<NarrowPane>('viewport')
  // Wide-mode feeds dock (console | network), persisted per workspace tab.
  const dockStoreKey = `browser-ui:${tabId || 'page'}:dock`
  const [dockPane, setDockPaneState] = useState<FeedPane>(() => {
    const stored = readStored(dockStoreKey)
    return stored === 'network' ? stored : 'console'
  })
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

  useEffect(() => {
    setActionError(null)
  }, [sessionId])

  // Annotate mode freezes the frame the pins sit on; the live view resumes
  // when the mode ends. The pins outlive the mode until sent or cleared.
  const [annotating, setAnnotating] = useState(false)
  const [frozen, setFrozen] = useState<LiveFrame | null>(null)
  const [annotations, setAnnotations] = useState<Annotation[]>([])
  const [selectedAnnotation, setSelectedAnnotation] = useState<string | null>(
    null,
  )
  const [sending, setSending] = useState(false)
  const unlabeledPinsRef = useRef<string[]>([])
  useEffect(() => {
    setAnnotating(false)
    setFrozen(null)
    setAnnotations([])
    setSelectedAnnotation(null)
    unlabeledPinsRef.current = []
  }, [sessionId])
  const liveFrameRef = useRef(live.frame)
  liveFrameRef.current = live.frame
  const toggleAnnotate = useCallback(() => {
    setAnnotating((current) => {
      if (current) return false
      // Re-entering with unsent marks resumes them on their own frozen
      // frame; only a fresh session starts from the live frame.
      if (annotationsRef.current.length > 0 && frozenRef.current) return true
      const frame = liveFrameRef.current
      if (!frame) {
        setActionError('no frame to annotate yet')
        return false
      }
      setFrozen(frame)
      setAnnotations([])
      setSelectedAnnotation(null)
      unlabeledPinsRef.current = []
      return true
    })
  }, [])
  const annotationSet = useCallback((): AnnotationSet | null => {
    if (!frozen) return null
    return {
      subject: session.url,
      imageUrl: frozen.dataUrl,
      width: frozen.width,
      height: frozen.height,
      annotations,
      capturedAt: Date.now(),
    }
  }, [frozen, session.url, annotations])
  const sendAnnotations = useCallback(() => {
    const set = annotationSet()
    if (!set || set.annotations.length === 0 || !host.chat?.compose) return
    setSending(true)
    void runAction(async () => {
      const whole = new File(
        [await renderAnnotatedImage(set)],
        annotationFileName(set, 'png'),
        { type: 'image/png' },
      )
      const pins = await Promise.all(
        set.annotations.map(async (pin, index) => {
          const blob = await renderAnnotationCrop(set, pin.id)
          return new File([blob], annotationPinFileName(pin, index, 'png'), {
            type: 'image/png',
          })
        }),
      )
      host.chat?.compose?.({
        text: annotationsMarkdown(set),
        files: [whole, ...pins],
      })
      setAnnotating(false)
      setAnnotations([])
      unlabeledPinsRef.current = []
    }).finally(() => setSending(false))
  }, [annotationSet, host, runAction])
  const downloadAnnotations = useCallback(() => {
    const set = annotationSet()
    if (!set || set.annotations.length === 0) return
    void runAction(async () => {
      const blob = await renderAnnotatedImage(set)
      const url = URL.createObjectURL(blob)
      const link = document.createElement('a')
      link.href = url
      link.download = annotationFileName(set, 'png')
      link.click()
      window.setTimeout(() => URL.revokeObjectURL(url), 1000)
    })
  }, [annotationSet, runAction])
  const clearAnnotations = useCallback(() => {
    setAnnotations([])
    setSelectedAnnotation(null)
    unlabeledPinsRef.current = []
  }, [])
  const viewportAnnotation = annotating
    ? {
        annotations,
        selectedId: selectedAnnotation,
        onAdd: (x: number, y: number) => {
          const next = addAnnotation(annotationsRef.current, x, y)
          const pin = next[next.length - 1]
          setAnnotations(next)
          setSelectedAnnotation(pin?.id ?? null)
          if (!pin || !frozen) return
          unlabeledPinsRef.current.push(pin.id)
          void resolveBrowserPick(
            host.iii,
            sessionId,
            Math.min(frozen.width - 1, Math.round(x * frozen.width)),
            Math.min(frozen.height - 1, Math.round(y * frozen.height)),
          ).catch(() => {
            unlabeledPinsRef.current = unlabeledPinsRef.current.filter(
              (queued) => queued !== pin.id,
            )
          })
        },
        onSelect: setSelectedAnnotation,
        onMove: (id: string, x: number, y: number) =>
          setAnnotations((list) => moveAnnotation(list, id, x, y)),
        onRemove: (id: string) =>
          setAnnotations((list) => removeAnnotation(list, id)),
        onNote: (id: string, note: string) =>
          setAnnotations((list) => noteAnnotation(list, id, note)),
      }
    : null

  useEffect(() => {
    if (!annotating) return
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      const target = e.target as HTMLElement | null
      // A note field keeps its own Escape; the mode ends from the viewport.
      if (target?.tagName === 'INPUT') return
      e.preventDefault()
      setAnnotating(false)
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [annotating])

  // A dropped pin asks the worker what sits under it (the page is still
  // live under the frozen frame); the answer labels the newest unlabeled
  // pin so the note carries the element it points at.
  // Picked events carry no correlation token, so pins waiting for their
  // element label queue up first-in first-out; two quick drops each get
  // their own answer instead of the second overwriting the first.
  useBrowserSessionEvent({
    host,
    enabled: enabled && annotating,
    triggerType: BROWSER_PICKED_TRIGGER,
    sessionId,
    fnId: PICKED_FN,
    onEvent: (payload) => {
      const evt = parsePickedEvent(payload)
      if (!evt || evt.session_id !== sessionId) return
      const id = unlabeledPinsRef.current.shift()
      if (!id) return
      setAnnotations((list) => labelAnnotation(list, id, pinLabel(evt)))
    },
  })

  const handleClickAt = useCallback(
    (x: number, y: number, options?: BrowserClickOptions) => {
      void runAction(() => clickBrowserAt(host.iii, sessionId, x, y, options))
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

  // Find in page: the worker keeps the match list in the document; this
  // side keeps the query, the count and the current index for the bar.
  const [find, setFind] = useState<FindState | null>(null)
  const findTimerRef = useRef<number | undefined>(undefined)
  // Responses can land out of order while typing; only the newest request
  // may touch the bar.
  const findRevisionRef = useRef(0)
  const runFind = useCallback(
    (query: string, action: BrowserFindAction) => {
      window.clearTimeout(findTimerRef.current)
      const revision = ++findRevisionRef.current
      void findInBrowserPage(host.iii, sessionId, query, action)
        .then((result) => {
          if (revision !== findRevisionRef.current) return
          setFind((current) =>
            current === null
              ? current
              : { ...current, count: result.count, index: result.index },
          )
        })
        .catch((error: unknown) => {
          if (revision !== findRevisionRef.current) return
          setActionError(errorMessage(error))
        })
    },
    [host, sessionId],
  )
  const openFind = useCallback(() => {
    setFind((current) => current ?? { query: '', count: 0, index: 0 })
  }, [])
  const closeFind = useCallback(() => {
    window.clearTimeout(findTimerRef.current)
    findRevisionRef.current += 1
    setFind(null)
    void findInBrowserPage(host.iii, sessionId, '', 'close').catch(() => {})
  }, [host, sessionId])
  const setFindQuery = useCallback(
    (query: string) => {
      setFind((current) => (current ? { ...current, query } : current))
      window.clearTimeout(findTimerRef.current)
      findTimerRef.current = window.setTimeout(
        () => runFind(query, 'search'),
        FIND_DEBOUNCE_MS,
      )
    },
    [runFind],
  )
  const findRef = useRef(find)
  findRef.current = find
  const stepFind = useCallback(
    (direction: 'next' | 'previous') => {
      const current = findRef.current
      if (!current || current.query.trim() === '') return
      runFind(current.query, direction)
    },
    [runFind],
  )
  useEffect(() => {
    setFind(null)
    return () => window.clearTimeout(findTimerRef.current)
  }, [sessionId])

  // Zoom belongs to the loaded document, so a navigation resets it in the
  // page; the level the user chose is re-applied when the next page commits.
  const [zoom, setZoom] = useState(100)
  const zoomRef = useRef(zoom)
  zoomRef.current = zoom
  // The document keeps its zoom across a pane remount; read it back so the
  // menu shows the real level.
  useEffect(() => {
    setZoom(100)
    let cancelled = false
    void zoomBrowserPage(host.iii, sessionId, 'read')
      .then((level) => {
        if (!cancelled) setZoom(level)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [host, sessionId])
  const applyZoom = useCallback(
    (action: 'in' | 'out' | 'reset' | 'set', level?: number) => {
      void runAction(async () => {
        setZoom(await zoomBrowserPage(host.iii, sessionId, action, level))
      })
    },
    [host, sessionId, runAction],
  )
  useBrowserSessionEvent({
    host,
    enabled: enabled && zoom !== 100,
    triggerType: BROWSER_NAVIGATED_TRIGGER,
    sessionId,
    fnId: NAVIGATED_FN,
    onEvent: () => {
      const level = zoomRef.current
      if (level === 100) return
      void zoomBrowserPage(host.iii, sessionId, 'set', level).catch(() => {})
    },
  })

  const takeScreenshot = useCallback(() => {
    void runAction(async () => {
      const shot = await takeBrowserScreenshot(host.iii, sessionId)
      if (!shot?.dataUrl) throw new Error('no image came back')
      downloadFile(
        fileFromBase64(
          shot.dataUrl.split(',')[1] ?? '',
          screenshotFileName(shot.url),
          'image/jpeg',
        ),
      )
    })
  }, [host, sessionId, runAction])
  const screenshotToChat = useCallback(() => {
    if (!host.chat?.compose) return
    void runAction(async () => {
      const shot = await takeBrowserScreenshot(host.iii, sessionId)
      if (!shot?.dataUrl) throw new Error('no image came back')
      host.chat?.compose?.({
        files: [
          fileFromBase64(
            shot.dataUrl.split(',')[1] ?? '',
            screenshotFileName(shot.url),
            'image/jpeg',
          ),
        ],
      })
    })
  }, [host, sessionId, runAction])
  const printToPdf = useCallback(() => {
    void runAction(async () => {
      const pdf = await printBrowserPageToPdf(host.iii, sessionId)
      downloadFile(fileFromBase64(pdf.data, pdf.file_name, 'application/pdf'))
    })
  }, [host, sessionId, runAction])

  const handleStop = useCallback(() => {
    void runAction(async () => {
      await stopBrowserSession(host.iii, sessionId)
      onSessionsRefresh()
      onStopped()
    })
  }, [host, sessionId, runAction, onSessionsRefresh, onStopped])

  const urlInputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (!actionsRef) return
    actionsRef.current = {
      stop: handleStop,
      focusUrl: () => urlInputRef.current?.focus(),
      toggleAnnotate,
      annotating: () => annotatingRef.current,
      annotationCount: () => annotationsRef.current.length,
      sendAnnotations,
      downloadAnnotations,
      clearAnnotations,
      findInPage: openFind,
      zoom: applyZoom,
      takeScreenshot,
      screenshotToChat,
      printToPdf,
    }
    return () => {
      actionsRef.current = null
    }
  }, [
    actionsRef,
    handleStop,
    toggleAnnotate,
    sendAnnotations,
    downloadAnnotations,
    clearAnnotations,
    openFind,
    applyZoom,
    takeScreenshot,
    screenshotToChat,
    printToPdf,
  ])
  const annotatingRef = useRef(annotating)
  annotatingRef.current = annotating
  const annotationsRef = useRef(annotations)
  annotationsRef.current = annotations
  const frozenRef = useRef(frozen)
  frozenRef.current = frozen

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
          {annotations.length > 0 ? (
            <>
              <Button
                variant="primary"
                size="sm"
                onClick={sendAnnotations}
                disabled={sending || typeof host.chat?.compose !== 'function'}
                title="send the pins to the chat, one attachment each (⌘↵)"
              >
                {sending ? 'Sending…' : `Send ${annotations.length} to chat`}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={downloadAnnotations}
                title="save the frozen view with its pins as a PNG"
              >
                Download
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={clearAnnotations}
                title="drop every pin"
              >
                Clear
              </Button>
            </>
          ) : null}
          <button
            type="button"
            onClick={toggleAnnotate}
            aria-pressed={annotating}
            title={
              annotating
                ? 'annotating: click an element to drop a pin on it, esc ends'
                : 'freeze the view and pin elements with notes'
            }
            className={cn('br-ui-pick-btn', annotating && 'is-on')}
          >
            <MessageSquarePlus size={16} aria-hidden />
            {annotating ? 'Annotating' : 'Annotate'}
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
              label: PANE_LABELS[pane],
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
                  ref={urlInputRef}
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
              <PageMenu
                zoom={zoom}
                canSendToChat={typeof host.chat?.compose === 'function'}
                actions={{
                  findInPage: openFind,
                  takeScreenshot,
                  screenshotToChat,
                  printToPdf,
                  zoomIn: () => applyZoom('in'),
                  zoomOut: () => applyZoom('out'),
                  zoomReset: () => applyZoom('reset'),
                }}
              />
              <button
                type="submit"
                className="br-ui-address-submit"
                tabIndex={-1}
              >
                navigate to address
              </button>
            </form>
            {find ? (
              <FindBar
                state={find}
                onQuery={setFindQuery}
                onNext={() => stepFind('next')}
                onPrevious={() => stepFind('previous')}
                onClose={closeFind}
              />
            ) : null}
            <Viewport
              frame={annotating && frozen ? frozen : live.frame}
              loading={live.loading}
              error={live.error}
              annotation={viewportAnnotation}
              onClickAt={handleClickAt}
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
                label: PANE_LABELS[pane],
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
          annotating ? (
            <span className="fact hint">
              annotating: click an element to pin it, esc ends
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

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

import {
  Button,
  ConfirmDialog,
  type Host,
  Input,
  SegmentedControl,
} from '@iii-dev/console-ui'
import type { RefObject } from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  BROWSER_HANDOFF_REQUESTED_TRIGGER,
  BROWSER_HANDOFF_RESOLVED_TRIGGER,
  BROWSER_NAVIGATED_TRIGGER,
  BROWSER_PICKED_TRIGGER,
  type BrowserClickOptions,
  type BrowserFindAction,
  type BrowserHandoffEvent,
  type BrowserSessionInfo,
  clearBrowserData,
  clickBrowserAt,
  confirmBrowserHandoff,
  controlBrowserHistory,
  downloadFile,
  errorMessage,
  fileFromBase64,
  findInBrowserPage,
  elementLabel,
  hintBrowserPick,
  listBrowserCookies,
  navigateBrowser,
  parseCookieFile,
  parseHandoffEvent,
  parseHandoffResolved,
  parsePickedEvent,
  pinLabel,
  pressBrowserKey,
  printBrowserPageToPdf,
  resizeBrowser,
  resolveBrowserPick,
  screenshotFileName,
  scrollBrowserAt,
  setBrowserCookies,
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
  type AnnotationTool,
  addAnnotation,
  addElementMark,
  addShape,
  annotationFileName,
  annotationPinFileName,
  annotationsMarkdown,
  labelAnnotation,
  MIN_SHAPE_SIZE,
  moveAnnotation,
  noteAnnotation,
  removeAnnotation,
  renderAnnotatedImage,
  renderAnnotationCrop,
  resizeAnnotation,
  undoAnnotation,
} from './annotations'
import { ConsolePanel } from './ConsolePanel'
import {
  type DevicePreset,
  type DeviceState,
  DeviceToolbar,
} from './DeviceToolbar'
import { DoctorDialog } from './DoctorDialog'
import { DownloadsPanel, useDownloadCount } from './DownloadsPanel'
import { FindBar, type FindState } from './FindBar'
import { HistoryPanel } from './HistoryPanel'
import { NetworkPanel } from './NetworkPanel'
import { PageMenu } from './PageMenu'
import { type LiveFrame, useLiveFrames } from './useLiveFrames'
import { Viewport } from './Viewport'

const PICKED_FN = 'iii::browser-ui::picked'
const TOOL_HINTS: Record<AnnotationTool, string> = {
  pin: 'Click a spot to drop a numbered pin.',
  select: 'Click an element to box it, labelled with its selector.',
  rect: 'Drag a rectangle, then add a note to it.',
  arrow: 'Drag from tail to head, then add a note to it.',
}
const SHAPE_COLORS = [
  '#e5484d',
  '#f5a623',
  '#30a46c',
  '#0091ff',
  '#8e4ec6',
] as const
const HANDOFF_FN = 'iii::browser-ui::handoff'
const HANDOFF_RESOLVED_FN = 'iii::browser-ui::handoff-resolved'
const NAVIGATED_FN = 'iii::browser-ui::navigated'
const FIND_DEBOUNCE_MS = 150
const TYPE_FLUSH_MS = 200

type FeedPane = 'console' | 'network' | 'downloads' | 'history'
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
  clearData: () => void
  toggleDeviceToolbar: () => void
  importCookies: () => void
  copyCookies: () => void
  showDiagnostics: () => void
}

const FEED_PANES: readonly FeedPane[] = [
  'console',
  'network',
  'downloads',
  'history',
]
const NARROW_PANES: readonly NarrowPane[] = [
  'viewport',
  'console',
  'network',
  'downloads',
  'history',
]
const PANE_LABELS: Record<NarrowPane, string> = {
  viewport: 'Viewport',
  console: 'Console',
  network: 'Network',
  downloads: 'Downloads',
  history: 'History',
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
    return stored === 'network' ||
      stored === 'downloads' ||
      stored === 'history'
      ? (stored as FeedPane)
      : 'console'
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
  const [reseedToken, setReseedToken] = useState(0)
  const live = useLiveFrames(
    host,
    sessionId,
    enabled && viewportShown,
    reseedToken,
  )

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
  const [tool, setTool] = useState<AnnotationTool>('pin')
  const [drawColor, setDrawColor] = useState<string>(SHAPE_COLORS[0])
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
    setTool('pin')
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
  const drawingIdRef = useRef<string | null>(null)
  const viewportAnnotation = annotating
    ? {
        tool,
        drawColor,
        onAddShape: (kind: 'rect' | 'arrow', x: number, y: number) => {
          if (kind !== 'rect' && kind !== 'arrow') return
          const next = addShape(annotationsRef.current, kind, x, y, drawColor)
          const shape = next[next.length - 1]
          drawingIdRef.current = shape?.id ?? null
          setAnnotations(next)
        },
        onResizeShape: (x2: number, y2: number) => {
          const id = drawingIdRef.current
          if (id) setAnnotations((list) => resizeAnnotation(list, id, x2, y2))
        },
        onEndShape: () => {
          const id = drawingIdRef.current
          drawingIdRef.current = null
          if (!id) return
          const shape = annotationsRef.current.find((a) => a.id === id)
          if (!shape) return
          const w = Math.abs((shape.x2 ?? shape.x) - shape.x)
          const h = Math.abs((shape.y2 ?? shape.y) - shape.y)
          if (w < MIN_SHAPE_SIZE && h < MIN_SHAPE_SIZE) {
            setAnnotations((list) => list.filter((a) => a.id !== id))
            return
          }
          // A finished box or arrow opens its note editor, like a dropped
          // pin, so a mark and its message are one gesture.
          setSelectedAnnotation(id)
        },
        annotations,
        selectedId: selectedAnnotation,
        onAdd: (x: number, y: number) => {
          if (!frozen) return
          if (tool === 'select') {
            // The element under the click becomes a box snapped to its
            // bounds, labelled with its selector - the inspector gesture.
            void hintBrowserPick(
              host.iii,
              sessionId,
              Math.min(frozen.width - 1, Math.round(x * frozen.width)),
              Math.min(frozen.height - 1, Math.round(y * frozen.height)),
            )
              .then((hint) => {
                if (!hint?.hit || !hint.bounds) return
                const b = hint.bounds
                setAnnotations(
                  addElementMark(
                    annotationsRef.current,
                    b.x / frozen.width,
                    b.y / frozen.height,
                    (b.x + b.width) / frozen.width,
                    (b.y + b.height) / frozen.height,
                    drawColor,
                    elementLabel(hint.tag, hint.id, hint.classes),
                  ),
                )
              })
              .catch(() => {})
            return
          }
          const next = addAnnotation(annotationsRef.current, x, y)
          const pin = next[next.length - 1]
          setAnnotations(next)
          if (!pin) return
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

  // Match the Chromium viewport to the pane as it resizes, so the streamed
  // frame fills the surface with no letterboxing and clicks map 1:1. The
  // observer fires often; debounce and skip sub-pixel-ish changes.
  const resizeTimerRef = useRef<number | undefined>(undefined)
  const lastSentSizeRef = useRef<{ w: number; h: number } | null>(null)
  const lastPaneSizeRef = useRef<{ w: number; h: number } | null>(null)
  // A pinned device size takes the viewport out of pane-tracking until reset.
  const [device, setDevice] = useState<DeviceState | null>(null)
  const [showDevice, setShowDevice] = useState(false)
  const deviceRef = useRef(device)
  deviceRef.current = device
  const applyViewport = useCallback(
    (
      width: number,
      height: number,
      dpr?: number,
      mobile?: boolean,
      fit?: boolean,
    ) => {
      void resizeBrowser(host.iii, sessionId, width, height, {
        deviceScaleFactor: dpr,
        mobile,
        fit,
      })
        .then((applied) => {
          if (!applied) return
          setReseedToken((t) => t + 1)
          // The worker clamps (200..4000); a pinned device shows the size
          // that actually applies, not the raw typed number.
          setDevice((current) =>
            current &&
            (current.width !== applied.width ||
              current.height !== applied.height)
              ? { ...current, width: applied.width, height: applied.height }
              : current,
          )
        })
        .catch(() => {})
    },
    [host, sessionId],
  )
  const onSurfaceResize = useCallback(
    (width: number, height: number) => {
      lastPaneSizeRef.current = { w: width, h: height }
      // A read-only session's viewport is not ours to change; the frame
      // letterbox-scales instead.
      if (session.read_only === true) return
      if (deviceRef.current) return
      const last = lastSentSizeRef.current
      if (last && Math.abs(last.w - width) < 4 && Math.abs(last.h - height) < 4)
        return
      window.clearTimeout(resizeTimerRef.current)
      resizeTimerRef.current = window.setTimeout(() => {
        lastSentSizeRef.current = { w: width, h: height }
        applyViewport(
          width,
          height,
          Math.min(window.devicePixelRatio || 1, 2),
          undefined,
          true,
        )
      }, 180)
    },
    [applyViewport],
  )
  useEffect(() => {
    lastSentSizeRef.current = null
    setReseedToken(0)
    setDevice(null)
    setShowDevice(false)
    return () => window.clearTimeout(resizeTimerRef.current)
  }, [sessionId])
  const applyDevice = useCallback(
    (preset: DevicePreset) => {
      setDevice({
        width: preset.width,
        height: preset.height,
        deviceScaleFactor: preset.deviceScaleFactor,
        mobile: preset.mobile,
        presetId: preset.id,
      })
      applyViewport(
        preset.width,
        preset.height,
        preset.deviceScaleFactor,
        preset.mobile,
      )
    },
    [applyViewport],
  )
  const setDeviceDimensions = useCallback(
    (width: number, height: number) => {
      setDevice((current) => {
        const base = current ?? {
          width,
          height,
          deviceScaleFactor: 1,
          mobile: false,
          presetId: null,
        }
        const next = { ...base, width, height, presetId: null }
        applyViewport(
          next.width,
          next.height,
          next.deviceScaleFactor,
          next.mobile,
        )
        return next
      })
    },
    [applyViewport],
  )
  const rotateDevice = useCallback(() => {
    setDevice((current) => {
      if (!current) return current
      const next = { ...current, width: current.height, height: current.width }
      applyViewport(
        next.width,
        next.height,
        next.deviceScaleFactor,
        next.mobile,
      )
      return next
    })
  }, [applyViewport])
  const resetDevice = useCallback(() => {
    setDevice(null)
    const pane = lastPaneSizeRef.current
    if (pane) {
      lastSentSizeRef.current = pane
      applyViewport(pane.w, pane.h)
    }
  }, [applyViewport])
  const toggleDeviceToolbar = useCallback(() => {
    setShowDevice((v) => {
      const next = !v
      if (!next) resetDevice()
      return next
    })
  }, [resetDevice])

  // Cookie import: read a JSON or Netscape file and set the cookies.
  const cookieInputRef = useRef<HTMLInputElement>(null)
  const importCookies = useCallback(() => {
    cookieInputRef.current?.click()
  }, [])
  const onCookieFile = useCallback(
    (file: File | undefined) => {
      if (!file) return
      void (async () => {
        try {
          const cookies = parseCookieFile(await file.text())
          if (cookies.length === 0)
            throw new Error('no cookies found in that file')
          const count = await setBrowserCookies(host.iii, sessionId, cookies)
          setActionError(`imported ${count} cookie${count === 1 ? '' : 's'}`)
        } catch (err) {
          setActionError(errorMessage(err))
        }
      })()
    },
    [host, sessionId],
  )
  const copyCookies = useCallback(() => {
    void (async () => {
      try {
        if (!navigator.clipboard) {
          throw new Error('clipboard unavailable in this context')
        }
        const cookies = await listBrowserCookies(host.iii, sessionId)
        await navigator.clipboard.writeText(JSON.stringify(cookies, null, 2))
        setActionError(`copied ${cookies.length} cookies to the clipboard`)
      } catch (err) {
        setActionError(errorMessage(err))
      }
    })()
  }, [host, sessionId])

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

  const downloadCount = useDownloadCount(host, sessionId, enabled)
  const [confirmingClear, setConfirmingClear] = useState(false)
  const [showDoctor, setShowDoctor] = useState(false)
  // Handoffs queue: several can be pending at once, and any of them may
  // resolve out-of-band (in-page click, another caller, timeout) — the
  // resolved event drops it from the queue so the banner never goes stale.
  const [handoffs, setHandoffs] = useState<BrowserHandoffEvent[]>([])
  useEffect(() => setHandoffs([]), [sessionId])
  useBrowserSessionEvent({
    host,
    enabled,
    triggerType: BROWSER_HANDOFF_REQUESTED_TRIGGER,
    sessionId,
    fnId: HANDOFF_FN,
    onEvent: (payload) => {
      const evt = parseHandoffEvent(payload)
      if (!evt || evt.session_id !== sessionId) return
      setHandoffs((queue) =>
        queue.some((h) => h.handoff_id === evt.handoff_id)
          ? queue
          : [...queue, evt],
      )
    },
  })
  useBrowserSessionEvent({
    host,
    enabled,
    triggerType: BROWSER_HANDOFF_RESOLVED_TRIGGER,
    sessionId,
    fnId: HANDOFF_RESOLVED_FN,
    onEvent: (payload) => {
      const evt = parseHandoffResolved(payload)
      if (!evt || evt.session_id !== sessionId) return
      setHandoffs((queue) =>
        queue.filter((h) => h.handoff_id !== evt.handoff_id),
      )
    },
  })
  const handoff = handoffs[0] ?? null
  const confirmHandoff = useCallback(() => {
    const current = handoffs[0]
    if (!current) return
    setHandoffs((queue) =>
      queue.filter((h) => h.handoff_id !== current.handoff_id),
    )
    void confirmBrowserHandoff(host.iii, sessionId, current.handoff_id).catch(
      () => {},
    )
  }, [host, sessionId, handoffs])
  const clearData = useCallback(() => {
    void runAction(async () => {
      await clearBrowserData(host.iii, sessionId)
    })
  }, [host, sessionId, runAction])
  const paneBody = (pane: FeedPane) =>
    pane === 'console' ? (
      <ConsolePanel host={host} sessionId={sessionId} enabled={enabled} />
    ) : pane === 'network' ? (
      <NetworkPanel host={host} sessionId={sessionId} enabled={enabled} />
    ) : pane === 'downloads' ? (
      <DownloadsPanel host={host} sessionId={sessionId} enabled={enabled} />
    ) : (
      <HistoryPanel host={host} sessionId={sessionId} enabled={enabled} />
    )

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
      clearData: () => setConfirmingClear(true),
      toggleDeviceToolbar,
      importCookies,
      copyCookies,
      showDiagnostics: () => setShowDoctor(true),
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
    toggleDeviceToolbar,
    importCookies,
    copyCookies,
  ])
  const annotatingRef = useRef(annotating)
  annotatingRef.current = annotating
  const annotationsRef = useRef(annotations)
  annotationsRef.current = annotations
  const frozenRef = useRef(frozen)
  frozenRef.current = frozen

  const displayName =
    session.title?.trim() || hostOf(session.url) || 'about:blank'
  const feedPane: FeedPane =
    narrow && narrowPane !== 'viewport' ? narrowPane : dockPane
  const browserMajor = chromiumVersion?.match(/\d+/)?.[0]
  const browserLabel = browserMajor ? `Chromium ${browserMajor}` : null
  const readOnly = session.read_only === true

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
            {readOnly ? (
              <span className="br-ui-doc-badge is-readonly">read-only</span>
            ) : null}
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
                  clearData: () => setConfirmingClear(true),
                  toggleDeviceToolbar,
                  importCookies,
                  copyCookies,
                  showDiagnostics: () => setShowDoctor(true),
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
            <ConfirmDialog
              open={confirmingClear}
              onOpenChange={setConfirmingClear}
              title="Clear browsing data?"
              description="Cookies, cache and storage for this session are cleared. Other sessions are untouched."
              confirmLabel="Clear"
              onConfirm={clearData}
            />
            <DoctorDialog
              host={host}
              open={showDoctor}
              onOpenChange={setShowDoctor}
            />
            {showDevice && device ? (
              <DeviceToolbar
                device={device}
                onPreset={applyDevice}
                onDimensions={setDeviceDimensions}
                onRotate={rotateDevice}
                onReset={resetDevice}
              />
            ) : showDevice ? (
              <DeviceToolbar
                device={{
                  width: lastPaneSizeRef.current?.w ?? 0,
                  height: lastPaneSizeRef.current?.h ?? 0,
                  deviceScaleFactor: 1,
                  mobile: false,
                  presetId: null,
                }}
                onPreset={applyDevice}
                onDimensions={setDeviceDimensions}
                onRotate={rotateDevice}
                onReset={resetDevice}
              />
            ) : null}
            {handoff ? (
              <div className="br-ui-handoff" role="alert">
                <div className="br-ui-handoff-text">
                  <span className="br-ui-handoff-title">Waiting for you</span>
                  <span className="br-ui-handoff-instructions">
                    {handoff.instructions}
                    {handoffs.length > 1
                      ? ` (${handoffs.length - 1} more waiting)`
                      : ''}
                  </span>
                </div>
                <Button variant="primary" size="sm" onClick={confirmHandoff}>
                  I'm done
                </Button>
              </div>
            ) : null}
            <input
              ref={cookieInputRef}
              type="file"
              accept=".json,.txt,application/json,text/plain"
              className="br-ui-visually-hidden"
              onChange={(event) => {
                onCookieFile(event.target.files?.[0])
                event.target.value = ''
              }}
            />
            {annotating ? (
              <fieldset
                className="br-ui-annot-tools"
                aria-label="annotation tools"
              >
                <SegmentedControl<AnnotationTool>
                  value={tool}
                  onChange={setTool}
                  options={[
                    {
                      value: 'pin',
                      label: 'Pin',
                      title: 'Drop a numbered pin on a spot',
                    },
                    {
                      value: 'select',
                      label: 'Element',
                      title: 'Click an element to box it and label its selector',
                    },
                    {
                      value: 'rect',
                      label: 'Box',
                      title: 'Drag a rectangle, then add a note',
                    },
                    {
                      value: 'arrow',
                      label: 'Arrow',
                      title: 'Drag an arrow, then add a note',
                    },
                  ]}
                  className="br-ui-tabs"
                  aria-label="annotation tool"
                />
                <span className="br-ui-annot-hint">{TOOL_HINTS[tool]}</span>
                <span className="br-ui-annot-swatches">
                  {SHAPE_COLORS.map((c) => (
                    <button
                      key={c}
                      type="button"
                      aria-pressed={drawColor === c}
                      aria-label={`colour ${c}`}
                      className={cn(
                        'br-ui-annot-swatch',
                        drawColor === c && 'is-on',
                      )}
                      style={{ background: c }}
                      onClick={() => {
                        setDrawColor(c)
                        const id = selectedAnnotation
                        if (id)
                          setAnnotations((list) =>
                            list.map((a) =>
                              a.id === id && (a.kind ?? 'pin') !== 'pin'
                                ? { ...a, color: c }
                                : a,
                            ),
                          )
                      }}
                    />
                  ))}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setAnnotations((list) => undoAnnotation(list))}
                  disabled={annotations.length === 0}
                  title="undo the last mark"
                >
                  Undo
                </Button>
              </fieldset>
            ) : null}
            <Viewport
              frame={annotating && frozen ? frozen : live.frame}
              loading={live.loading}
              error={live.error}
              annotation={viewportAnnotation}
              onSurfaceResize={onSurfaceResize}
              onClickAt={handleClickAt}
              onScrollAt={handleScrollAt}
              onTextInput={handleTextInput}
              onPressKey={handlePressKey}
              requestHint={requestHint}
            />
          </div>
        </div>
      ) : (
        <div className="br-ui-pane-fill">{paneBody(feedPane)}</div>
      )}

      {!narrow ? (
        <div className={`br-ui-dock${dockCollapsed ? ' collapsed' : ''}`}>
          <div className="br-ui-dock-head">
            <SegmentedControl<FeedPane>
              value={dockPane}
              onChange={setDockPane}
              options={FEED_PANES.map((pane) => ({
                value: pane,
                label:
                  pane === 'downloads' && downloadCount > 0
                    ? `Downloads ${downloadCount}`
                    : PANE_LABELS[pane],
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
            <div className="br-ui-dock-body">{paneBody(dockPane)}</div>
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
            <span className="fact hint">{TOOL_HINTS[tool]} Esc ends.</span>
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

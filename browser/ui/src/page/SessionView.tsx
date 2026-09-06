/**
 * One tab's workspace, under the tab strip: the toolbar (back / forward /
 * reload, the address bar, the annotate and menu buttons), the
 * screencast-fed viewport filling the rest of the pane, and — only when
 * asked for from the ⋮ menu — the developer tools docked below it
 * (console | network | downloads | history).
 *
 * Selecting a tab starts its screencast, which wakes a sleeping tab: the
 * viewport shows "opening the page" until the first frame lands. The
 * viewport tracks the pane's size through browser::resize so the frame
 * fills it and clicks map 1:1; the device toolbar pins a size instead.
 *
 * The page remounts this component per tab (React key), so all state here —
 * url draft, annotate mode, type buffer, devtools pane — is tab-local.
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
  takeBrowserScreenshot,
  typeBrowserText,
  zoomBrowserPage,
} from '../lib/browser'
import { toUrl } from '../lib/address'
import { cn } from '../lib/cn'
import { useBrowserSessionEvent } from '../lib/events'
import {
  ExternalLink,
  Globe,
  Incognito,
  MessageSquarePlus,
  RefreshCw,
  X,
} from '../lib/icons'
import { ChevronLeftIcon } from '../lib/widgets'
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
import { saveAnnotationSet } from './annotations-store'
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

type DevtoolsPane = 'console' | 'network' | 'downloads' | 'history'

/** Verbs the page's commands reach through a ref, since they close over
 * this component's tab-local state. */
export interface SessionActions {
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
  clearSiteData: () => void
  toggleDevtools: () => void
  toggleDeviceToolbar: () => void
  importCookies: () => void
  copyCookies: () => void
  showDiagnostics: () => void
  saveSet: () => void
  openSavedSets: () => void
}

const DEVTOOLS_PANES: readonly DevtoolsPane[] = [
  'console',
  'network',
  'downloads',
  'history',
]
const PANE_LABELS: Record<DevtoolsPane, string> = {
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

interface SessionViewProps {
  /** Opens the saved-sets dialog (page-level; works without a tab). */
  onOpenSavedSets?: (key?: string | null) => void
  host: Host
  session: BrowserSessionInfo
  enabled: boolean
  /** Stable workspace-tab id — namespaces persisted UI state. */
  tabId: string
  /** Populated with this tab's verbs while mounted, so the page's commands
   * can reach them. */
  actionsRef?: RefObject<SessionActions | null>
  onSessionsRefresh: () => void
  onNewTab: (incognito: boolean) => void
}

export function SessionView({
  host,
  onOpenSavedSets,
  session,
  enabled,
  tabId,
  actionsRef,
  onSessionsRefresh,
  onNewTab,
}: SessionViewProps) {
  const sessionId = session.session_id
  const asleep = session.active === false

  // Developer tools: hidden until asked for, then remembered per workspace
  // tab (open or not, and which pane) so a reload lands where it was left.
  const devtoolsStoreKey = `browser-ui:${tabId || 'page'}:devtools`
  const devtoolsPaneStoreKey = `browser-ui:${tabId || 'page'}:dock`
  const [devtoolsOpen, setDevtoolsOpenState] = useState(
    () => readStored(devtoolsStoreKey) === 'true',
  )
  const [devtoolsPane, setDevtoolsPaneState] = useState<DevtoolsPane>(() => {
    const stored = readStored(devtoolsPaneStoreKey)
    return DEVTOOLS_PANES.includes(stored as DevtoolsPane)
      ? (stored as DevtoolsPane)
      : 'console'
  })
  const setDevtoolsOpen = useCallback(
    (open: boolean) => {
      setDevtoolsOpenState(open)
      writeStored(devtoolsStoreKey, String(open))
    },
    [devtoolsStoreKey],
  )
  const toggleDevtools = useCallback(
    () => setDevtoolsOpen(!devtoolsOpen),
    [devtoolsOpen, setDevtoolsOpen],
  )
  const setDevtoolsPane = (pane: DevtoolsPane) => {
    setDevtoolsPaneState(pane)
    writeStored(devtoolsPaneStoreKey, pane)
  }

  const [reseedToken, setReseedToken] = useState(0)
  // Bumped when the tab wakes under us, so the screencast is started again
  // on the new page (see useLiveFrames).
  const [wakeToken, setWakeToken] = useState(0)
  const live = useLiveFrames(host, sessionId, enabled, reseedToken, wakeToken)

  const [actionError, setActionError] = useState<string | null>(null)
  const runAction = useCallback(async (action: () => Promise<void>) => {
    try {
      await action()
      setActionError(null)
    } catch (err) {
      setActionError(errorMessage(err))
    }
  }, [])

  // URL bar: mirrors the tab's committed url, but never clobbers
  // in-progress typing (focus-aware sync). about:blank reads as empty, the
  // way a new tab's bar does.
  const shownUrl = session.url === 'about:blank' ? '' : session.url
  const [urlDraft, setUrlDraft] = useState(shownUrl)
  const urlFocusedRef = useRef(false)
  const lastSessionUrlRef = useRef(shownUrl)
  useEffect(() => {
    if (lastSessionUrlRef.current === shownUrl) return
    lastSessionUrlRef.current = shownUrl
    if (!urlFocusedRef.current) setUrlDraft(shownUrl)
  }, [shownUrl])

  const submitUrl = useCallback(() => {
    const url = toUrl(urlDraft)
    if (!url) return
    void runAction(async () => {
      const result = await navigateBrowser(host.iii, sessionId, url)
      onSessionsRefresh()
      // Like a browser: the tab shows Chromium's error page; say why here.
      if (result?.error) throw new Error(`page failed to load: ${result.error}`)
    })
    urlInputRef.current?.blur()
  }, [host, urlDraft, sessionId, runAction, onSessionsRefresh])

  const handleHistory = useCallback(
    (action: 'back' | 'forward' | 'reload') => {
      void runAction(async () => {
        const result = await controlBrowserHistory(host.iii, sessionId, action)
        if (result?.url && result.url !== 'about:blank') setUrlDraft(result.url)
        onSessionsRefresh()
      })
    },
    [host, sessionId, runAction, onSessionsRefresh],
  )

  const openCurrentPage = useCallback(() => {
    const url = toUrl(urlDraft) || shownUrl
    if (url) window.open(url, '_blank', 'noopener,noreferrer')
  }, [shownUrl, urlDraft])

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
  const liveFrameRef = useRef(live.frame)
  liveFrameRef.current = live.frame
  const annotatingRef = useRef(annotating)
  annotatingRef.current = annotating
  const annotationsRef = useRef(annotations)
  annotationsRef.current = annotations
  const frozenRef = useRef(frozen)
  frozenRef.current = frozen
  const toggleAnnotate = useCallback(() => {
    setAnnotating((current) => {
      if (current) return false
      // Re-entering with unsent marks resumes them on their own frozen
      // frame; only a fresh set starts from the live frame.
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
  const [saving, setSaving] = useState(false)
  const saveSet = useCallback(() => {
    const set = annotationSet()
    if (!set || set.annotations.length === 0 || saving) return
    setSaving(true)
    void (async () => {
      try {
        await saveAnnotationSet(host.iii, set)
        setActionError(
          `saved ${set.annotations.length} mark${set.annotations.length === 1 ? '' : 's'}`,
        )
      } catch (err) {
        setActionError(errorMessage(err))
      } finally {
        setSaving(false)
      }
    })()
  }, [annotationSet, host, saving])

  const downloadAnnotations = useCallback(() => {
    const set = annotationSet()
    if (!set || set.annotations.length === 0) return
    void runAction(async () => {
      const blob = await renderAnnotatedImage(set)
      downloadFile(
        new File([blob], annotationFileName(set, 'png'), { type: 'image/png' }),
      )
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
  // pin so the note carries the element it points at. Picked events carry
  // no correlation token, so pins waiting for their element label queue up
  // first-in first-out; two quick drops each get their own answer.
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

  // Flush any buffered text before the component unmounts, so keystrokes
  // typed against one tab are sent to that tab rather than dropped.
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
  // observer fires often; debounce and skip sub-pixel-ish changes. The
  // screencast is CSS-pixel sized whatever the device scale factor, so the
  // pane fit asks for 1x and spares the page a 2x render it cannot show.
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
      // A read-only tab's viewport is not ours to change; the frame
      // letterbox-scales instead.
      if (session.read_only === true) return
      if (deviceRef.current) return
      const last = lastSentSizeRef.current
      if (last && Math.abs(last.w - width) < 4 && Math.abs(last.h - height) < 4)
        return
      window.clearTimeout(resizeTimerRef.current)
      resizeTimerRef.current = window.setTimeout(() => {
        lastSentSizeRef.current = { w: width, h: height }
        applyViewport(width, height, 1, undefined, true)
      }, 180)
    },
    [applyViewport, session.read_only],
  )
  useEffect(() => () => window.clearTimeout(resizeTimerRef.current), [])
  // A tab that just woke has a fresh page: start its screencast again and
  // fit the config-sized viewport to the pane.
  const wasAsleepRef = useRef(asleep)
  useEffect(() => {
    if (wasAsleepRef.current && !asleep) {
      setWakeToken((t) => t + 1)
      lastSentSizeRef.current = null
      const pane = lastPaneSizeRef.current
      if (pane && !deviceRef.current) onSurfaceResize(pane.w, pane.h)
    }
    wasAsleepRef.current = asleep
  }, [asleep, onSurfaceResize])
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
      applyViewport(pane.w, pane.h, 1)
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
  useEffect(() => () => window.clearTimeout(findTimerRef.current), [])

  // Zoom belongs to the loaded document, so a navigation resets it in the
  // page; the level the user chose is re-applied when the next page commits.
  // Read only once the page is up: asking a sleeping tab would wake it.
  const [zoom, setZoom] = useState(100)
  const zoomRef = useRef(zoom)
  zoomRef.current = zoom
  useEffect(() => {
    if (asleep) return
    let cancelled = false
    void zoomBrowserPage(host.iii, sessionId, 'read')
      .then((level) => {
        if (!cancelled) setZoom(level)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [host, sessionId, asleep])
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
  const clearSiteData = useCallback(() => {
    void runAction(async () => {
      const cleared = await clearBrowserData(host.iii, sessionId)
      setActionError(
        cleared.length > 0
          ? `cleared ${cleared.join(', ')} for this site`
          : 'nothing to clear for this site',
      )
      handleHistory('reload')
    })
  }, [host, sessionId, runAction, handleHistory])

  const urlInputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (!actionsRef) return
    actionsRef.current = {
      focusUrl: () => {
        urlInputRef.current?.focus()
        urlInputRef.current?.select()
      },
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
      clearSiteData: () => setConfirmingClear(true),
      toggleDevtools,
      toggleDeviceToolbar,
      importCookies,
      copyCookies,
      showDiagnostics: () => setShowDoctor(true),
      saveSet,
      openSavedSets: () => onOpenSavedSets?.(),
    }
    return () => {
      actionsRef.current = null
    }
  }, [
    actionsRef,
    toggleAnnotate,
    sendAnnotations,
    downloadAnnotations,
    clearAnnotations,
    openFind,
    applyZoom,
    takeScreenshot,
    screenshotToChat,
    printToPdf,
    toggleDevtools,
    toggleDeviceToolbar,
    importCookies,
    copyCookies,
    saveSet,
    onOpenSavedSets,
  ])

  const paneBody = (pane: DevtoolsPane) =>
    pane === 'console' ? (
      <ConsolePanel host={host} sessionId={sessionId} enabled={enabled} />
    ) : pane === 'network' ? (
      <NetworkPanel host={host} sessionId={sessionId} enabled={enabled} />
    ) : pane === 'downloads' ? (
      <DownloadsPanel host={host} sessionId={sessionId} enabled={enabled} />
    ) : (
      <HistoryPanel host={host} sessionId={sessionId} enabled={enabled} />
    )

  const viewportLabel = live.error
    ? `live view failed: ${live.error}`
    : asleep
      ? 'opening the page…'
      : 'waiting for the first frame…'
  const marks = annotations.length

  return (
    <section
      className={cn(
        'br-ui-stage',
        session.incognito && 'is-incognito',
        session.read_only && 'is-readonly',
      )}
      aria-label={`browser tab ${sessionId}`}
    >
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
            {session.incognito ? (
              <Incognito size={16} aria-hidden className="br-ui-address-icon" />
            ) : (
              <Globe size={16} aria-hidden className="br-ui-address-icon" />
            )}
            <Input
              ref={urlInputRef}
              name="browser-url"
              value={urlDraft}
              onChange={setUrlDraft}
              preserveCase
              placeholder={
                session.incognito
                  ? 'Search or type a URL — incognito'
                  : 'Search or type a URL'
              }
              aria-label="page url"
              onFocus={(event) => {
                urlFocusedRef.current = true
                event.currentTarget.select()
              }}
              onBlur={() => {
                urlFocusedRef.current = false
                // Typing abandoned: fall back to where the tab really is.
                setUrlDraft(shownUrl)
              }}
              onKeyDown={(event) => {
                if (event.key === 'Escape') {
                  setUrlDraft(shownUrl)
                  event.currentTarget.blur()
                }
              }}
              className="br-ui-url-input"
            />
            {session.read_only ? (
              <span className="br-ui-address-tag">read-only</span>
            ) : null}
          </div>
          {session.incognito ? (
            <span className="br-ui-incognito-pill" title="Incognito tab: nothing is saved">
              <Incognito size={14} aria-hidden />
              Incognito
            </span>
          ) : null}
          <button
            type="button"
            onClick={toggleAnnotate}
            aria-pressed={annotating}
            aria-label={annotating ? 'stop annotating' : 'annotate the view'}
            title={
              annotating
                ? 'annotating: click an element to drop a pin on it, esc ends'
                : 'annotate: freeze the view and pin elements with notes'
            }
            className={cn(
              'br-ui-chrome-btn',
              annotating && 'is-on',
              !annotating && marks > 0 && 'has-marks',
            )}
          >
            <MessageSquarePlus size={17} aria-hidden />
            {!annotating && marks > 0 ? (
              <span className="br-ui-chrome-badge">{marks}</span>
            ) : null}
          </button>
          <button
            type="button"
            className="br-ui-chrome-btn"
            onClick={openCurrentPage}
            title="open page in your own browser"
            aria-label="open page in your own browser"
          >
            <ExternalLink size={16} aria-hidden />
          </button>
          <PageMenu
            zoom={zoom}
            devtoolsOpen={devtoolsOpen}
            canSendToChat={typeof host.chat?.compose === 'function'}
            actions={{
              newTab: () => onNewTab(false),
              newIncognitoTab: () => onNewTab(true),
              findInPage: openFind,
              takeScreenshot,
              screenshotToChat,
              printToPdf,
              zoomIn: () => applyZoom('in'),
              zoomOut: () => applyZoom('out'),
              zoomReset: () => applyZoom('reset'),
              toggleDevtools,
              clearSiteData: () => setConfirmingClear(true),
              toggleDeviceToolbar,
              importCookies,
              copyCookies,
              showDiagnostics: () => setShowDoctor(true),
              openSavedSets: () => onOpenSavedSets?.(),
            }}
          />
          <button type="submit" className="br-ui-address-submit" tabIndex={-1}>
            navigate to address
          </button>
        </form>

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
          title="Clear cookies and site data?"
          description="Cookies and storage for the site this tab is on, plus the cache, are cleared and the page reloads. Other sites stay signed in. Settings has the button that clears everything."
          confirmLabel="Clear"
          onConfirm={clearSiteData}
        />
        <DoctorDialog
          host={host}
          open={showDoctor}
          onOpenChange={setShowDoctor}
        />
        {showDevice ? (
          <DeviceToolbar
            device={
              device ?? {
                width: lastPaneSizeRef.current?.w ?? 0,
                height: lastPaneSizeRef.current?.h ?? 0,
                deviceScaleFactor: 1,
                mobile: false,
                presetId: null,
              }
            }
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
        {annotating || marks > 0 ? (
          <fieldset className="br-ui-annot-tools" aria-label="annotation tools">
            {annotating ? (
              <>
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
                <span className="br-ui-annot-hint">{TOOL_HINTS[tool]} Esc ends.</span>
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
                  disabled={marks === 0}
                  title="undo the last mark"
                >
                  Undo
                </Button>
              </>
            ) : (
              <span className="br-ui-annot-hint">
                {marks} {marks === 1 ? 'mark' : 'marks'} pinned on the frozen
                view
              </span>
            )}
            {marks > 0 ? (
              <>
                <Button
                  variant="primary"
                  size="sm"
                  onClick={sendAnnotations}
                  disabled={sending || typeof host.chat?.compose !== 'function'}
                  title="send the pins to the chat, one attachment each (⌘↵)"
                >
                  {sending ? 'Sending…' : `Send ${marks} to chat`}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={saveSet}
                  disabled={saving}
                  title="save this set for later; anyone on this engine can reopen it"
                >
                  {saving ? 'Saving…' : 'Save'}
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
                  title="drop every mark"
                >
                  Clear
                </Button>
              </>
            ) : null}
            <Button
              variant="ghost"
              size="sm"
              onClick={toggleAnnotate}
              title={annotating ? 'back to the live view' : 'resume annotating'}
            >
              {annotating ? 'Done' : 'Resume'}
            </Button>
          </fieldset>
        ) : null}
        <Viewport
          frame={annotating && frozen ? frozen : live.frame}
          loading={live.loading}
          emptyLabel={viewportLabel}
          annotation={viewportAnnotation}
          onSurfaceResize={onSurfaceResize}
          onClickAt={handleClickAt}
          onScrollAt={handleScrollAt}
          onTextInput={handleTextInput}
          onPressKey={handlePressKey}
          requestHint={requestHint}
        />

        {devtoolsOpen ? (
          <div className="br-ui-dock">
            <div className="br-ui-dock-head">
              <SegmentedControl<DevtoolsPane>
                value={devtoolsPane}
                onChange={setDevtoolsPane}
                options={DEVTOOLS_PANES.map((pane) => ({
                  value: pane,
                  label:
                    pane === 'downloads' && downloadCount > 0
                      ? `Downloads ${downloadCount}`
                      : PANE_LABELS[pane],
                }))}
                className="br-ui-tabs"
                aria-label="Developer tools"
              />
              <button
                type="button"
                className="br-ui-dock-toggle"
                aria-label="hide developer tools"
                title="hide developer tools"
                onClick={() => setDevtoolsOpen(false)}
              >
                <X size={16} aria-hidden />
              </button>
            </div>
            <div className="br-ui-dock-body">{paneBody(devtoolsPane)}</div>
          </div>
        ) : null}
      </div>
    </section>
  )
}

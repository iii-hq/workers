import * as ConsoleUi from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  type BrowserClickOptions,
  type BrowserPickHint,
  elementLabel,
} from '../lib/browser'
import { cn } from '../lib/cn'
import type { Annotation } from './annotations'
import type { LiveFrame } from './useLiveFrames'

/** Annotate mode: the frame is frozen, clicks drop pins instead of reaching
    the page, and the pins render over the picture. */
export interface ViewportAnnotation {
  annotations: readonly Annotation[]
  selectedId: string | null
  onAdd: (x: number, y: number) => void
  onSelect: (id: string | null) => void
  onMove: (id: string, x: number, y: number) => void
  onRemove: (id: string) => void
  onNote: (id: string, note: string) => void
  tool?: 'pin' | 'rect' | 'arrow' | 'select'
  drawColor?: string
  onAddShape?: (kind: 'rect' | 'arrow', x: number, y: number) => void
  onResizeShape?: (x2: number, y2: number) => void
  onEndShape?: () => void
}

/**
 * The session viewport: the latest screencast frame scaled to fit the pane
 * (aspect preserved, centered, letterboxed), acting as a real browser
 * surface. Mouse and keyboard map from the rendered image rect to
 * page-viewport space, so the mapping survives any pane size: clicks, double
 * clicks, right clicks, wheel scroll, and (while the surface is focused)
 * typing and special keys all forward as `browser::act` — keyboard
 * forwarding wins over page shortcuts, and Shift+Escape is the one reserved
 * way out of the surface. Events in the letterbox margin outside the image
 * do nothing. In pick mode the page is in
 * inspect mode: the forwarded click resolves the pick, and a throttled
 * `browser::pick::hint` drives the client-drawn hover highlight over the
 * image.
 */

const HINT_INTERVAL_MS = 120
/** Wheel deltas accumulate this long before one scroll act goes out: short
 * enough that a flick reads as continuous, long enough that a trackpad's
 * burst of tiny deltas is one round trip, not fifty. */
const SCROLL_FLUSH_MS = 40
const DOUBLE_CLICK_WINDOW_MS = 220
/** Keys forwarded as `browser::act {action:'press'}` (worker key_spec set). */
const PRESS_KEYS: ReadonlySet<string> = new Set([
  'Enter',
  'Tab',
  'Escape',
  'Backspace',
  'Delete',
  'ArrowUp',
  'ArrowDown',
  'ArrowLeft',
  'ArrowRight',
  'Home',
  'End',
  'PageUp',
  'PageDown',
])

interface HintDisplay {
  left: number
  top: number
  width: number
  height: number
  label: string
  dims: string
}

interface RenderedImageRect {
  left: number
  top: number
  width: number
  height: number
}

/**
 * `object-fit: contain` paints the screenshot inside the image element's
 * content box and can leave horizontal or vertical letterboxing. DOM APIs
 * only expose the element box, so derive the centered painted rect from the
 * picture the browser actually painted — its natural pixel size — before
 * translating pointer coordinates. The frame's reported viewport size is
 * the coordinate space the point is then scaled into; deriving the rect from
 * that instead put clicks off whenever a frame's pixels did not match its
 * metadata (the transitional frame after a resize).
 */
function renderedImageRect(img: HTMLImageElement): RenderedImageRect | null {
  const naturalWidth = img.naturalWidth
  const naturalHeight = img.naturalHeight
  if (naturalWidth <= 0 || naturalHeight <= 0) return null
  const box = img.getBoundingClientRect()
  if (box.width <= 0 || box.height <= 0) return null

  const scale = Math.min(box.width / naturalWidth, box.height / naturalHeight)
  const width = naturalWidth * scale
  const height = naturalHeight * scale
  return {
    left: box.left + (box.width - width) / 2,
    top: box.top + (box.height - height) / 2,
    width,
    height,
  }
}

interface ViewportProps {
  frame: LiveFrame | null
  loading: boolean
  /** What the empty surface says while there is no frame. */
  emptyLabel: string
  onClickAt: (x: number, y: number, options?: BrowserClickOptions) => void
  onScrollAt: (x: number, y: number, deltaY: number) => void
  onTextInput: (text: string) => void
  onPressKey: (key: string) => void
  requestHint: (x: number, y: number) => Promise<BrowserPickHint | null>
  /** Present while annotating; the frame shown is the frozen one. */
  annotation?: ViewportAnnotation | null
  /** The surface's CSS pixel size, reported as it changes, so the caller can
   * match the live viewport to the pane (no letterboxing). Not called while
   * annotating (the frozen frame must not resize under the pins). */
  onSurfaceResize?: (width: number, height: number) => void
}

export function Viewport({
  frame,
  loading,
  emptyLabel,
  onClickAt,
  onScrollAt,
  onTextInput,
  onPressKey,
  requestHint,
  annotation = null,
  onSurfaceResize,
}: ViewportProps) {
  const surfaceRef = useRef<HTMLDivElement>(null)
  const annotating = annotation !== null
  const imgRef = useRef<HTMLImageElement>(null)

  const frameRef = useRef(frame)
  frameRef.current = frame
  const annotatingRef = useRef(annotating)
  annotatingRef.current = annotating
  const onScrollAtRef = useRef(onScrollAt)
  onScrollAtRef.current = onScrollAt

  // Report the surface's CSS pixel size so the caller can size the live
  // viewport to match the pane. Paused while annotating (the frozen frame
  // and its pins must not move); the caller debounces the actual resize.
  const onSurfaceResizeRef = useRef(onSurfaceResize)
  onSurfaceResizeRef.current = onSurfaceResize
  useEffect(() => {
    const surface = surfaceRef.current
    if (!surface || typeof ResizeObserver === 'undefined') return
    const report = () => {
      if (annotatingRef.current) return
      const w = surface.clientWidth
      const h = surface.clientHeight
      if (w > 0 && h > 0) onSurfaceResizeRef.current?.(w, h)
    }
    const observer = new ResizeObserver(report)
    observer.observe(surface)
    report()
    return () => observer.disconnect()
  }, [])

  /** Client point -> page-viewport point, null outside the rendered image. */
  const mapToPage = useCallback(
    (clientX: number, clientY: number): { x: number; y: number } | null => {
      const current = frameRef.current
      const img = imgRef.current
      if (!current || !img || current.width <= 0 || current.height <= 0) {
        return null
      }
      const rect = renderedImageRect(img)
      if (!rect) return null
      const relX = (clientX - rect.left) / rect.width
      const relY = (clientY - rect.top) / rect.height
      if (relX < 0 || relX > 1 || relY < 0 || relY > 1) return null
      return {
        x: Math.min(current.width - 1, Math.round(relX * current.width)),
        y: Math.min(current.height - 1, Math.round(relY * current.height)),
      }
    },
    [],
  )

  // Single vs double click: a first click waits out the double-click window
  // so a dblclick can replace it with one click_count:2 act.
  const pendingClickRef = useRef<number | undefined>(undefined)
  const clearPendingClick = useCallback(() => {
    window.clearTimeout(pendingClickRef.current)
    pendingClickRef.current = undefined
  }, [])
  useEffect(() => clearPendingClick, [clearPendingClick])
  useEffect(() => {
    if (annotating) clearPendingClick()
  }, [annotating, clearPendingClick])

  const handleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (annotating) return
    const pt = mapToPage(e.clientX, e.clientY)
    if (!pt) return
    if (e.detail >= 2) {
      clearPendingClick()
      return
    }
    clearPendingClick()
    pendingClickRef.current = window.setTimeout(() => {
      pendingClickRef.current = undefined
      onClickAt(pt.x, pt.y)
    }, DOUBLE_CLICK_WINDOW_MS)
  }

  const handleDoubleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (annotating) return
    clearPendingClick()
    const pt = mapToPage(e.clientX, e.clientY)
    if (!pt) return
    onClickAt(pt.x, pt.y, { clickCount: 2 })
  }

  const handleContextMenu = (e: React.MouseEvent<HTMLDivElement>) => {
    const pt = mapToPage(e.clientX, e.clientY)
    if (!pt) return
    e.preventDefault()
    if (annotating) return
    onClickAt(pt.x, pt.y, { button: 'right' })
  }

  // Wheel forwards as one accumulated scroll act per flush window. Native
  // non-passive listener: React attaches wheel passively, so preventDefault
  // would be ignored through the synthetic event.
  const wheelAccumRef = useRef<{
    delta: number
    pt: { x: number; y: number } | null
    timer: number | undefined
  }>({ delta: 0, pt: null, timer: undefined })
  useEffect(() => {
    const surface = surfaceRef.current
    if (!surface) return
    const accum = wheelAccumRef.current
    const onWheel = (e: WheelEvent) => {
      const pt = mapToPage(e.clientX, e.clientY)
      if (!pt) return
      e.preventDefault()
      if (annotatingRef.current) return
      const scale = e.deltaMode === 1 ? 16 : 1
      accum.delta += e.deltaY * scale
      accum.pt = pt
      if (accum.timer !== undefined) return
      accum.timer = window.setTimeout(() => {
        accum.timer = undefined
        const { delta, pt: point } = accum
        accum.delta = 0
        accum.pt = null
        if (point && delta !== 0) {
          onScrollAtRef.current(point.x, point.y, Math.round(delta))
        }
      }, SCROLL_FLUSH_MS)
    }
    surface.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      surface.removeEventListener('wheel', onWheel)
      window.clearTimeout(accum.timer)
      accum.timer = undefined
      accum.delta = 0
      accum.pt = null
    }
  }, [mapToPage])

  const handleKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    // Annotate mode owns the keyboard: Escape bubbles to the page-level
    // listener that exits the mode; nothing forwards to the page.
    if (annotating) return
    // The page wants Tab and Escape, which is exactly what a keyboard user
    // needs to leave with. Shift+Escape is reserved as the way out and
    // never reaches the page.
    if (e.key === 'Escape' && e.shiftKey) {
      e.preventDefault()
      e.stopPropagation()
      surfaceRef.current?.blur()
      return
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return
    if (PRESS_KEYS.has(e.key)) {
      e.preventDefault()
      e.stopPropagation()
      onPressKey(e.key)
      return
    }
    if (e.key.length === 1) {
      e.preventDefault()
      e.stopPropagation()
      onTextInput(e.key)
    }
  }

  // Hover hint: while annotating, sample the latest cursor position on an
  // interval and ask the worker what a pin dropped there would point at; the
  // highlight box is drawn client-side over the frozen frame (the page under
  // it is still live, so the hit-test matches what the frame shows).
  const cursorRef = useRef<{ x: number; y: number } | null>(null)
  const [hint, setHint] = useState<HintDisplay | null>(null)

  const handleMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!annotating) return
    const pt = mapToPage(e.clientX, e.clientY)
    cursorRef.current = pt
    if (!pt) setHint(null)
  }

  const handleMouseLeave = () => {
    cursorRef.current = null
    setHint(null)
  }

  useEffect(() => {
    if (!annotating) {
      cursorRef.current = null
      setHint(null)
      return
    }
    let cancelled = false
    let busy = false
    const id = window.setInterval(() => {
      if (busy || cancelled) return
      const pt = cursorRef.current
      if (!pt) return
      busy = true
      void (async () => {
        try {
          const res = await requestHint(pt.x, pt.y)
          if (cancelled) return
          if (!cursorRef.current || !res?.hit || !res.bounds) {
            setHint(null)
            return
          }
          const current = frameRef.current
          const img = imgRef.current
          const surface = surfaceRef.current
          if (!current || !img || !surface || current.width <= 0) {
            setHint(null)
            return
          }
          const imgRect = renderedImageRect(img)
          const surfaceRect = surface.getBoundingClientRect()
          if (!imgRect) {
            setHint(null)
            return
          }
          const scaleX = imgRect.width / current.width
          const scaleY = imgRect.height / current.height
          setHint({
            left: imgRect.left - surfaceRect.left + res.bounds.x * scaleX,
            top: imgRect.top - surfaceRect.top + res.bounds.y * scaleY,
            width: res.bounds.width * scaleX,
            height: res.bounds.height * scaleY,
            label: elementLabel(res.tag, res.id, res.classes),
            dims: `${Math.round(res.bounds.width)}x${Math.round(res.bounds.height)}`,
          })
        } catch {
          if (!cancelled) setHint(null)
        } finally {
          busy = false
        }
      })()
    }, HINT_INTERVAL_MS)
    return () => {
      cancelled = true
      window.clearInterval(id)
      setHint(null)
    }
  }, [annotating, requestHint])

  return (
    <div
      ref={surfaceRef}
      role="application"
      // biome-ignore lint/a11y/noNoninteractiveTabindex: a live remote-browser surface forwarding raw mouse/keyboard input; focus is how typing reaches the page
      tabIndex={0}
      aria-label={
        annotating
          ? 'browser viewport, annotate mode: click an element to drop a numbered pin on the frozen view'
          : 'browser viewport: clicks, scrolling, and typing forward to the page'
      }
      onPointerDown={(e) => {
        if (
          (e.target as HTMLElement).closest(
            '[data-annotation-pin], [data-annotation-callout]',
          )
        )
          return
        surfaceRef.current?.focus()
      }}
      onClick={handleClick}
      onDoubleClick={handleDoubleClick}
      onContextMenu={handleContextMenu}
      onMouseMove={handleMouseMove}
      onMouseLeave={handleMouseLeave}
      onKeyDown={handleKeyDown}
      className={cn('br-ui-vp', annotating && 'is-annotating')}
    >
      {frame && annotation && ConsoleUi.AnnotationLayer ? (
        <ConsoleUi.AnnotationLayer
          annotations={annotation.annotations}
          image={{ width: frame.width, height: frame.height }}
          active
          selectedId={annotation.selectedId}
          onAdd={annotation.onAdd}
          onSelect={annotation.onSelect}
          onMove={annotation.onMove}
          onRemove={annotation.onRemove}
          onNote={annotation.onNote}
          tool={annotation.tool}
          onAddShape={annotation.onAddShape}
          onResizeShape={annotation.onResizeShape}
          onEndShape={annotation.onEndShape}
          className="br-ui-vp-annot"
        >
          <img
            ref={imgRef}
            src={frame.dataUrl}
            alt="frozen view of the current page"
            draggable={false}
            className="br-ui-vp-img"
          />
        </ConsoleUi.AnnotationLayer>
      ) : frame ? (
        <img
          ref={imgRef}
          src={frame.dataUrl}
          alt="live view of the current page"
          draggable={false}
          className="br-ui-vp-img"
        />
      ) : (
        <p className={cn('br-ui-vp-empty', loading && 'is-loading')}>{emptyLabel}</p>
      )}
      {hint ? (
        <div
          aria-hidden
          className="br-ui-vp-hint"
          style={{
            left: hint.left,
            top: hint.top,
            width: hint.width,
            height: hint.height,
          }}
        >
          <span
            className={cn(
              'br-ui-vp-hint-label',
              hint.top >= 22 ? 'above' : 'below',
            )}
          >
            <span className="br-ui-vp-hint-tag">{hint.label}</span>
            <span className="br-ui-vp-hint-dims">{hint.dims}</span>
          </span>
        </div>
      ) : null}
    </div>
  )
}

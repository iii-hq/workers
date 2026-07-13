import { useCallback, useEffect, useRef, useState } from 'react'
import {
  type BrowserClickOptions,
  type BrowserPickHint,
  elementLabel,
} from '@/lib/browser'
import { cn } from '@/lib/utils'
import type { LiveFrame } from '../hooks/useLiveFrames'

/**
 * The session viewport: the latest screencast frame scaled to fit the pane
 * (aspect preserved, centered, letterboxed), acting as a real browser
 * surface. Mouse and keyboard map from the rendered image rect to
 * page-viewport space (the frame's width/height against
 * `getBoundingClientRect` at event time), so the mapping survives any pane
 * size: clicks, double clicks, right clicks, wheel scroll, and (while the
 * surface is focused) typing and special keys all forward as
 * `browser::act`. Events in the letterbox margin outside the image do
 * nothing. In pick mode the page is in inspect mode: the forwarded click
 * resolves the pick, and a throttled `browser::pick::hint` drives the
 * client-drawn hover highlight over the image.
 */

const HINT_INTERVAL_MS = 120
const SCROLL_FLUSH_MS = 150
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

interface ViewportProps {
  frame: LiveFrame | null
  loading: boolean
  error: string | null
  picking: boolean
  onClickAt: (x: number, y: number, options?: BrowserClickOptions) => void
  onScrollAt: (x: number, y: number, deltaY: number) => void
  onTextInput: (text: string) => void
  onPressKey: (key: string) => void
  requestHint: (x: number, y: number) => Promise<BrowserPickHint | null>
}

export function Viewport({
  frame,
  loading,
  error,
  picking,
  onClickAt,
  onScrollAt,
  onTextInput,
  onPressKey,
  requestHint,
}: ViewportProps) {
  const surfaceRef = useRef<HTMLDivElement>(null)
  const imgRef = useRef<HTMLImageElement>(null)

  const frameRef = useRef(frame)
  frameRef.current = frame
  const pickingRef = useRef(picking)
  pickingRef.current = picking
  const onScrollAtRef = useRef(onScrollAt)
  onScrollAtRef.current = onScrollAt

  /** Client point -> page-viewport point, null outside the rendered image. */
  const mapToPage = useCallback(
    (clientX: number, clientY: number): { x: number; y: number } | null => {
      const current = frameRef.current
      const img = imgRef.current
      if (!current || !img || current.width <= 0 || current.height <= 0) {
        return null
      }
      const rect = img.getBoundingClientRect()
      if (rect.width <= 0 || rect.height <= 0) return null
      const relX = (clientX - rect.left) / rect.width
      const relY = (clientY - rect.top) / rect.height
      if (relX < 0 || relX > 1 || relY < 0 || relY > 1) return null
      return {
        x: Math.round(relX * current.width),
        y: Math.round(relY * current.height),
      }
    },
    [],
  )

  // Single vs double click: a first click waits out the double-click window
  // so a dblclick can replace it with one click_count:2 act. Pick mode skips
  // the wait (the click resolves the pick).
  const pendingClickRef = useRef<number | undefined>(undefined)
  const clearPendingClick = useCallback(() => {
    window.clearTimeout(pendingClickRef.current)
    pendingClickRef.current = undefined
  }, [])
  useEffect(() => clearPendingClick, [clearPendingClick])
  useEffect(() => {
    if (picking) clearPendingClick()
  }, [picking, clearPendingClick])

  const handleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    const pt = mapToPage(e.clientX, e.clientY)
    if (!pt) return
    if (picking) {
      onClickAt(pt.x, pt.y)
      return
    }
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
    if (picking) return
    clearPendingClick()
    const pt = mapToPage(e.clientX, e.clientY)
    if (!pt) return
    onClickAt(pt.x, pt.y, { clickCount: 2 })
  }

  const handleContextMenu = (e: React.MouseEvent<HTMLDivElement>) => {
    const pt = mapToPage(e.clientX, e.clientY)
    if (!pt) return
    e.preventDefault()
    if (picking) return
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
      if (pickingRef.current) return
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
    // Pick mode owns the keyboard: Escape bubbles to the page-level
    // listener that exits pick mode; nothing forwards to the page.
    if (picking) return
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

  // Pick hover hint: while picking, sample the latest cursor position on an
  // interval and ask the worker what a click would hit; the highlight box
  // is drawn client-side over the image, never waiting for a screenshot
  // frame.
  const cursorRef = useRef<{ x: number; y: number } | null>(null)
  const [hint, setHint] = useState<HintDisplay | null>(null)

  const handleMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!picking) return
    const pt = mapToPage(e.clientX, e.clientY)
    cursorRef.current = pt
    if (!pt) setHint(null)
  }

  const handleMouseLeave = () => {
    cursorRef.current = null
    setHint(null)
  }

  useEffect(() => {
    if (!picking) {
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
          const imgRect = img.getBoundingClientRect()
          const surfaceRect = surface.getBoundingClientRect()
          if (imgRect.width <= 0 || imgRect.height <= 0) {
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
  }, [picking, requestHint])

  return (
    <div
      ref={surfaceRef}
      role="application"
      // biome-ignore lint/a11y/noNoninteractiveTabindex: a live remote-browser surface forwarding raw mouse/keyboard input; focus is how typing reaches the page
      tabIndex={0}
      aria-label={
        picking
          ? 'browser viewport, pick mode: click an element to send it to chat'
          : 'browser viewport: clicks, scrolling, and typing forward to the page'
      }
      onClick={handleClick}
      onDoubleClick={handleDoubleClick}
      onContextMenu={handleContextMenu}
      onMouseMove={handleMouseMove}
      onMouseLeave={handleMouseLeave}
      onKeyDown={handleKeyDown}
      className={cn(
        'relative flex-1 min-h-0 min-w-0 overflow-hidden bg-panel p-3',
        'flex items-center justify-center',
        picking ? 'cursor-crosshair' : 'cursor-default',
        'outline-none focus-visible:ring-1 focus-visible:ring-ring',
      )}
    >
      {frame ? (
        <img
          ref={imgRef}
          src={frame.dataUrl}
          alt="live view of the current page"
          draggable={false}
          className={cn(
            'block max-w-full max-h-full w-auto h-auto object-contain select-none border bg-bg',
            picking ? 'border-accent' : 'border-rule',
          )}
        />
      ) : (
        <p className="font-mono text-[12px] lowercase text-ink-ghost">
          {error
            ? `live view failed: ${error}`
            : loading
              ? 'waiting for the first frame...'
              : 'no frame yet'}
        </p>
      )}
      {hint ? (
        <div
          aria-hidden
          className="absolute pointer-events-none border border-accent bg-accent/15"
          style={{
            left: hint.left,
            top: hint.top,
            width: hint.width,
            height: hint.height,
          }}
        >
          <span
            className={cn(
              'absolute left-0 flex items-center gap-1.5 whitespace-nowrap bg-ink text-bg px-1.5 py-0.5 font-mono text-[10px] leading-none',
              hint.top >= 22 ? 'bottom-full mb-0.5' : 'top-full mt-0.5',
            )}
          >
            <span className="text-accent">{hint.label}</span>
            <span className="opacity-70 tabular-nums">{hint.dims}</span>
          </span>
        </div>
      ) : null}
    </div>
  )
}

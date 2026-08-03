import { useCallback, useEffect, useRef } from 'react'
import { cn } from '../lib/cn'
import type { LiveFrame } from './useLiveFrames'

/**
 * The desktop viewport: the latest screencast frame scaled to fit the pane
 * (aspect preserved, centered, letterboxed), acting as the desktop itself.
 * Mouse and keyboard map from the rendered image rect into desktop pixel
 * space, so the mapping survives any pane size: clicks, double clicks, right
 * clicks, wheel scroll, and (while the surface is focused) typing and special
 * keys all forward as `computer::act`. Events in the letterbox margin outside
 * the image do nothing.
 */

const SCROLL_FLUSH_MS = 150
/** Wheel pixels per scroll notch handed to `act`. */
const SCROLL_PIXELS_PER_NOTCH = 60
const DOUBLE_CLICK_WINDOW_MS = 220
/** Keys forwarded as `computer::act {action:'press'}` (worker key names). */
const PRESS_KEYS: Record<string, string> = {
  Enter: 'enter',
  Tab: 'tab',
  Escape: 'escape',
  Backspace: 'backspace',
  Delete: 'delete',
  ArrowUp: 'up',
  ArrowDown: 'down',
  ArrowLeft: 'left',
  ArrowRight: 'right',
  Home: 'home',
  End: 'end',
  PageUp: 'pageup',
  PageDown: 'pagedown',
}

interface ViewportProps {
  frame: LiveFrame | null
  loading: boolean
  error: string | null
  /** Input forwarding is off until a session is selected and interactive. */
  interactive: boolean
  onClickAt: (x: number, y: number, button: 'left' | 'right') => void
  onDoubleClickAt: (x: number, y: number) => void
  onScrollAt: (x: number, y: number, notches: number) => void
  onTextInput: (text: string) => void
  onPressKeys: (keys: string[]) => void
}

export function Viewport({
  frame,
  loading,
  error,
  interactive,
  onClickAt,
  onDoubleClickAt,
  onScrollAt,
  onTextInput,
  onPressKeys,
}: ViewportProps) {
  const surfaceRef = useRef<HTMLDivElement>(null)
  const imgRef = useRef<HTMLImageElement>(null)

  const frameRef = useRef(frame)
  frameRef.current = frame
  const interactiveRef = useRef(interactive)
  interactiveRef.current = interactive
  const onScrollAtRef = useRef(onScrollAt)
  onScrollAtRef.current = onScrollAt

  /** Client point → desktop pixel, null outside the rendered image. */
  const mapToDesktop = useCallback(
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

  // Single vs double click: a first click waits out the double-click window so
  // a dblclick can replace it with one double_click act.
  const pendingClickRef = useRef<number | undefined>(undefined)
  const clearPendingClick = useCallback(() => {
    window.clearTimeout(pendingClickRef.current)
    pendingClickRef.current = undefined
  }, [])
  useEffect(() => clearPendingClick, [clearPendingClick])

  const handleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!interactive) return
    const pt = mapToDesktop(e.clientX, e.clientY)
    if (!pt) return
    if (e.detail >= 2) {
      clearPendingClick()
      return
    }
    clearPendingClick()
    pendingClickRef.current = window.setTimeout(() => {
      pendingClickRef.current = undefined
      onClickAt(pt.x, pt.y, 'left')
    }, DOUBLE_CLICK_WINDOW_MS)
  }

  const handleDoubleClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!interactive) return
    clearPendingClick()
    const pt = mapToDesktop(e.clientX, e.clientY)
    if (!pt) return
    onDoubleClickAt(pt.x, pt.y)
  }

  const handleContextMenu = (e: React.MouseEvent<HTMLDivElement>) => {
    const pt = mapToDesktop(e.clientX, e.clientY)
    if (!pt) return
    e.preventDefault()
    if (!interactive) return
    onClickAt(pt.x, pt.y, 'right')
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
      const pt = mapToDesktop(e.clientX, e.clientY)
      if (!pt) return
      e.preventDefault()
      if (!interactiveRef.current) return
      const scale = e.deltaMode === 1 ? 16 : 1
      accum.delta += e.deltaY * scale
      accum.pt = pt
      if (accum.timer !== undefined) return
      accum.timer = window.setTimeout(() => {
        accum.timer = undefined
        const { delta, pt: point } = accum
        accum.delta = 0
        accum.pt = null
        if (!point || delta === 0) return
        const notches = Math.trunc(delta / SCROLL_PIXELS_PER_NOTCH)
        // A short flick still scrolls: round the sub-notch remainder up to 1.
        const amount = notches !== 0 ? notches : delta > 0 ? 1 : -1
        onScrollAtRef.current(point.x, point.y, amount)
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
  }, [mapToDesktop])

  const handleKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!interactive) return
    const modifiers: string[] = []
    if (e.metaKey) modifiers.push('cmd')
    if (e.ctrlKey) modifiers.push('ctrl')
    if (e.altKey) modifiers.push('alt')
    if (e.shiftKey && (modifiers.length > 0 || PRESS_KEYS[e.key])) {
      modifiers.push('shift')
    }

    const named = PRESS_KEYS[e.key]
    if (named) {
      e.preventDefault()
      e.stopPropagation()
      onPressKeys([...modifiers, named])
      return
    }
    if (e.key.length !== 1) return
    e.preventDefault()
    e.stopPropagation()
    // A chord (cmd+c) is a hotkey; a bare character is text. Chords read the
    // physical key: alt+c reports 'ç' on macOS, and the desktop wants 'c'.
    if (modifiers.length > 0) {
      const physical = /^(Key([A-Z])|Digit([0-9]))$/.exec(e.code)
      const name = (physical?.[2] ?? physical?.[3] ?? e.key).toLowerCase()
      onPressKeys([...modifiers, name])
      return
    }
    onTextInput(e.key)
  }

  return (
    <div
      ref={surfaceRef}
      role="application"
      // biome-ignore lint/a11y/noNoninteractiveTabindex: a live desktop surface forwarding raw mouse/keyboard input; focus is how typing reaches the desktop
      tabIndex={0}
      aria-label="computer viewport: clicks, scrolling, and typing forward to the desktop"
      onPointerDown={() => surfaceRef.current?.focus()}
      onClick={handleClick}
      onDoubleClick={handleDoubleClick}
      onContextMenu={handleContextMenu}
      onKeyDown={handleKeyDown}
      className={cn('cp-ui-vp', interactive && 'is-live')}
    >
      {frame ? (
        <img
          ref={imgRef}
          src={frame.dataUrl}
          alt="live view of the desktop"
          draggable={false}
          className="cp-ui-vp-img"
        />
      ) : (
        <p className="cp-ui-vp-empty">
          {error
            ? `live view failed: ${error}`
            : loading
              ? 'waiting for the first frame...'
              : 'no frame yet'}
        </p>
      )}
    </div>
  )
}

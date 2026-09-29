import { useEffect, useRef } from 'react'
import type { TouchPoint } from '../lib/api'
import type { InputQueue, Screen } from './live'

/**
 * The simulator as an iPhone: a black bezel with the live framebuffer as its
 * screen and the side buttons on its edges. Pointer input maps from the
 * rendered image into device pixels, so it works at any pane size:
 *
 * - drag = one finger (tap, swipe, scroll, long-press);
 * - Option-drag = pinch (a second finger mirrored around the screen center,
 *   as in Simulator.app); Option-Shift-drag = two fingers moving together;
 * - two touch points on a touchscreen = two fingers;
 * - trackpad pinch (ctrl+wheel) = pinch at the cursor; wheel = a drag.
 *
 * While focused, typing reaches the simulator's hardware keyboard; a paste
 * types the clipboard; Shift+Escape leaves.
 */

const WHEEL_IDLE_MS = 140
/** Hardware keys forwarded by name (worker key names). */
const NAMED_KEYS: Record<string, string> = {
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

/** Bezel padding as a fraction of the phone's width (drawn in styles.css). */
const BEZEL = 0.032

interface PhoneProps {
  screen: Screen | null
  /** Shown on the glass while there is no frame. */
  placeholder: string
  input: InputQueue | null
  onButton: (button: string, phase: 'down' | 'up') => void
  onText: (text: string) => void
  onKeys: (keys: string[]) => void
}

type Pt = { x: number; y: number }

export function Phone({ screen, placeholder, input, onButton, onText, onKeys }: PhoneProps) {
  const glassRef = useRef<HTMLDivElement>(null)
  const imgRef = useRef<HTMLImageElement>(null)
  const screenRef = useRef(screen)
  screenRef.current = screen
  const inputRef = useRef(input)
  inputRef.current = input

  // Width / height of the whole phone (screen + bezel), for the CSS fit.
  const aspect = screen && screen.deviceHeight > 0 ? screen.deviceWidth / screen.deviceHeight : 1206 / 2622
  const ratio = 1 / ((1 - 2 * BEZEL) / aspect + 2 * BEZEL)

  /** Client point → device pixel, clamped to the screen. */
  const toDevice = (clientX: number, clientY: number): Pt | null => {
    const s = screenRef.current
    const rect = imgRef.current?.getBoundingClientRect()
    if (!s || !rect || rect.width <= 0 || s.deviceWidth <= 0) return null
    const clamp = (v: number) => Math.min(1, Math.max(0, v))
    return {
      x: clamp((clientX - rect.left) / rect.width) * s.deviceWidth,
      y: clamp((clientY - rect.top) / rect.height) * s.deviceHeight,
    }
  }

  const send = (phase: TouchPoint['phase'], a: Pt, b?: Pt) =>
    inputRef.current?.touch(b ? { phase, x: a.x, y: a.y, x2: b.x, y2: b.y } : { phase, x: a.x, y: a.y })

  // ── pointer: one finger, Option pinch/pan, multi-touch screens ─────────
  const gesture = useRef<{
    mode: 'one' | 'pinch' | 'pan' | 'multi'
    offset: Pt
    pointers: Map<number, Pt>
    last: [Pt, Pt?]
  } | null>(null)

  const mirror = (p: Pt): Pt => {
    const s = screenRef.current
    return s ? { x: s.deviceWidth - p.x, y: s.deviceHeight - p.y } : p
  }

  const second = (p: Pt): Pt | undefined => {
    const g = gesture.current
    if (!g || g.mode === 'one') return undefined
    if (g.mode === 'pinch') return mirror(p)
    return { x: p.x + g.offset.x, y: p.y + g.offset.y }
  }

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    glassRef.current?.focus()
    if (!inputRef.current || e.button !== 0) return
    const p = toDevice(e.clientX, e.clientY)
    if (!p) return
    e.preventDefault()
    e.currentTarget.setPointerCapture(e.pointerId)
    const g = gesture.current
    if (g && e.pointerType === 'touch' && g.pointers.size === 1) {
      // A second finger lands: restart as a two-finger touch.
      const [first] = [...g.pointers.values()]
      g.pointers.set(e.pointerId, p)
      send('up', g.last[0])
      g.mode = 'multi'
      g.last = [first, p]
      send('down', first, p)
      return
    }
    if (g) return
    const mode = e.altKey ? (e.shiftKey ? 'pan' : 'pinch') : 'one'
    const offset = mode === 'pan' ? { x: mirror(p).x - p.x, y: 0 } : { x: 0, y: 0 }
    gesture.current = { mode, offset, pointers: new Map([[e.pointerId, p]]), last: [p] }
    const b = second(p)
    gesture.current.last = [p, b]
    send('down', p, b)
  }

  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const g = gesture.current
    if (!g?.pointers.has(e.pointerId)) return
    const p = toDevice(e.clientX, e.clientY)
    if (!p) return
    g.pointers.set(e.pointerId, p)
    if (g.mode === 'multi') {
      const [a, b] = [...g.pointers.values()]
      g.last = [a, b]
      send('move', a, b)
    } else {
      const b = second(p)
      g.last = [p, b]
      send('move', p, b)
    }
  }

  const onPointerEnd = (e: React.PointerEvent<HTMLDivElement>) => {
    const g = gesture.current
    if (!g?.pointers.has(e.pointerId)) return
    send('up', g.last[0], g.last[1])
    gesture.current = null
  }

  // ── wheel: trackpad pinch (ctrl) and two-finger scroll as a drag ────────
  useEffect(() => {
    const glass = glassRef.current
    if (!glass) return
    let active: { pinch: boolean; at: Pt; spread: number; drag: Pt; timer: number } | null = null
    const finish = () => {
      if (!active) return
      const a = active
      active = null
      if (a.pinch) {
        const [p, q] = pinchPoints(a.at, a.spread)
        inputRef.current?.touch({ phase: 'up', x: p.x, y: p.y, x2: q.x, y2: q.y })
      } else {
        inputRef.current?.touch({ phase: 'up', x: a.drag.x, y: a.drag.y })
      }
    }
    const onWheel = (e: WheelEvent) => {
      const s = screenRef.current
      const rect = imgRef.current?.getBoundingClientRect()
      const at = toDevice(e.clientX, e.clientY)
      if (!s || !rect || !at || !inputRef.current) return
      e.preventDefault()
      const pinch = e.ctrlKey
      if (active && active.pinch !== pinch) finish()
      const scale = s.deviceWidth / rect.width
      const lines = e.deltaMode === 1 ? 16 : 1
      if (!active) {
        active = { pinch, at, spread: s.deviceWidth * 0.25, drag: at, timer: 0 }
        if (pinch) {
          const [p, q] = pinchPoints(at, active.spread)
          inputRef.current.touch({ phase: 'down', x: p.x, y: p.y, x2: q.x, y2: q.y })
        } else {
          inputRef.current.touch({ phase: 'down', x: at.x, y: at.y })
        }
      }
      if (pinch) {
        active.spread = Math.min(s.deviceWidth * 0.9, Math.max(40, active.spread * Math.exp(-e.deltaY * 0.01)))
        const [p, q] = pinchPoints(active.at, active.spread)
        inputRef.current.touch({ phase: 'move', x: p.x, y: p.y, x2: q.x, y2: q.y })
      } else {
        active.drag = {
          x: Math.min(s.deviceWidth, Math.max(0, active.drag.x - e.deltaX * lines * scale)),
          y: Math.min(s.deviceHeight, Math.max(0, active.drag.y - e.deltaY * lines * scale)),
        }
        inputRef.current.touch({ phase: 'move', x: active.drag.x, y: active.drag.y })
      }
      window.clearTimeout(active.timer)
      active.timer = window.setTimeout(finish, WHEEL_IDLE_MS)
    }
    glass.addEventListener('wheel', onWheel, { passive: false })
    return () => {
      glass.removeEventListener('wheel', onWheel)
      if (active) window.clearTimeout(active.timer)
      finish()
    }
    // biome-ignore lint/correctness/useExhaustiveDependencies: reads live values through refs
  }, [])

  // ── keyboard ────────────────────────────────────────────────────────────
  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!inputRef.current) return
    if (e.key === 'Escape' && e.shiftKey) {
      e.preventDefault()
      glassRef.current?.blur()
      return
    }
    // Cmd/Ctrl+V pastes through the paste event below.
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'v') return
    const modifiers = [e.metaKey && 'cmd', e.ctrlKey && 'ctrl', e.altKey && 'alt'].filter(Boolean) as string[]
    const named = NAMED_KEYS[e.key]
    if (named) {
      e.preventDefault()
      onKeys([...modifiers, ...(e.shiftKey ? ['shift'] : []), named])
      return
    }
    if (e.key.length !== 1) return
    e.preventDefault()
    if (modifiers.length > 0) {
      // Physical key, not the produced character (alt+c is 'ç' on a Mac).
      const physical = /^(?:Key([A-Z])|Digit([0-9]))$/.exec(e.code)
      onKeys([...modifiers, (physical?.[1] ?? physical?.[2] ?? e.key).toLowerCase()])
      return
    }
    onText(e.key)
  }

  const onPaste = (e: React.ClipboardEvent<HTMLDivElement>) => {
    const text = e.clipboardData.getData('text/plain')
    if (!text || !inputRef.current) return
    e.preventDefault()
    onText(text)
  }

  const hold = (button: string) => ({
    onPointerDown: (e: React.PointerEvent) => {
      e.preventDefault()
      onButton(button, 'down')
    },
    onPointerUp: () => onButton(button, 'up'),
    onPointerCancel: () => onButton(button, 'up'),
    onKeyDown: (e: React.KeyboardEvent) => {
      if ((e.key === 'Enter' || e.key === ' ') && !e.repeat) onButton(button, 'down')
    },
    onKeyUp: (e: React.KeyboardEvent) => {
      if (e.key === 'Enter' || e.key === ' ') onButton(button, 'up')
    },
  })

  return (
    <div className="ios-ui-fit">
      <div className="ios-ui-phone" style={{ '--ios-ui-ratio': ratio } as React.CSSProperties}>
        <button type="button" className="ios-ui-side ios-ui-side--action" aria-label="Action button" {...hold('action')} />
        <button type="button" className="ios-ui-side ios-ui-side--vol-up" aria-label="Volume up" {...hold('volume_up')} />
        <button
          type="button"
          className="ios-ui-side ios-ui-side--vol-down"
          aria-label="Volume down"
          {...hold('volume_down')}
        />
        <button
          type="button"
          className="ios-ui-side ios-ui-side--power"
          aria-label="Side button (hold for Siri)"
          {...hold('lock')}
        />
        <div className="ios-ui-bezel">
          <div
            ref={glassRef}
            role="application"
            // biome-ignore lint/a11y/noNoninteractiveTabindex: the live screen forwards raw touch and keyboard input; focus is how typing reaches the simulator
            tabIndex={0}
            aria-label="Simulator screen: drag to touch, Option-drag to pinch, type to use the keyboard, Shift+Escape to leave"
            className="ios-ui-glass"
            data-live={input ? 'true' : undefined}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerEnd}
            onPointerCancel={onPointerEnd}
            onLostPointerCapture={onPointerEnd}
            onContextMenu={(e) => e.preventDefault()}
            onKeyDown={onKeyDown}
            onPaste={onPaste}
          >
            {screen ? (
              <img ref={imgRef} src={screen.src} alt="" draggable={false} className="ios-ui-screen" />
            ) : (
              <p className="ios-ui-glass-note">{placeholder}</p>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

/** Two fingers on a diagonal through `at`, `spread` device pixels apart. */
function pinchPoints(at: Pt, spread: number): [Pt, Pt] {
  const half = spread / 2 / Math.SQRT2
  return [
    { x: at.x - half, y: at.y - half },
    { x: at.x + half, y: at.y + half },
  ]
}

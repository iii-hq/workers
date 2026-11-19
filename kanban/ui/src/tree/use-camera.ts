import { usePaneState } from '@iii-dev/console-ui/hooks'
import { type PointerEvent, useCallback, useEffect, useRef, useState } from 'react'
import { type Axis, advance, axis, settled } from './spring'

/**
 * The Tree's camera: one `translate3d(x, y, 0) scale(s)` on the stage that holds the chart, written straight to
 * the element (never through React state, never through a parent's CSS variable), so pan and zoom stay on the
 * compositor and never re-run layout.
 *
 * - Wheel pans; pinch (or ⌘/Ctrl + wheel) zooms about the cursor — both applied at once, one write per frame.
 * - −/+, 100% and Fit glide there over 260ms on a strong ease-out, from wherever the camera is (a second press
 *   mid-glide retargets, it never restarts).
 * - A dragged canvas carries its speed on release and settles with friction.
 * - Fit is sticky: while it holds, a chart that grows or shrinks is re-fitted with the same glide; any pan or
 *   zoom of your own lets go of it.
 * - prefers-reduced-motion: glides and momentum jump to their end; direct manipulation is unchanged.
 */

export const MIN_SCALE = 0.25
export const MAX_SCALE = 1.5
const STEP = 1.25
const GLIDE_MS = 260
/** The chart never leaves the view entirely: this much of it stays inside on each axis. */
const KEEP = 120
/** Room around a fitted chart, and around a card brought into view. */
const FIT_PAD = 32
const REVEAL_PAD = 48
/** A pane resize is over once it has been still this long (a column opening animates its width). */
const RESIZE_SETTLE_MS = 160

type Cam = { x: number; y: number; s: number }

/** cubic-bezier(x1, y1, x2, y2) as an easing function (Newton's method on x, then y). */
export function bezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
  const cx = 3 * x1
  const bx = 3 * (x2 - x1) - cx
  const ax = 1 - cx - bx
  const cy = 3 * y1
  const by = 3 * (y2 - y1) - cy
  const ay = 1 - cy - by
  const sx = (t: number) => ((ax * t + bx) * t + cx) * t
  const sy = (t: number) => ((ay * t + by) * t + cy) * t
  const dx = (t: number) => (3 * ax * t + 2 * bx) * t + cx
  return (x: number) => {
    if (x <= 0) return 0
    if (x >= 1) return 1
    let t = x
    for (let i = 0; i < 8; i += 1) {
      const err = sx(t) - x
      if (Math.abs(err) < 1e-5) break
      const d = dx(t)
      if (Math.abs(d) < 1e-6) break
      t -= err / d
    }
    return sy(Math.min(1, Math.max(0, t)))
  }
}
/** The UI's strong ease-out: starts at once, lands softly — for a move you asked for. */
export const easeOut = bezier(0.23, 1, 0.32, 1)

const clampScale = (s: number) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, s))
const reducedMotion = () =>
  typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true

export type Camera = ReturnType<typeof useCamera>

export function useCamera(paneKey: string) {
  const [stored, setStored] = usePaneState<number>(`kanban:tree-zoom:${paneKey}`, 1)
  const [viewport, setViewport] = useState<HTMLDivElement | null>(null)
  const [stage, setStage] = useState<HTMLDivElement | null>(null)
  const cam = useRef<Cam>({ x: 0, y: 0, s: clampScale(typeof stored === 'number' ? stored : 1) })
  const [scale, setScale] = useState(cam.current.s)
  const [follow, setFollowState] = useState(false)
  const followRef = useRef(false)
  const frame = useRef(0)
  const glide = useRef(0)
  const idle = useRef<number | undefined>(undefined)
  const persist = useRef<number | undefined>(undefined)
  const labelAt = useRef(0)
  const placed = useRef(false)

  const setFollow = useCallback((on: boolean) => {
    followRef.current = on
    setFollowState(on)
  }, [])

  const size = useCallback(() => {
    // offsetWidth/Height ignore transforms: the chart's own size at 100%.
    const w = stage?.offsetWidth ?? 0
    const h = stage?.offsetHeight ?? 0
    const vw = viewport?.clientWidth ?? 0
    const vh = viewport?.clientHeight ?? 0
    return { w, h, vw, vh }
  }, [stage, viewport])

  const clamp = useCallback(
    (c: Cam): Cam => {
      const { w, h, vw, vh } = size()
      const s = clampScale(c.s)
      if (!vw || !w) return { ...c, s }
      const keepX = Math.min(KEEP, (w * s) / 2)
      const keepY = Math.min(KEEP, (h * s) / 2)
      return {
        s,
        x: Math.min(vw - keepX, Math.max(keepX - w * s, c.x)),
        y: Math.min(vh - keepY, Math.max(keepY - h * s, c.y)),
      }
    },
    [size],
  )

  /** Write the camera to the stage now. */
  const apply = useCallback(() => {
    cancelAnimationFrame(frame.current)
    frame.current = 0
    {
      if (!stage) return
      const { x, y, s } = cam.current
      stage.style.transform = `translate3d(${x}px, ${y}px, 0) scale(${s})`
      // A compositor layer only while moving: at rest the text re-rasterises sharp at its final scale.
      stage.style.willChange = 'transform'
      window.clearTimeout(idle.current)
      idle.current = window.setTimeout(() => {
        if (stage) stage.style.willChange = ''
      }, 150)
      const now = performance.now()
      if (now - labelAt.current > 90) {
        labelAt.current = now
        setScale(s)
      }
      window.clearTimeout(persist.current)
      persist.current = window.setTimeout(() => {
        setScale(cam.current.s)
        setStored(Math.round(cam.current.s * 100) / 100)
      }, 300)
    }
  }, [stage, setStored])

  /** Paint on the next frame: wheel and drag input, one write per frame however many events arrived. */
  const paint = useCallback(() => {
    if (frame.current) return
    frame.current = requestAnimationFrame(apply)
  }, [apply])

  /** The follow spring: how far the camera still is from the fit it is heading to, and how fast it is closing. */
  const chase = useRef<{ to: Cam; x: Axis; y: Axis; s: Axis } | null>(null)

  const stop = useCallback(() => {
    cancelAnimationFrame(glide.current)
    glide.current = 0
    chase.current = null
  }, [])

  const write = useCallback(
    (next: Cam) => {
      cam.current = clamp(next)
      paint()
    },
    [clamp, paint],
  )

  /** Glide to `target` from wherever the camera is now; a new glide mid-way starts from there. */
  const glideTo = useCallback(
    (target: Cam) => {
      stop()
      const to = clamp(target)
      if (reducedMotion()) {
        write(to)
        return
      }
      const from = { ...cam.current }
      // Timed from the first frame's own timestamp: a frame's time can trail performance.now(), and a glide timed
      // from the call would start "before" its first frame and stand still.
      let start = -1
      const step = (now: number) => {
        if (start < 0) start = now
        const t = Math.min(1, Math.max(0, (now - start) / GLIDE_MS))
        const k = easeOut(t)
        cam.current = {
          x: from.x + (to.x - from.x) * k,
          y: from.y + (to.y - from.y) * k,
          s: from.s + (to.s - from.s) * k,
        }
        // In this frame, not the next: a glide that paints a frame late trails whatever it moves with.
        apply()
        glide.current = t < 1 ? requestAnimationFrame(step) : 0
      }
      glide.current = requestAnimationFrame(step)
    },
    [apply, clamp, stop, write],
  )

  /**
   * Move the camera by (dx, dy) now, before this frame paints (a re-layout is being kept steady): returns the move
   * it made, which the bounds may have cut short.
   */
  const shiftNow = useCallback(
    (dx: number, dy: number) => {
      stop()
      const before = cam.current
      cam.current = clamp({ ...before, x: before.x + dx, y: before.y + dy })
      apply()
      return { dx: cam.current.x - before.x, dy: cam.current.y - before.y }
    },
    [apply, clamp, stop],
  )

  /** Scale about a point of the viewport (cursor, or the middle). */
  const scaledAt = useCallback((s: number, px: number, py: number): Cam => {
    const c = cam.current
    const next = clampScale(s)
    const k = next / c.s
    return { s: next, x: px - (px - c.x) * k, y: py - (py - c.y) * k }
  }, [])

  const fitTarget = useCallback((): Cam => {
    const { w, h, vw, vh } = size()
    if (!w || !vw) return cam.current
    const s = clampScale(Math.min(1, (vw - FIT_PAD * 2) / w, (vh - FIT_PAD * 2) / h))
    return { s, x: (vw - w * s) / 2, y: Math.max(0, (vh - h * s) / 2) }
  }, [size])

  /** The opening view: the root (centred over the chart) in the middle, the top of the chart at the top. */
  const homeTarget = useCallback(
    (s: number): Cam => {
      const { w, vw } = size()
      return { s, x: (vw - w * s) / 2, y: 0 }
    },
    [size],
  )

  // ── controls ──────────────────────────────────────────────────────────────

  const middle = useCallback(() => {
    const { vw, vh } = size()
    return [vw / 2, vh / 2] as const
  }, [size])

  const zoomIn = useCallback(() => {
    setFollow(false)
    const [mx, my] = middle()
    glideTo(scaledAt(cam.current.s * STEP, mx, my))
  }, [glideTo, middle, scaledAt, setFollow])

  const zoomOut = useCallback(() => {
    setFollow(false)
    const [mx, my] = middle()
    glideTo(scaledAt(cam.current.s / STEP, mx, my))
  }, [glideTo, middle, scaledAt, setFollow])

  const actualSize = useCallback(() => {
    setFollow(false)
    const [mx, my] = middle()
    glideTo(scaledAt(1, mx, my))
  }, [glideTo, middle, scaledAt, setFollow])

  const fit = useCallback(() => {
    setFollow(true)
    glideTo(fitTarget())
  }, [fitTarget, glideTo, setFollow])

  /**
   * While Fit holds, chase the fit of the chart as it is now laid out — on a spring, so a chart that grows again
   * mid-way bends the camera's path and keeps its speed instead of starting a new glide.
   */
  const refit = useCallback(() => {
    if (!followRef.current) return
    const to = clamp(fitTarget())
    if (reducedMotion()) {
      stop()
      write(to)
      return
    }
    const c = cam.current
    const running = chase.current
    if (running) {
      running.x.x = c.x - to.x
      running.y.x = c.y - to.y
      running.s.x = c.s - to.s
      running.to = to
      return
    }
    // From a timed glide (the Fit press) into the chase: the glide hands over where it is, at rest.
    cancelAnimationFrame(glide.current)
    const state = { to, x: axis(c.x - to.x), y: axis(c.y - to.y), s: axis(c.s - to.s) }
    chase.current = state
    // Timed from frame to frame (a frame's own time can trail performance.now()): the first frame only starts it.
    let last = -1
    const step = (now: number) => {
      if (chase.current !== state) return
      const dt = last < 0 ? 0 : Math.max(0, now - last) / 1000
      last = now
      advance(state.x, dt)
      advance(state.y, dt)
      advance(state.s, dt)
      cam.current = { x: state.to.x + state.x.x, y: state.to.y + state.y.x, s: state.to.s + state.s.x }
      apply()
      const done = settled(state.x) && settled(state.y) && Math.abs(state.s.x) < 0.0005 && Math.abs(state.s.v) < 0.005
      if (done) {
        cam.current = state.to
        apply()
        chase.current = null
        glide.current = 0
        return
      }
      glide.current = requestAnimationFrame(step)
    }
    glide.current = requestAnimationFrame(step)
  }, [apply, clamp, fitTarget, stop, write])

  /** Bring an element (a card the keyboard moved to) into view, gliding only as far as needed. */
  const reveal = useCallback(
    (el: Element | null) => {
      if (!el || !viewport) return
      const box = el.getBoundingClientRect()
      const view = viewport.getBoundingClientRect()
      let dx = 0
      let dy = 0
      if (box.left < view.left + REVEAL_PAD) dx = view.left + REVEAL_PAD - box.left
      else if (box.right > view.right - REVEAL_PAD) dx = view.right - REVEAL_PAD - box.right
      if (box.top < view.top + REVEAL_PAD) dy = view.top + REVEAL_PAD - box.top
      else if (box.bottom > view.bottom - REVEAL_PAD) dy = view.bottom - REVEAL_PAD - box.bottom
      if (!dx && !dy) return
      setFollow(false)
      glideTo({ ...cam.current, x: cam.current.x + dx, y: cam.current.y + dy })
    },
    [glideTo, setFollow, viewport],
  )
  const revealRef = useRef(reveal)
  revealRef.current = reveal
  /** The card a resize of the pane must not hide (the selected one), when the camera is not following the chart. */
  const keepInViewRef = useRef<(() => Element | null) | null>(null)

  // ── the stage: first placement, and what to do when the chart changes size ─

  useEffect(() => {
    if (!stage || !viewport) return
    placed.current = false
    let lastView = { vw: 0, vh: 0 }
    // The chart's own growth is corrected by use-chart-motion after each commit; this places the chart the first
    // time, and re-fits when the pane itself is resized.
    const observer = new ResizeObserver(() => {
      const { w, vw, vh } = size()
      if (!w) return
      if (!placed.current) {
        placed.current = true
        stop()
        // Placed before the first paint (not on the next frame), so the chart never flashes at the top left.
        cam.current = clamp(followRef.current ? fitTarget() : homeTarget(cam.current.s))
        const { x, y, s } = cam.current
        stage.style.transform = `translate3d(${x}px, ${y}px, 0) scale(${s})`
        stage.dataset.placed = 'true'
        // A pane re-mounted narrower (a chat column opened beside it) keeps its selected card on screen.
        if (!followRef.current) {
          settle = window.setTimeout(() => revealRef.current(keepInViewRef.current?.() ?? null), RESIZE_SETTLE_MS)
        }
      } else if (followRef.current && (vw !== lastView.vw || vh !== lastView.vh)) {
        refit()
      } else if (lastView.vw && (vw !== lastView.vw || vh !== lastView.vh)) {
        // The pane was resized (a chat column opened beside it): once it settles, bring the kept card back in.
        window.clearTimeout(settle)
        settle = window.setTimeout(() => revealRef.current(keepInViewRef.current?.() ?? null), RESIZE_SETTLE_MS)
      }
      lastView = { vw, vh }
    })
    let settle: number | undefined
    observer.observe(stage)
    observer.observe(viewport)
    return () => {
      observer.disconnect()
      window.clearTimeout(settle)
    }
  }, [stage, viewport, size, stop, clamp, refit, fitTarget, homeTarget])

  // ── wheel: pan, or pinch / ⌘ / Ctrl to zoom about the cursor ──────────────

  useEffect(() => {
    if (!viewport) return
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      stop()
      setFollow(false)
      const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? viewport.clientHeight : 1
      const dx = event.deltaX * unit
      const dy = event.deltaY * unit
      if (event.ctrlKey || event.metaKey) {
        const box = viewport.getBoundingClientRect()
        write(scaledAt(cam.current.s * Math.exp(-dy * 0.0075), event.clientX - box.left, event.clientY - box.top))
      } else {
        const c = cam.current
        write({ ...c, x: c.x - (event.shiftKey && !dx ? dy : dx), y: c.y - (event.shiftKey && !dx ? 0 : dy) })
      }
    }
    viewport.addEventListener('wheel', onWheel, { passive: false })
    return () => viewport.removeEventListener('wheel', onWheel)
  }, [viewport, scaledAt, setFollow, stop, write])

  useEffect(
    () => () => {
      cancelAnimationFrame(frame.current)
      cancelAnimationFrame(glide.current)
      window.clearTimeout(idle.current)
      window.clearTimeout(persist.current)
    },
    [],
  )

  // ── drag the empty canvas, with momentum on release ──────────────────────

  const [panning, setPanning] = useState(false)
  const drag = useRef<{ id: number; x: number; y: number; t: number; vx: number; vy: number } | null>(null)

  const onPointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      if (event.button !== 0 || event.pointerType === 'touch') return
      if ((event.target as HTMLElement).closest('.kb-node, button, a, input, label')) return
      event.preventDefault()
      stop()
      setFollow(false)
      drag.current = { id: event.pointerId, x: event.clientX, y: event.clientY, t: performance.now(), vx: 0, vy: 0 }
      event.currentTarget.setPointerCapture(event.pointerId)
      setPanning(true)
    },
    [setFollow, stop],
  )

  const onPointerMove = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      const d = drag.current
      if (!d || d.id !== event.pointerId) return
      const now = performance.now()
      const dx = event.clientX - d.x
      const dy = event.clientY - d.y
      const dt = Math.max(1, now - d.t)
      // A smoothed velocity (px/ms): the last few moves, not one jittery sample.
      d.vx = d.vx * 0.6 + (dx / dt) * 0.4
      d.vy = d.vy * 0.6 + (dy / dt) * 0.4
      d.x = event.clientX
      d.y = event.clientY
      d.t = now
      write({ ...cam.current, x: cam.current.x + dx, y: cam.current.y + dy })
    },
    [write],
  )

  const onPointerUp = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      const d = drag.current
      if (!d || d.id !== event.pointerId) return
      drag.current = null
      setPanning(false)
      // Held still before letting go: no fling.
      if (performance.now() - d.t > 80 || reducedMotion()) return
      let vx = d.vx
      let vy = d.vy
      if (Math.hypot(vx, vy) < 0.1) return
      let prev = -1
      const step = (now: number) => {
        const dt = prev < 0 ? 16.7 : Math.min(48, Math.max(0, now - prev))
        prev = now
        const decay = 0.95 ** (dt / 16.7)
        vx *= decay
        vy *= decay
        const before = cam.current
        write({ ...before, x: before.x + vx * dt, y: before.y + vy * dt })
        // Stopped by friction, or by the edge of the bounds.
        const moved = cam.current.x !== before.x || cam.current.y !== before.y
        glide.current = Math.hypot(vx, vy) > 0.02 && moved ? requestAnimationFrame(step) : 0
      }
      glide.current = requestAnimationFrame(step)
    },
    [write],
  )

  return {
    viewportRef: setViewport,
    stageRef: setStage,
    scale,
    follow,
    panning,
    zoomIn,
    zoomOut,
    actualSize,
    fit,
    refit,
    shiftNow,
    reveal,
    /** Set the card a resize of the pane keeps on screen. */
    keepInView: (get: (() => Element | null) | null) => {
      keepInViewRef.current = get
    },
    stage,
    /** The scale the camera is at this instant (the label lags a little). */
    scaleNow: () => cam.current.s,
    isFollowing: () => followRef.current,
    handlers: { onPointerDown, onPointerMove, onPointerUp, onPointerCancel: onPointerUp },
  }
}

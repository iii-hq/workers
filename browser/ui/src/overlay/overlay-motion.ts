/**
 * Movement for the live preview: a drag follows the pointer, a released
 * swipe keeps going and slows down (exponential decay of the velocity),
 * and past the viewport's edges the position rubber-bands so a gesture
 * still answers; the caller snaps back inside once things settle.
 * Positions are translate offsets in CSS pixels, velocities px per ms.
 */

export interface Point {
  x: number
  y: number
}

export interface Bounds {
  minX: number
  maxX: number
  minY: number
  maxY: number
}

export interface Sample extends Point {
  /** Event timestamp, ms. */
  t: number
}

/** Pointer travel before a press becomes a drag instead of a tap. */
export const DRAG_THRESHOLD = 6
/** Room kept between the overlay and the viewport's edges. */
export const EDGE_MARGIN = 8
/** Samples older than this do not count toward the release velocity. */
const VELOCITY_WINDOW_MS = 100
/** A fling faster than this is clamped: a flick, not a throw off-screen. */
const MAX_SPEED = 4
/** Speed below which a fling is over. */
const STILL_SPEED = 0.02
/** Time constant of the deceleration: speed halves every ~140 ms. */
const DECAY_MS = 200
const RUBBER = 0.35

/** How far the overlay may translate without leaving the viewport, given
 * where it sits now (`rect`, at translate `current`). */
export function boundsFor(
  rect: { left: number; top: number; right: number; bottom: number },
  current: Point,
  viewport: { width: number; height: number },
): Bounds {
  let minX = current.x - (rect.left - EDGE_MARGIN)
  let maxX = current.x + (viewport.width - EDGE_MARGIN - rect.right)
  let minY = current.y - (rect.top - EDGE_MARGIN)
  let maxY = current.y + (viewport.height - EDGE_MARGIN - rect.bottom)
  // Wider than the viewport: hold still rather than oscillate.
  if (minX > maxX) minX = maxX = (minX + maxX) / 2
  if (minY > maxY) minY = maxY = (minY + maxY) / 2
  return { minX, maxX, minY, maxY }
}

/** The box once its width becomes `width`, before any move: it keeps its
 * aspect ratio and grows (or shrinks) from the corner the pointer layout
 * anchors it to, the bottom right. */
export function resizedFromBottomRight(
  rect: { left: number; top: number; right: number; bottom: number },
  width: number,
): { left: number; top: number; right: number; bottom: number } {
  const current = rect.right - rect.left
  if (current <= 0) return rect
  const height = ((rect.bottom - rect.top) * width) / current
  return {
    left: rect.right - width,
    top: rect.bottom - height,
    right: rect.right,
    bottom: rect.bottom,
  }
}

export function clampPoint(p: Point, b: Bounds): Point {
  return {
    x: Math.min(b.maxX, Math.max(b.minX, p.x)),
    y: Math.min(b.maxY, Math.max(b.minY, p.y)),
  }
}

function rubber(value: number, min: number, max: number): number {
  if (value > max) return max + (value - max) * RUBBER
  if (value < min) return min - (min - value) * RUBBER
  return value
}

/** Inside the bounds the point is itself; past them it moves at a third of the rate. */
export function rubberBandPoint(p: Point, b: Bounds): Point {
  return { x: rubber(p.x, b.minX, b.maxX), y: rubber(p.y, b.minY, b.maxY) }
}

/** How far outside the bounds a point is, in px (0 inside). */
export function overshoot(p: Point, b: Bounds): number {
  const c = clampPoint(p, b)
  return Math.hypot(p.x - c.x, p.y - c.y)
}

/** Release velocity from the tail of a gesture: the oldest sample still
 * inside the window against the newest. Zero without two samples. */
export function velocityFromSamples(
  samples: readonly Sample[],
  now: number,
): Point {
  const recent = samples.filter((s) => now - s.t <= VELOCITY_WINDOW_MS)
  if (recent.length < 2) return { x: 0, y: 0 }
  const first = recent[0]
  const last = recent[recent.length - 1]
  const dt = last.t - first.t
  if (dt <= 0) return { x: 0, y: 0 }
  const v = { x: (last.x - first.x) / dt, y: (last.y - first.y) / dt }
  const speed = Math.hypot(v.x, v.y)
  if (speed <= MAX_SPEED) return v
  return { x: (v.x / speed) * MAX_SPEED, y: (v.y / speed) * MAX_SPEED }
}

/** Velocity after `dt` ms of coasting. */
export function decayVelocity(v: Point, dt: number): Point {
  const k = Math.exp(-dt / DECAY_MS)
  return { x: v.x * k, y: v.y * k }
}

export function isStill(v: Point): boolean {
  return Math.hypot(v.x, v.y) < STILL_SPEED
}

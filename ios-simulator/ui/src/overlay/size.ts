/**
 * Sizing for the simulator preview, after browser/ui/src/overlay/overlay-size.ts:
 * a width the user pinches between a floor and a ceiling, rubber-banded past
 * either, snapped back on release. A phone card is about twice as tall as it
 * is wide, so the ceiling also keeps the whole card inside the viewport's
 * height.
 */

export const OVERLAY_MIN_WIDTH = 90
export const OVERLAY_MAX_WIDTH = 440
/** Room kept between the overlay and the viewport's edges. */
export const OVERLAY_EDGE_GAP = 32
/** Height / width of the tallest simulator screen (iPhone Pro Max). */
const PHONE_ASPECT = 2.2
const RUBBER = 0.25

export interface Limits {
  min: number
  max: number
}

export function overlayLimits(viewportWidth: number, viewportHeight: number): Limits {
  const max = Math.max(
    OVERLAY_MIN_WIDTH,
    Math.min(
      OVERLAY_MAX_WIDTH,
      viewportWidth - OVERLAY_EDGE_GAP,
      (viewportHeight - 2 * OVERLAY_EDGE_GAP) / PHONE_ASPECT,
    ),
  )
  return { min: OVERLAY_MIN_WIDTH, max }
}

/** Width while pinching: proportional inside the limits, damped past them. */
export function pinchWidth(startWidth: number, ratio: number, limits: Limits): number {
  const raw = startWidth * ratio
  if (raw > limits.max) return limits.max + (raw - limits.max) * RUBBER
  if (raw < limits.min) return limits.min - (limits.min - raw) * RUBBER
  return raw
}

/** Width once the fingers lift: inside the limits, whole pixels. */
export function settleWidth(width: number, limits: Limits): number {
  if (!Number.isFinite(width)) return limits.min
  return Math.round(Math.min(limits.max, Math.max(limits.min, width)))
}

/** A double-click alternates the default width and twice it. */
export function toggledWidth(width: number, base: number, limits: Limits): number {
  return settleWidth(width === base ? base * 2 : base, limits)
}

export function pinchDistance(a: { x: number; y: number }, b: { x: number; y: number }): number {
  return Math.hypot(a.x - b.x, a.y - b.y)
}

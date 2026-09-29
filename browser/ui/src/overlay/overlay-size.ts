/**
 * Sizing for the live preview overlay: a width the user pinches between a
 * floor and a ceiling. Past either limit the width follows the fingers at a
 * quarter of the rate (rubber band), so the gesture still answers, and
 * `settle` snaps it back inside on release — the card-resize transition
 * animates that return.
 */

export const OVERLAY_MIN_WIDTH = 120
/** Twice the pointer layout's default width, the double-click's 2x. */
export const OVERLAY_MAX_WIDTH = 880
/** Room kept between the overlay and the far edge of the viewport. */
export const OVERLAY_EDGE_GAP = 32
const RUBBER = 0.25

export interface Limits {
  min: number
  max: number
}

/** The ceiling never exceeds what the viewport can hold. */
export function overlayLimits(viewportWidth: number): Limits {
  const max = Math.max(
    OVERLAY_MIN_WIDTH,
    Math.min(OVERLAY_MAX_WIDTH, viewportWidth - OVERLAY_EDGE_GAP),
  )
  return { min: OVERLAY_MIN_WIDTH, max }
}

/** Width while pinching: proportional inside the limits, damped past them. */
export function pinchWidth(
  startWidth: number,
  ratio: number,
  limits: Limits,
): number {
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

/** A double-click on a pointer layout: the default width and twice it
 * alternate (any other width, a pinch's say, goes back to the default). */
export function toggledWidth(
  width: number,
  base: number,
  limits: Limits,
): number {
  return settleWidth(width === base ? base * 2 : base, limits)
}

export function pinchDistance(
  a: { x: number; y: number },
  b: { x: number; y: number },
): number {
  return Math.hypot(a.x - b.x, a.y - b.y)
}

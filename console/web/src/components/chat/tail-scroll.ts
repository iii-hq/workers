export type TailScrollState = 'initializing' | 'following' | 'paused'

/** Only the actual end re-arms follow; this tolerance covers sub-pixel layout. */
export const TAIL_REARM_DISTANCE_PX = 2

/** Ignore browser rounding noise when comparing consecutive scroll positions. */
export const TAIL_DIRECTION_EPSILON_PX = 0.5

/** A short continuous glide follows streaming layout without lagging behind it. */
export const TAIL_GLIDE_TIME_CONSTANT_MS = 90
export const TAIL_GLIDE_SETTLE_DISTANCE_PX = 1

export interface TailScrollMetrics {
  scrollTop: number
  scrollHeight: number
  clientHeight: number
}

export function tailScrollTarget(metrics: TailScrollMetrics): number {
  return Math.max(0, metrics.scrollHeight - metrics.clientHeight)
}

export function tailDistanceFromBottom(metrics: TailScrollMetrics): number {
  return Math.max(0, tailScrollTarget(metrics) - metrics.scrollTop)
}

export function isAtTail(
  metrics: TailScrollMetrics,
  tolerance = TAIL_REARM_DISTANCE_PX,
): boolean {
  return tailDistanceFromBottom(metrics) <= tolerance
}

export function didScrollUp(
  previousScrollTop: number,
  currentScrollTop: number,
): boolean {
  return currentScrollTop < previousScrollTop - TAIL_DIRECTION_EPSILON_PX
}

/**
 * Position alone cannot distinguish a programmatic downward follow from a
 * person's scroll. The caller records every programmatic write before its
 * scroll event arrives, so a real decrease is the one transition that pauses.
 */
export function tailStateAfterScroll(
  state: TailScrollState,
  previousScrollTop: number,
  metrics: TailScrollMetrics,
): TailScrollState {
  if (didScrollUp(previousScrollTop, metrics.scrollTop)) {
    // Shrinking content (or a taller viewport) clamps scrollTop downward and
    // emits a scroll event even though nobody scrolled. When a state that was
    // already following lands exactly on the new tail, preserve it. A real
    // upward gesture leaves a positive gap — even 400 -> 399 still pauses.
    if (state !== 'paused' && isAtTail(metrics, TAIL_DIRECTION_EPSILON_PX)) {
      return state
    }
    return 'paused'
  }
  if (state === 'paused' && isAtTail(metrics)) return 'following'
  return state
}

/**
 * Exponential target following is frame-rate independent and retargetable: a
 * growing stream updates the destination without restarting or queuing motion.
 */
export function nextTailScrollTop(
  current: number,
  target: number,
  elapsedMs: number,
): number {
  const distance = target - current
  if (Math.abs(distance) <= TAIL_GLIDE_SETTLE_DISTANCE_PX) return target
  const alpha =
    1 - Math.exp(-Math.max(0, elapsedMs) / TAIL_GLIDE_TIME_CONSTANT_MS)
  return current + distance * alpha
}

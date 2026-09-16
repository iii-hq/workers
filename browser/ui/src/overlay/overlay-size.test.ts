import { describe, expect, it } from 'vitest'
import {
  OVERLAY_MAX_WIDTH,
  OVERLAY_MIN_WIDTH,
  overlayLimits,
  pinchWidth,
  settleWidth,
} from './overlay-size'

describe('overlay sizing', () => {
  const limits = { min: OVERLAY_MIN_WIDTH, max: OVERLAY_MAX_WIDTH }

  it('follows the pinch inside the limits and rubber-bands past them', () => {
    expect(pinchWidth(200, 1.5, limits)).toBe(300)
    expect(pinchWidth(200, 0.75, limits)).toBe(150)
    // 200 * 4 = 800, 280 past the ceiling: only a quarter of that shows.
    expect(pinchWidth(200, 4, limits)).toBe(OVERLAY_MAX_WIDTH + 70)
    // 200 * 0.25 = 50, 70 under the floor.
    expect(pinchWidth(200, 0.25, limits)).toBe(OVERLAY_MIN_WIDTH - 17.5)
  })

  it('settles back inside the limits on release', () => {
    expect(settleWidth(OVERLAY_MAX_WIDTH + 70, limits)).toBe(OVERLAY_MAX_WIDTH)
    expect(settleWidth(OVERLAY_MIN_WIDTH - 17.5, limits)).toBe(
      OVERLAY_MIN_WIDTH,
    )
    expect(settleWidth(233.6, limits)).toBe(234)
    expect(settleWidth(Number.NaN, limits)).toBe(OVERLAY_MIN_WIDTH)
  })

  it('caps the ceiling by the viewport but never below the floor', () => {
    expect(overlayLimits(1440)).toEqual(limits)
    expect(overlayLimits(390).max).toBe(390 - 32)
    expect(overlayLimits(100).max).toBe(OVERLAY_MIN_WIDTH)
  })
})

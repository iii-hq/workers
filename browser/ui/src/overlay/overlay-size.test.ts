import { describe, expect, it } from 'vitest'
import {
  OVERLAY_MAX_WIDTH,
  OVERLAY_MIN_WIDTH,
  overlayLimits,
  pinchWidth,
  settleWidth,
  toggledWidth,
} from './overlay-size'

describe('overlay sizing', () => {
  const limits = { min: OVERLAY_MIN_WIDTH, max: OVERLAY_MAX_WIDTH }

  it('follows the pinch inside the limits and rubber-bands past them', () => {
    expect(pinchWidth(200, 1.5, limits)).toBe(300)
    expect(pinchWidth(200, 0.75, limits)).toBe(150)
    // 200 * 6 = 1200, 320 past the ceiling: only a quarter of that shows.
    expect(pinchWidth(200, 6, limits)).toBe(OVERLAY_MAX_WIDTH + 80)
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

  it('a double-click alternates 1x and 2x, from any other width back to 1x', () => {
    expect(toggledWidth(440, 440, limits)).toBe(880)
    expect(toggledWidth(880, 440, limits)).toBe(440)
    expect(toggledWidth(300, 440, limits)).toBe(440)
    // A viewport too narrow for 2x settles at its ceiling, and back.
    expect(toggledWidth(440, 440, overlayLimits(800))).toBe(768)
    expect(toggledWidth(768, 440, overlayLimits(800))).toBe(440)
  })

  it('caps the ceiling by the viewport but never below the floor', () => {
    expect(overlayLimits(1440)).toEqual(limits)
    expect(overlayLimits(390).max).toBe(390 - 32)
    expect(overlayLimits(100).max).toBe(OVERLAY_MIN_WIDTH)
  })
})

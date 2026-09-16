import { describe, expect, it } from 'vitest'
import {
  boundsFor,
  clampPoint,
  decayVelocity,
  EDGE_MARGIN,
  isStill,
  overshoot,
  rubberBandPoint,
  velocityFromSamples,
} from './overlay-motion'

describe('overlay motion', () => {
  const viewport = { width: 1000, height: 800 }
  // A 200×100 box whose top-left sits at (700, 650) with translate (-50, -30).
  const rect = { left: 700, top: 650, right: 900, bottom: 750 }
  const current = { x: -50, y: -30 }
  const bounds = boundsFor(rect, current, viewport)

  it('derives how far the box may travel from where it sits now', () => {
    expect(bounds).toEqual({
      minX: -50 - (700 - EDGE_MARGIN),
      maxX: -50 + (1000 - EDGE_MARGIN - 900),
      minY: -30 - (650 - EDGE_MARGIN),
      maxY: -30 + (800 - EDGE_MARGIN - 750),
    })
    // A box wider than the viewport holds still on that axis.
    const wide = boundsFor(
      { left: -100, top: 0, right: 1100, bottom: 50 },
      { x: 0, y: 0 },
      viewport,
    )
    expect(wide.minX).toBe(wide.maxX)
  })

  it('clamps and rubber-bands past the edges', () => {
    expect(clampPoint({ x: 500, y: -900 }, bounds)).toEqual({
      x: bounds.maxX,
      y: bounds.minY,
    })
    const past = rubberBandPoint(
      { x: bounds.maxX + 100, y: bounds.minY - 100 },
      bounds,
    )
    expect(past.x).toBeCloseTo(bounds.maxX + 35)
    expect(past.y).toBeCloseTo(bounds.minY - 35)
    expect(rubberBandPoint({ x: 0, y: 0 }, bounds)).toEqual({ x: 0, y: 0 })
    expect(
      overshoot({ x: bounds.maxX + 30, y: bounds.minY - 40 }, bounds),
    ).toBe(50)
    expect(overshoot({ x: 0, y: 0 }, bounds)).toBe(0)
  })

  it('reads the release velocity from the recent samples only, capped', () => {
    const samples = [
      { t: 0, x: 0, y: 0 },
      { t: 500, x: 10, y: 0 },
      { t: 550, x: 60, y: 0 },
      { t: 600, x: 110, y: 0 },
    ]
    // Only the samples from t=500 on are within the window: 100 px in 100 ms.
    expect(velocityFromSamples(samples, 600)).toEqual({ x: 1, y: 0 })
    expect(velocityFromSamples([{ t: 600, x: 0, y: 0 }], 600)).toEqual({
      x: 0,
      y: 0,
    })
    const flick = velocityFromSamples(
      [
        { t: 0, x: 0, y: 0 },
        { t: 10, x: 300, y: 400 },
      ],
      10,
    )
    expect(Math.hypot(flick.x, flick.y)).toBeCloseTo(4)
    expect(flick.x / flick.y).toBeCloseTo(0.75)
  })

  it('coasts to a stop', () => {
    let v = { x: 2, y: -1 }
    let steps = 0
    while (!isStill(v) && steps < 1000) {
      v = decayVelocity(v, 16)
      steps += 1
    }
    expect(steps).toBeGreaterThan(20)
    expect(steps).toBeLessThan(200)
    expect(isStill(v)).toBe(true)
  })
})

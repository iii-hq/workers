// Guarded percentage helper. Several trace views divide a part by a total
// duration that can legitimately be 0 (a single instantaneous span, or a
// batch of zero-duration spans), producing NaN/Infinity widths.

import { describe, expect, it } from 'vitest'
import { percentOfTotal } from './percent'

describe('percentOfTotal', () => {
  it('computes a normal percentage', () => {
    expect(percentOfTotal(25, 100)).toBe(25)
  })

  it('returns 0 when the total is 0 (no divide-by-zero NaN)', () => {
    expect(percentOfTotal(5, 0)).toBe(0)
  })

  it('returns 0 when the total is negative', () => {
    expect(percentOfTotal(5, -10)).toBe(0)
  })

  it('clamps above 100 (overlapping spans can exceed wall-clock total)', () => {
    expect(percentOfTotal(150, 100)).toBe(100)
  })

  it('clamps a negative part to 0', () => {
    expect(percentOfTotal(-5, 100)).toBe(0)
  })

  it.each([
    ['NaN part', Number.NaN, 100],
    ['NaN total', 5, Number.NaN],
    ['Infinity total', 5, Number.POSITIVE_INFINITY],
  ])('returns 0 for non-finite input (%s)', (_label, part, total) => {
    expect(percentOfTotal(part, total)).toBe(0)
  })
})

import { describe, expect, it } from 'vitest'
import { centeredScrollTop, clampLine } from './reveal-line'

describe('clampLine', () => {
  it('keeps lines inside the model and floors fractions', () => {
    expect(clampLine(0, 10)).toBe(1)
    expect(clampLine(7.8, 10)).toBe(7)
    expect(clampLine(99, 10)).toBe(10)
    expect(clampLine(Number.NaN, 10)).toBe(1)
    expect(clampLine(3, 0)).toBe(1)
  })
})

describe('centeredScrollTop', () => {
  it('centers the line in the scroller and never goes negative', () => {
    expect(centeredScrollTop(100, 400, 600, 20)).toBe(210)
    expect(centeredScrollTop(0, 10, 600, 20)).toBe(0)
  })
})

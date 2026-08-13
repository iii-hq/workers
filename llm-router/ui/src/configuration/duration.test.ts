import { describe, expect, it } from 'vitest'
import { minutesToMs, msToMinutes } from './duration'

describe('timeout minutes', () => {
  it('converts the router defaults without rounding error', () => {
    expect(msToMinutes(300_000)).toBe(5)
    expect(msToMinutes(120_000)).toBe(2)
    expect(minutesToMs(5)).toBe(300_000)
    expect(minutesToMs(2)).toBe(120_000)
  })

  it('round-trips fractional minutes to whole milliseconds', () => {
    expect(minutesToMs(1.5)).toBe(90_000)
    expect(msToMinutes(90_000)).toBe(1.5)
  })
})

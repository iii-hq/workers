import { describe, expect, it } from 'vitest'
import { ago, clock, dayClock, span, spanRange } from './time'

const NOW = new Date(2026, 9, 3, 14, 5, 0).getTime()

describe('dayClock', () => {
  it('puts the day in the time, so 13:21 above 22:45 is not a sort bug', () => {
    expect(dayClock(new Date(2026, 9, 3, 13, 21).getTime(), NOW)).toBe('Today 13:21')
    expect(dayClock(new Date(2026, 9, 2, 22, 45).getTime(), NOW)).toBe('Yesterday 22:45')
    expect(dayClock(new Date(2026, 9, 1, 9, 5).getTime(), NOW)).toBe('01 Oct 09:05')
  })

  it('reads the calendar day, not the last 24 hours', () => {
    const justAfterMidnight = new Date(2026, 9, 3, 0, 10).getTime()
    expect(dayClock(new Date(2026, 9, 2, 23, 50).getTime(), justAfterMidnight)).toBe('Yesterday 23:50')
  })
})

describe('spans', () => {
  it('writes the length of an analysis', () => {
    expect(span(52_000)).toBe('52 s')
    expect(span(235_000)).toBe('3m 55s')
    expect(span(380_000)).toBe('6m 20s')
    expect(span(3_840_000)).toBe('1h 04m')
    expect(span(-5)).toBe('0 s')
  })

  it('writes a range, once when both ends round alike', () => {
    expect(spanRange(120_000, 360_000)).toBe('2–6 min')
    expect(spanRange(100_000, 110_000)).toBe('2 min')
    expect(spanRange(45_000, 50_000)).toBe('45–50 s')
    expect(spanRange(200, 400)).toBe('1 s')
  })
})

describe('ago and clock', () => {
  it('says how long ago', () => {
    expect(ago(NOW - 20_000, NOW)).toBe('just now')
    expect(ago(NOW - 2 * 60_000, NOW)).toBe('2 min ago')
    expect(ago(NOW - 3 * 3_600_000, NOW)).toBe('3 h ago')
    expect(ago(NOW - 86_400_000, NOW)).toBe('1 day ago')
    expect(ago(NOW + 5000, NOW)).toBe('just now')
  })

  it('pads the clock', () => {
    expect(clock(new Date(2026, 9, 3, 7, 4).getTime())).toBe('07:04')
  })
})

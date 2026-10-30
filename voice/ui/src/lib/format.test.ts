import { describe, expect, it } from 'vitest'
import { formatClock, formatDuration } from './format'

describe('formatDuration', () => {
  it('rounds before choosing the unit', () => {
    expect(formatDuration(59.6)).toBe('1m 0s')
    expect(formatDuration(59.4)).toBe('59s')
    expect(formatDuration(3.25)).toBe('3.3s')
    expect(formatDuration(125)).toBe('2m 5s')
    expect(formatDuration(Number.NaN)).toBe('0s')
  })
})

describe('formatClock', () => {
  it('reads minutes and zero-padded seconds', () => {
    expect(formatClock(6.9)).toBe('0:06')
    expect(formatClock(760)).toBe('12:40')
    expect(formatClock(-1)).toBe('0:00')
  })
})

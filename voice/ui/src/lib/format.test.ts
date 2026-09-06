import { describe, expect, it } from 'vitest'
import { errorMessage, formatBytes, formatDuration } from './format'

describe('formatDuration', () => {
  it('rounds before choosing the unit', () => {
    expect(formatDuration(59.6)).toBe('1m 0s')
    expect(formatDuration(59.4)).toBe('59s')
    expect(formatDuration(3.25)).toBe('3.3s')
    expect(formatDuration(125)).toBe('2m 5s')
    expect(formatDuration(Number.NaN)).toBe('0s')
  })
})

describe('errorMessage', () => {
  it('reads the message off engine error objects', () => {
    expect(errorMessage({ code: 'invocation_failed', message: 'handler error: no voice named x' })).toBe(
      'handler error: no voice named x',
    )
    expect(errorMessage(new Error('boom'))).toBe('boom')
    expect(errorMessage('plain')).toBe('plain')
    expect(errorMessage(42)).toBe('unknown error')
  })
})

describe('formatBytes', () => {
  it('picks the unit by magnitude', () => {
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(2_048)).toBe('2 KB')
    expect(formatBytes(3_500_000)).toBe('4 MB')
    expect(formatBytes(1_250_000_000)).toBe('1.3 GB')
  })
})

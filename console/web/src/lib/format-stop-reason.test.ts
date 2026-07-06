import { describe, expect, it } from 'vitest'
import { formatStopReason } from './format-stop-reason'

describe('formatStopReason', () => {
  it('length: returns the truncation-with-recovery notice', () => {
    const out = formatStopReason('length')
    expect(out).toMatch(/response truncated/)
    expect(out).toMatch(/max output tokens/)
    expect(out).toMatch(/"continue"/)
  })

  it('error with message: surfaces the provider message', () => {
    expect(formatStopReason('error', 'rate-limited')).toBe(
      'response failed: rate-limited',
    )
  })

  it('error without message: returns a fallback', () => {
    expect(formatStopReason('error')).toMatch(/response failed mid-stream/)
  })

  it('aborted: returns the abort notice', () => {
    expect(formatStopReason('aborted')).toBe(
      'response aborted before completion.',
    )
  })

  it('function_call: returns the trigger-paused notice', () => {
    expect(formatStopReason('function_call')).toBe(
      'response paused for function trigger.',
    )
  })
})

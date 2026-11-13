import { describe, expect, it } from 'vitest'
import type { TraceGroup } from '../api/traces'
import { orderTraceGroups } from './groupTraces'

function group(value: string, firstSeenMs: number): TraceGroup {
  return {
    value,
    trace_ids: [`${value}-t`],
    span_count: 1,
    first_seen_ms: firstSeenMs,
    last_seen_ms: firstSeenMs + 1,
    duration_ms: 1,
    error_count: 0,
  }
}

describe('orderTraceGroups', () => {
  it('orders newest first and breaks first_seen ties by value, whatever the input order', () => {
    // Two sessions spawned in the same millisecond: the engine returns
    // them in hash-map order, which differs call to call.
    const a = group('a', 1_000)
    const b = group('b', 1_000)
    const newer = group('z', 2_000)
    expect(orderTraceGroups([b, newer, a]).map((g) => g.value)).toEqual([
      'z',
      'a',
      'b',
    ])
    expect(orderTraceGroups([a, b, newer]).map((g) => g.value)).toEqual([
      'z',
      'a',
      'b',
    ])
  })

  it('returns the input array when it is already in order', () => {
    const groups = [group('z', 2_000), group('a', 1_000)]
    expect(orderTraceGroups(groups)).toBe(groups)
  })
})

// Regression tests for the `isGroupByUnavailable` heuristic.
//
// The regex was tightened from `/function[_ ]not[_ ]found|group_by|404/`
// (which would swallow legitimate errors like "failed to group_by: timeout")
// to `/function[_ ]not[_ ]found/i`. These tests pin the new behavior and
// guard against future loosening that re-introduces silent error swallowing.

import { describe, expect, it } from 'vitest'
import type { TraceGroup } from '../api/traces'
import {
  groupByLabel,
  groupHeading,
  isGroupByUnavailable,
  summarizeGroup,
} from './groupTraces'

function makeGroup(overrides: Partial<TraceGroup> = {}): TraceGroup {
  return {
    value: 'sess-1',
    trace_ids: ['t-1', 't-2'],
    span_count: 12,
    first_seen_ms: 1_700_000_000_000,
    last_seen_ms: 1_700_000_001_000,
    duration_ms: 1_420,
    error_count: 0,
    ...overrides,
  }
}

describe('groupByLabel', () => {
  it('returns human-readable labels for each option', () => {
    expect(groupByLabel('none')).toBe('No grouping')
    expect(groupByLabel('iii.message.id')).toBe('Group by message')
    expect(groupByLabel('iii.session.id')).toBe('Group by session')
    expect(groupByLabel('iii.function.id')).toBe('Group by function')
  })
})

describe('summarizeGroup', () => {
  it('renders pluralization for the error count', () => {
    expect(summarizeGroup(makeGroup({ error_count: 0 }))).toContain('0 errors')
    expect(summarizeGroup(makeGroup({ error_count: 1 }))).toContain('1 error')
    expect(summarizeGroup(makeGroup({ error_count: 5 }))).toContain('5 errors')
  })

  it('formats sub-second durations as ms, multi-second as s, long as m+s', () => {
    expect(summarizeGroup(makeGroup({ duration_ms: 0.4 }))).toContain('<1ms')
    expect(summarizeGroup(makeGroup({ duration_ms: 42 }))).toContain('42ms')
    expect(summarizeGroup(makeGroup({ duration_ms: 1_420 }))).toContain('1.42s')
    expect(summarizeGroup(makeGroup({ duration_ms: 62_000 }))).toContain('1m2s')
  })

  it('includes span_count in the summary', () => {
    expect(summarizeGroup(makeGroup({ span_count: 12 }))).toContain('12 spans')
  })
})

describe('groupHeading', () => {
  it('returns the raw value for normal attributes', () => {
    expect(
      groupHeading(makeGroup({ value: 'fn-foo' }), 'iii.function.id'),
    ).toBe('fn-foo')
  })

  it('rewrites the wildcard session sentinel to a friendlier label', () => {
    // ui::subscribe/unsubscribe calls use `*` for the session id; the
    // raw value would render as an asterisk which is meaningless to the
    // user.
    expect(groupHeading(makeGroup({ value: '*' }), 'iii.session.id')).toBe(
      'all sessions (wildcard)',
    )
  })

  it('does NOT rewrite the wildcard for non-session attributes', () => {
    // A `*` message id should render as `*`, not the session label.
    expect(groupHeading(makeGroup({ value: '*' }), 'iii.message.id')).toBe('*')
  })
})

describe('isGroupByUnavailable', () => {
  it('matches the engine error_code with underscore (function_not_found)', () => {
    expect(
      isGroupByUnavailable(
        new Error('function_not_found: engine::traces::group_by'),
      ),
    ).toBe(true)
  })

  it('matches with a space (function not found)', () => {
    expect(isGroupByUnavailable(new Error('function not found'))).toBe(true)
  })

  it('matches case-insensitively (FUNCTION_NOT_FOUND, Function_Not_Found)', () => {
    expect(isGroupByUnavailable(new Error('FUNCTION_NOT_FOUND'))).toBe(true)
    expect(
      isGroupByUnavailable(new Error('Function_Not_Found in registry')),
    ).toBe(true)
  })

  it('accepts a plain string error', () => {
    expect(isGroupByUnavailable('function_not_found')).toBe(true)
  })

  it('does NOT match a timeout that happens to mention group_by', () => {
    // This is the regression: the old regex matched `/group_by/` and would
    // have classified this as "unavailable", silently swallowing a real
    // timeout error.
    expect(
      isGroupByUnavailable(new Error('failed to group_by: request timeout')),
    ).toBe(false)
  })

  it('does NOT match an HTTP 404 unrelated to function dispatch', () => {
    // Old regex matched `/404/` anywhere. A network 404 from a CDN or
    // proxy in front of the engine would silently degrade the UI.
    expect(
      isGroupByUnavailable(new Error('Network error: 404 Not Found')),
    ).toBe(false)
  })

  it('does NOT match a generic error', () => {
    expect(isGroupByUnavailable(new Error('something went wrong'))).toBe(false)
    expect(isGroupByUnavailable(null)).toBe(false)
    expect(isGroupByUnavailable(undefined)).toBe(false)
  })
})

import { describe, expect, it } from 'vitest'
import {
  formatDelta,
  formatMetricValue,
  rootSessions,
  toggleSession,
} from './sessionComparison'
import type { JsonValue, SessionMeta } from './types'

function session(sessionId: string, metadata?: Record<string, JsonValue>): SessionMeta {
  return {
    session_id: sessionId,
    title: sessionId,
    description: '',
    status: 'idle',
    metadata,
    created_at: 1,
    updated_at: 1,
    message_count: 0,
  }
}

describe('session comparison selection', () => {
  it('keeps only roots and includes every lifecycle status', () => {
    expect(rootSessions([
      session('root'),
      session('child', { parent_session_id: 'root' }),
      session('active'),
    ]).map((item) => item.session_id)).toEqual(['root', 'active'])
  })

  it('enforces the five-session selection limit and supports deselection', () => {
    const five = ['a', 'b', 'c', 'd', 'e']
    expect(toggleSession(five, 'f')).toEqual(five)
    expect(toggleSession(five, 'c')).toEqual(['a', 'b', 'd', 'e'])
  })
})

describe('session comparison presentation', () => {
  it('renders unavailable and partial values without inventing zero', () => {
    expect(formatMetricValue(null)).toBe('—')
    expect(formatMetricValue(undefined)).toBe('—')
    expect(formatMetricValue(0)).toBe('0')
    expect(formatDelta({ absolute: null, percent: null })).toBe('—')
  })

  it('formats objective ratios and deltas without direction colors', () => {
    expect(formatMetricValue(0.25, 'ratio')).toBe('25.0%')
    expect(formatDelta({ absolute: 10, percent: 25 }, 'number')).toBe('Δ +10 · +25.0%')
    expect(formatDelta({ absolute: -0.5, percent: null }, 'cost')).toBe('Δ $-0.500000 · —')
  })
})

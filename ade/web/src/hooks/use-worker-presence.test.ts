import { describe, expect, it } from 'vitest'
import { eventMatchesWorker } from './use-worker-presence'

describe('eventMatchesWorker', () => {
  it('matches the worker name as a whole token, not a substring', () => {
    expect(eventMatchesWorker({ worker: 'ide' }, 'ide')).toBe(true)
    expect(eventMatchesWorker({ worker: 'IDE' }, 'ide')).toBe(true)
    expect(eventMatchesWorker({ worker: 'workers/ide' }, 'ide')).toBe(true)
    expect(eventMatchesWorker({ source: { ref: 'ide@0.3' } }, 'ide')).toBe(true)
    expect(eventMatchesWorker({ worker: 'provider-openai' }, 'ide')).toBe(false)
    expect(eventMatchesWorker({ worker: 'memory-consolidate' }, 'memory')).toBe(
      false,
    )
    expect(eventMatchesWorker({ worker: null }, 'ide')).toBe(false)
  })
})

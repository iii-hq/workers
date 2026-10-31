import { describe, expect, it } from 'vitest'
import { workerListed } from './use-worker-presence'

describe('workerListed', () => {
  it('matches a worker by its exact name', () => {
    const list = { workers: [{ name: 'ide' }, { name: 'memory-consolidate' }] }
    expect(workerListed(list, 'ide')).toBe(true)
    expect(workerListed(list, 'memory-consolidate')).toBe(true)
    expect(workerListed(list, 'memory')).toBe(false)
    expect(workerListed(list, 'IDE')).toBe(false)
  })

  it('reads a malformed answer as absent', () => {
    expect(workerListed(null, 'ide')).toBe(false)
    expect(workerListed({}, 'ide')).toBe(false)
    expect(workerListed({ workers: [null, { name: 3 }] }, 'ide')).toBe(false)
  })
})

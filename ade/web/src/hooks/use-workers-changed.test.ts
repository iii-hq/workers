import { describe, expect, it } from 'vitest'
import { parseWorkersChanged } from './use-workers-changed'

describe('parseWorkersChanged', () => {
  it('reads an engine::on-workers-changed event', () => {
    expect(
      parseWorkersChanged({ event: 'worker_connected', worker_id: 'w-1' }),
    ).toEqual({ event: 'worker_connected', worker_id: 'w-1' })
    expect(
      parseWorkersChanged({ event: 'worker_disconnected', worker_id: 'w-1' })
        ?.event,
    ).toBe('worker_disconnected')
  })

  it('rejects anything else', () => {
    expect(parseWorkersChanged({ event: 'worker_connected' })).toBeNull()
    expect(
      parseWorkersChanged({ operation: 'add', stage: 'done', worker: 'ide' }),
    ).toBeNull()
    expect(parseWorkersChanged(null)).toBeNull()
  })
})

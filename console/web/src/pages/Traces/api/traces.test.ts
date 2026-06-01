// Tests for the traces engine transport. The focus here is the
// exporter-availability signal: a genuinely-disabled memory exporter must
// be distinguishable from a legitimately-empty result so the UI can show
// "no observability" only in the former case (not for every empty filter).

import { beforeEach, describe, expect, it, vi } from 'vitest'

const call = vi.fn()
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: vi.fn(async () => ({ call })),
}))

import { fetchTraces } from './traces'

beforeEach(() => {
  call.mockReset()
})

describe('fetchTraces — exporter availability', () => {
  it('flags exporterDisabled and returns an empty list when the memory exporter is not enabled', async () => {
    call.mockRejectedValueOnce(new Error('memory exporter is not enabled'))

    const res = await fetchTraces()

    expect(res.spans).toEqual([])
    expect(res.exporterDisabled).toBe(true)
  })

  it('does NOT set exporterDisabled for a legitimately empty engine response', async () => {
    call.mockResolvedValueOnce({ spans: [], total: 0, offset: 0, limit: 100 })

    const res = await fetchTraces()

    expect(res.spans).toEqual([])
    expect(res.exporterDisabled).toBeUndefined()
  })

  it('does NOT set exporterDisabled when spans are returned', async () => {
    call.mockResolvedValueOnce({
      spans: [{ trace_id: 't1' }],
      total: 1,
      offset: 0,
      limit: 100,
    })

    const res = await fetchTraces()

    expect(res.exporterDisabled).toBeUndefined()
    expect(res.spans).toHaveLength(1)
  })

  it('rethrows non-exporter errors instead of masking them as empty', async () => {
    call.mockRejectedValueOnce(new Error('connection refused'))

    await expect(fetchTraces()).rejects.toThrow('connection refused')
  })
})

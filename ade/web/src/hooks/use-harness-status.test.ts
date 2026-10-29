import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import { addHarnessToCompose } from './use-harness-status'

describe('addHarnessToCompose', () => {
  it('adds harness through the Compose control plane', async () => {
    const call = vi.fn().mockResolvedValue({ status: 'started' })
    const client = { call } as unknown as Pick<IiiClient, 'call'>

    await addHarnessToCompose(client)

    expect(call).toHaveBeenCalledOnce()
    expect(call).toHaveBeenCalledWith('compose::add', {
      workers: ['harness'],
    })
  })
})

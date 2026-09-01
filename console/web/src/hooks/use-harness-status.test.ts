import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import { addHarnessToCompose } from './use-harness-status'

describe('addHarnessToCompose', () => {
  it('adds harness through the Compose control plane', async () => {
    const trigger = vi.fn().mockResolvedValue({ status: 'started' })
    const client = { trigger } as unknown as Pick<IiiClient, 'trigger'>

    await addHarnessToCompose(client)

    expect(trigger).toHaveBeenCalledOnce()
    expect(trigger).toHaveBeenCalledWith('compose::add', {
      workers: ['harness'],
    })
  })
})

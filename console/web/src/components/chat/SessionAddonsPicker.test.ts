import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import { loadSessionSkills } from './SessionAddonsPicker'

describe('session skill picker', () => {
  it('loads list metadata only and omits skills disabled for model invocation', async () => {
    const trigger = vi.fn(async () => ({
      skills: [
        {
          id: 'review',
          title: 'Review',
          description: 'Review changes',
          modified_at: '2026-08-21T00:00:00Z',
          disable_model_invocation: false,
        },
        {
          id: 'manual-only',
          title: 'Manual only',
          description: 'Only explicit invocation',
          modified_at: '2026-08-21T00:00:00Z',
          disable_model_invocation: true,
        },
      ],
    }))

    expect(
      await loadSessionSkills({
        trigger: trigger as unknown as <T>(
          fn: string,
          payload?: object,
        ) => Promise<T>,
      } as unknown as IiiClient),
    ).toEqual([expect.objectContaining({ id: 'review' })])
    expect(trigger).toHaveBeenCalledTimes(1)
    expect(trigger).toHaveBeenCalledWith(
      'directory::skills::list',
      { include_description: true },
      { timeoutMs: 10_000 },
    )
  })
})

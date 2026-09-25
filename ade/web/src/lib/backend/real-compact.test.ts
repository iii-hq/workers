import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import { realBackend } from './real'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

const user = (id: string, text: string) => ({
  entry_id: id,
  message: { role: 'user', content: [{ type: 'text', text }], timestamp: 1 },
})

describe('realBackend.compactSession', () => {
  const trigger = vi.fn()

  beforeEach(() => {
    trigger.mockReset()
    trigger.mockImplementation(async (id: string) => {
      switch (id) {
        case 'harness::status':
          return null
        case 'session::messages':
          return {
            messages: [user('u1', 'a'), user('u2', 'b'), user('u3', 'c')],
          }
        case 'context::compact':
          return {
            status: 'ok',
            summary: 'S',
            tail_start_index: 1,
            tokens_before: 9,
          }
        case 'session::append':
          throw new Error('session/not_found: gone')
      }
      throw new Error(`unexpected ${id}`)
    })
    vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  })

  // Without the persisted entry nothing anchors on the summary, so the
  // compaction did not happen and must not be reported as done.
  it('reports an error when the compaction entry cannot be persisted', async () => {
    const result = await realBackend.compactSession?.(
      's-1',
      'claude-x' as never,
    )
    expect(result).toEqual({
      status: 'error',
      message: expect.stringContaining('session/not_found'),
    })
    expect(trigger).toHaveBeenCalledWith('session::append', {
      session_id: 's-1',
      custom: {
        custom_type: 'compaction',
        data: expect.objectContaining({
          summary: 'S',
          tail_start_entry_id: 'u2',
        }),
      },
    })
  })
})

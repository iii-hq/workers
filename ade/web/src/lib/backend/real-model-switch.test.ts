import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import { realBackend } from './real'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
const message = (id: string) => ({
  entry_id: id,
  message: {
    role: 'user',
    content: [{ type: 'text', text: id }],
    timestamp: 1,
  },
})
const transcript = [
  message('old'),
  {
    entry_id: 'summary',
    custom: {
      custom_type: 'compaction',
      data: { summary: 'Prior summary', tail_start_entry_id: 'recent' },
    },
  },
  message('recent'),
]

const preview = (
  ...args: Parameters<NonNullable<typeof realBackend.previewModelSwitch>>
) => {
  if (!realBackend.previewModelSwitch)
    throw new Error('Preview is not registered')
  return realBackend.previewModelSwitch(...args)
}

describe('read-only model switch preview', () => {
  const trigger = vi.fn()
  const defaultImplementation = async (id: string) => {
    switch (id) {
      case 'harness::status':
        return {
          status: 'completed',
          context: { categories: { tools: 100, overhead: 65 } },
        }
      case 'harness::context-policy':
        return { allow_prune: true }
      case 'session::messages':
        return { messages: transcript }
      case 'harness::system-prompt::get':
        return { parts: [{ body: 'Session prompt' }] }
      case 'context::assemble':
        return { token_count: 25000, usable: 10000, model_resolved: 'router' }
      default:
        throw new Error(`Unexpected call ${id}`)
    }
  }
  beforeEach(() => {
    trigger.mockReset().mockImplementation(defaultImplementation)
    vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  })
  it('uses the full persisted compacted window, Router budgets and a no-inference preview', async () => {
    await expect(preview('s', 'openai::small', 'low')).resolves.toEqual({
      needsCompaction: true,
      tokens: 25000,
      usable: 10000,
    })
    expect(trigger).toHaveBeenCalledWith(
      'context::assemble',
      expect.objectContaining({
        messages: [message('recent').message],
        model: { id: 'small', provider: 'openai' },
        system_prompt: 'Session prompt',
        options: {
          preview_only: true,
          allow_compaction: false,
          previous_summary: 'Prior summary',
          allow_prune: true,
          request_overhead_tokens: 165,
          thinking_level: 'low',
        },
      }),
    )
    expect(
      trigger.mock.calls.some(([id]) =>
        ['context::compact', 'session::append', 'router::chat'].includes(id),
      ),
    ).toBe(false)
  })
  it('uses Harness policy false for a thinking-bound Opus destination', async () => {
    trigger.mockImplementation(
      async (id: string, _payload: { model?: string }) => {
        if (id === 'harness::context-policy') return { allow_prune: false }
        if (id === 'context::assemble')
          return { token_count: 25000, usable: 10000, model_resolved: 'router' }
        return defaultImplementation(id)
      },
    )
    await preview('s', 'claude-opus-5-5')
    expect(trigger).toHaveBeenCalledWith('harness::context-policy', {
      model: 'claude-opus-5-5',
    })
    expect(trigger).toHaveBeenCalledWith(
      'context::assemble',
      expect.objectContaining({
        options: expect.objectContaining({ allow_prune: false }),
      }),
    )
  })
  it('fails closed when Harness policy is unavailable', async () => {
    trigger.mockImplementation(async (id: string, _payload: unknown) => {
      if (id === 'harness::context-policy') return null
      return defaultImplementation(id)
    })
    await expect(preview('s', 'openai::small')).rejects.toThrow(
      'context policy',
    )
    expect(trigger).not.toHaveBeenCalledWith(
      'context::assemble',
      expect.anything(),
    )
  })
  it('does not request confirmation for a fitting window', async () => {
    trigger.mockImplementation((id, _payload) =>
      id === 'context::assemble'
        ? { token_count: 9000, usable: 10000, model_resolved: 'router' }
        : defaultImplementation(id),
    )
    expect((await preview('s', 'openai::small')).needsCompaction).toBe(false)
  })
  it('does not silently use fallback model limits', async () => {
    trigger.mockImplementation((id, _payload) =>
      id === 'context::assemble'
        ? { token_count: 9000, usable: 10000, model_resolved: 'fallback' }
        : defaultImplementation(id),
    )
    await expect(preview('s', 'openai::small')).rejects.toThrow(
      'context budget',
    )
  })
  it('refuses a live turn before reading or mutating history', async () => {
    trigger.mockResolvedValue({ status: 'running' })
    await expect(preview('s', 'openai::small')).rejects.toThrow('current turn')
    expect(trigger).toHaveBeenCalledTimes(1)
  })
})

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import { realBackend } from './real'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

const outcome = (status: string, extra: Record<string, unknown> = {}) => ({
  session_id: 's-1',
  compaction_id: 'cmp_1',
  status,
  message: `outcome ${status}`,
  entry_ids: ['e_cmp_1_started', 'e_cmp_1_outcome'],
  ...extra,
})

describe('realBackend.compactSession', () => {
  const trigger = vi.fn()

  beforeEach(() => {
    trigger.mockReset()
    vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  })

  it('compacts on the harness and returns every row it wrote', async () => {
    trigger.mockResolvedValue(
      outcome('ok', { tokens_before: 9, tokens_after: 3, summary: 'S' }),
    )
    const result = await realBackend.compactSession?.(
      's-1',
      'anthropic::claude-x' as never,
      200_000,
      '  keep the ids  ',
    )
    expect(result).toEqual({
      status: 'ok',
      tokensBefore: 9,
      autoContinued: false,
      summaryText: 'S',
      compactionEntryId: 'e_cmp_1_outcome',
      ownEntryIds: ['e_cmp_1_started', 'e_cmp_1_outcome'],
    })
    // No catalog window and no made-up output allowance: the router resolves
    // the summariser's real limits on the server.
    expect(trigger).toHaveBeenCalledWith(
      'harness::compact',
      {
        session_id: 's-1',
        model: 'claude-x',
        provider: 'anthropic',
        instructions: 'keep the ids',
      },
      { timeoutMs: 330_000 },
    )
    expect(trigger).toHaveBeenCalledTimes(1)
  })

  it('maps every other outcome', async () => {
    for (const [status, expected] of [
      ['busy', { status: 'busy' }],
      ['empty', { status: 'empty' }],
      ['failed', { status: 'error', message: 'outcome failed' }],
    ] as const) {
      trigger.mockResolvedValueOnce(outcome(status))
      expect(
        await realBackend.compactSession?.(
          's-1',
          'anthropic::claude-x' as never,
        ),
      ).toEqual(expected)
    }
  })

  it('reports a transport failure as an error', async () => {
    trigger.mockRejectedValue(new Error('TIMEOUT: invocation timed out'))
    expect(
      await realBackend.compactSession?.('s-1', 'anthropic::claude-x' as never),
    ).toEqual({
      status: 'error',
      message: expect.stringContaining('TIMEOUT'),
    })
  })
})

describe('realBackend.sendCompactCommand', () => {
  const trigger = vi.fn()

  beforeEach(() => {
    trigger.mockReset().mockResolvedValue({
      session_id: 's-1',
      turn_id: 't-1',
      accepted: true,
      compaction: { compaction_id: 'cmp_1', entry_id: 'e_idem_msg-1' },
    })
    vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  })

  // The command rides the ordinary send, so the harness's one parser decides
  // what `/compact` means for every client, and the idempotency key gives its
  // start notice this row's entry id.
  it('sends the typed command through harness::send', async () => {
    await realBackend.sendCompactCommand?.(
      's-1',
      'anthropic::claude-x' as never,
      '/compact keep the ids',
      'msg-1',
    )
    expect(trigger).toHaveBeenCalledWith('harness::send', {
      session_id: 's-1',
      message: '/compact keep the ids',
      model: 'claude-x',
      provider: 'anthropic',
      idempotency_key: 'msg-1',
    })
  })

  it('surfaces a rejected send', async () => {
    trigger.mockRejectedValue(new Error('harness/invalid_request: read-only'))
    await expect(
      realBackend.sendCompactCommand?.(
        's-1',
        'anthropic::claude-x' as never,
        '/compact',
        'msg-1',
      ),
    ).rejects.toThrow('read-only')
  })
})

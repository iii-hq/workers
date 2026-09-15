import { expect, it, vi } from 'vitest'
import { getIiiClient } from '@/lib/iii-client'
import {
  discoverConversations,
  importConversation,
  previewConversation,
} from './conversation-import'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
it('uses read discovery/preview functions and imports only explicitly selected ids', async () => {
  const trigger = vi.fn().mockResolvedValue({})
  vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  await discoverConversations({
    source: 'codex',
    query: 'project',
    cursor: 'next',
  })
  await previewConversation('claude-code', 'history-1')
  await importConversation('claude-code', 'history-1')
  expect(trigger.mock.calls).toEqual([
    [
      'console::conversations::discover',
      { source: 'codex', query: 'project', cursor: 'next' },
    ],
    [
      'console::conversations::preview',
      { source: 'claude-code', id: 'history-1' },
    ],
    [
      'console::conversations::import',
      { source: 'claude-code', id: 'history-1' },
      { timeoutMs: 120_000 },
    ],
  ])
})

it('keeps separate server IDs when importing the same source twice', async () => {
  const trigger = vi
    .fn()
    .mockResolvedValueOnce({
      session_id: 'new-1',
      imported_messages: 3,
      total_messages: 3,
    })
    .mockResolvedValueOnce({
      session_id: 'new-2',
      imported_messages: 3,
      total_messages: 3,
    })
  vi.mocked(getIiiClient).mockResolvedValue({ trigger } as never)
  const first = await importConversation('codex', 'same-source')
  const second = await importConversation('codex', 'same-source')
  expect(first.session_id).toBe('new-1')
  expect(second.session_id).toBe('new-2')
  expect(trigger).toHaveBeenCalledTimes(2)
})

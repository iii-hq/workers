import { describe, expect, it } from 'vitest'
import type { Conversation } from '@/types/chat'
import { hasWorkingConversation } from './chat-activity'

describe('chat screen activity', () => {
  it('includes working sessions/subagents even when another panel is selected', () => {
    expect(
      hasWorkingConversation([{ status: 'done' }, { status: 'working' }], true),
    ).toBe(true)
  })

  it.each(['idle', 'done', 'error', undefined] as const)(
    'does not hold the screen for %s sessions',
    (status) => {
      expect(hasWorkingConversation([{ status }], true)).toBe(false)
    },
  )

  it('ignores old message flags, drafts and registered triggers', () => {
    const session = {
      status: 'done',
      draftText: 'unsent',
      messages: [
        { role: 'function-trigger', running: true, pendingApproval: true },
      ],
    } as Conversation
    expect(hasWorkingConversation([session], true)).toBe(false)
    expect(hasWorkingConversation([], true)).toBe(false)
  })

  it('releases on disconnect instead of trusting stale working status', () => {
    expect(hasWorkingConversation([{ status: 'working' }], false)).toBe(false)
  })
})

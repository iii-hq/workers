import { beforeEach, describe, expect, it } from 'vitest'
import {
  clearChatMessageFocus,
  getChatMessageFocus,
  requestChatMessageFocus,
  resetTraceLinksForTests,
} from './trace-links'

beforeEach(resetTraceLinksForTests)

describe('chat message focus (traces → chat)', () => {
  it('carries the session and the harness turn id', () => {
    const event = requestChatMessageFocus({
      sessionId: 'console-1',
      turnId: 't_abc',
    })
    expect(getChatMessageFocus()).toBe(event)
    expect(event.sessionId).toBe('console-1')
    expect(event.turnId).toBe('t_abc')
  })

  it('a newer request replaces the pending one', () => {
    const first = requestChatMessageFocus({
      sessionId: 'console-1',
      turnId: 't_a',
    })
    const second = requestChatMessageFocus({
      sessionId: 'console-1',
      turnId: 't_b',
    })
    expect(second.id).toBe(first.id + 1)
    expect(getChatMessageFocus()).toBe(second)
  })

  it('clear is id-guarded and idempotent', () => {
    const first = requestChatMessageFocus({
      sessionId: 'console-1',
      turnId: 't_a',
    })
    const second = requestChatMessageFocus({
      sessionId: 'console-1',
      turnId: 't_b',
    })
    clearChatMessageFocus(first.id)
    expect(getChatMessageFocus()).toBe(second)
    clearChatMessageFocus(second.id)
    expect(getChatMessageFocus()).toBeUndefined()
    clearChatMessageFocus(second.id)
  })
})

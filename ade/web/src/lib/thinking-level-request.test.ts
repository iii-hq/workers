import { describe, expect, it, vi } from 'vitest'

import {
  onThinkingLevelChangeRequest,
  requestThinkingLevelChange,
} from './thinking-level-request'

describe('thinking-level requests', () => {
  it('routes a normalized request to the first matching session', () => {
    const ignored = vi.fn(() => false)
    const accepted = vi.fn(() => true)
    const duplicate = vi.fn(() => true)
    const disposeIgnored = onThinkingLevelChangeRequest(ignored)
    const disposeAccepted = onThinkingLevelChangeRequest(accepted)
    const disposeDuplicate = onThinkingLevelChangeRequest(duplicate)

    expect(
      requestThinkingLevelChange({
        sessionId: ' session-1 ',
        level: ' minimal ',
      }),
    ).toBe(true)
    expect(accepted).toHaveBeenCalledWith({
      sessionId: 'session-1',
      level: 'minimal',
    })
    expect(duplicate).not.toHaveBeenCalled()

    disposeIgnored()
    disposeAccepted()
    disposeDuplicate()
  })

  it('accepts a request for the lowest effort the model has', () => {
    const listener = vi.fn(() => true)
    const dispose = onThinkingLevelChangeRequest(listener)
    expect(
      requestThinkingLevelChange({ sessionId: 'session-1', level: 'lowest' }),
    ).toBe(true)
    expect(listener).toHaveBeenCalledWith({
      sessionId: 'session-1',
      level: 'lowest',
    })
    expect(
      requestThinkingLevelChange({
        sessionId: 'session-1',
        level: 'off_or_lowest',
      }),
    ).toBe(true)
    dispose()
  })

  it('rejects incomplete requests and removes listeners', () => {
    const listener = vi.fn(() => true)
    const dispose = onThinkingLevelChangeRequest(listener)

    expect(
      requestThinkingLevelChange({ sessionId: '', level: 'minimal' }),
    ).toBe(false)
    expect(
      requestThinkingLevelChange({ sessionId: 'session-1', level: '' }),
    ).toBe(false)
    dispose()
    expect(
      requestThinkingLevelChange({ sessionId: 'session-1', level: 'minimal' }),
    ).toBe(false)
    expect(listener).not.toHaveBeenCalled()
  })

  /**
   * A page can only ask for a level the console actually offers: an unknown
   * one would be written to the conversation and then sent to the provider
   * as a reasoning effort it does not know.
   */
  it('refuses a level the console does not offer', () => {
    const listener = vi.fn(() => true)
    const dispose = onThinkingLevelChangeRequest(listener)

    expect(
      requestThinkingLevelChange({
        sessionId: 'session-1',
        level: 'ludicrous',
      }),
    ).toBe(false)
    expect(listener).not.toHaveBeenCalled()
    dispose()
  })
})

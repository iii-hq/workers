import { describe, expect, it, vi } from 'vitest'
import type { RegisterTriggerInput } from '@/lib/iii-client'
import { subscribeSessionTranscript } from './events'
import type { MessageAddedEvent } from './types'

describe('subscribeSessionTranscript', () => {
  it('gives simultaneous transcript bindings unique handler ids and routes by session', () => {
    const handlers = new Map<
      string,
      (payload: unknown) => void | Promise<void>
    >()
    const registrations: RegisterTriggerInput[] = []
    const handlerCleanups: Array<ReturnType<typeof vi.fn>> = []
    const triggerCleanups: Array<ReturnType<typeof vi.fn>> = []
    const client = {
      browserId: 'browser-1',
      on: vi.fn(
        (
          functionId: string,
          handler: (payload: unknown) => void | Promise<void>,
        ) => {
          handlers.set(functionId, handler)
          const cleanup = vi.fn(() => handlers.delete(functionId))
          handlerCleanups.push(cleanup)
          return cleanup
        },
      ),
      registerTrigger: vi.fn((input: RegisterTriggerInput) => {
        registrations.push(input)
        const cleanup = vi.fn()
        triggerCleanups.push(cleanup)
        return cleanup
      }),
    } as unknown as Parameters<typeof subscribeSessionTranscript>[0]
    const firstAdded = vi.fn()
    const secondAdded = vi.fn()

    const offFirst = subscribeSessionTranscript(client, 'session-a', {
      onMessageAdded: firstAdded,
      onMessageUpdated: vi.fn(),
    })
    const offSecond = subscribeSessionTranscript(client, 'session-b', {
      onMessageAdded: secondAdded,
      onMessageUpdated: vi.fn(),
    })

    const localHandlerIds = [...handlers.keys()]
    expect(localHandlerIds).toHaveLength(4)
    expect(new Set(localHandlerIds).size).toBe(4)
    expect(
      registrations.map((registration) => registration.function_id),
    ).toEqual(localHandlerIds.map((id) => `${id}::browser-1`))
    expect(registrations.map((registration) => registration.config)).toEqual([
      { session_id: 'session-a' },
      { session_id: 'session-a' },
      { session_id: 'session-b' },
      { session_id: 'session-b' },
    ])

    const firstAddedHandler = handlers.get(localHandlerIds[0])
    expect(firstAddedHandler).toBeDefined()
    const event = (sessionId: string): MessageAddedEvent => ({
      session_id: sessionId,
      entry_id: `entry-${sessionId}`,
      parent_id: null,
      timestamp: 1,
    })
    firstAddedHandler?.(event('session-b'))
    firstAddedHandler?.(event('session-a'))
    expect(firstAdded).toHaveBeenCalledTimes(1)
    expect(firstAdded).toHaveBeenCalledWith(event('session-a'))
    expect(secondAdded).not.toHaveBeenCalled()

    offFirst()
    offSecond()
    for (const cleanup of [...handlerCleanups, ...triggerCleanups]) {
      expect(cleanup).toHaveBeenCalledOnce()
    }
  })
})

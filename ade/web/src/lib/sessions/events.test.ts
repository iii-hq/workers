import { describe, expect, it, vi } from 'vitest'
import type { ObserveSource } from '@/lib/iii-client'
import { subscribeSessionTranscript } from './events'
import type { MessageAddedEvent } from './types'

describe('subscribeSessionTranscript', () => {
  it('gives simultaneous transcript observers unique handler ids and routes by session', () => {
    const handlers = new Map<
      string,
      (payload: unknown) => void | Promise<void>
    >()
    const observers: Array<{ source: ObserveSource; target: string }> = []
    const handlerCleanups: Array<ReturnType<typeof vi.fn>> = []
    const observerCleanups: Array<ReturnType<typeof vi.fn>> = []
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
      observe: vi.fn((source: ObserveSource, target: string) => {
        observers.push({ source, target })
        const cleanup = vi.fn()
        observerCleanups.push(cleanup)
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
    expect(observers.map(({ target }) => target)).toEqual(
      localHandlerIds.map((id) => `${id}::browser-1`),
    )
    expect(observers.map(({ source }) => source)).toEqual([
      {
        function_id: 'session::on-message-added',
        filters: { 'input.session_id': { $eq: 'session-a' } },
      },
      {
        function_id: 'session::on-message-updated',
        filters: { 'input.session_id': { $eq: 'session-a' } },
      },
      {
        function_id: 'session::on-message-added',
        filters: { 'input.session_id': { $eq: 'session-b' } },
      },
      {
        function_id: 'session::on-message-updated',
        filters: { 'input.session_id': { $eq: 'session-b' } },
      },
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
    for (const cleanup of [...handlerCleanups, ...observerCleanups]) {
      expect(cleanup).toHaveBeenCalledOnce()
    }
  })
})

describe('transcript setup rollback', () => {
  it.each([1, 2])(
    'releases partial registrations when observer %s fails',
    (failedObserver) => {
      const handlerCleanups: Array<ReturnType<typeof vi.fn>> = []
      const offObserver = vi.fn()
      let registrations = 0
      const client = {
        browserId: 'rollback-browser',
        on: vi.fn(() => {
          const off = vi.fn()
          handlerCleanups.push(off)
          return off
        }),
        observe: vi.fn(() => {
          registrations += 1
          if (registrations === failedObserver) {
            throw new Error('observe failed')
          }
          return offObserver
        }),
      } as unknown as Parameters<typeof subscribeSessionTranscript>[0]
      expect(() =>
        subscribeSessionTranscript(client, 'session-a', {
          onMessageAdded: vi.fn(),
          onMessageUpdated: vi.fn(),
        }),
      ).toThrow('observe failed')
      expect(handlerCleanups).toHaveLength(failedObserver)
      for (const off of handlerCleanups) expect(off).toHaveBeenCalledOnce()
      expect(offObserver).toHaveBeenCalledTimes(failedObserver - 1)
    },
  )
})

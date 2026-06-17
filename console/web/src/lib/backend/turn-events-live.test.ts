import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import type {
  TurnCompletedEvent,
  TurnStartedEvent,
} from '@/types/iii-agent-event'
import { startTurnEventsSubscription } from './turn-events-live'

function fakeClient() {
  const triggers: Array<{
    type: string
    function_id: string
    config?: unknown
  }> = []
  const handlers = new Map<string, (p: unknown) => void>()
  const offHandler = vi.fn()
  const triggerUnregister = vi.fn()

  const on = vi.fn((fn: string, h: (p: unknown) => void) => {
    handlers.set(fn, h)
    return offHandler
  })
  const registerTrigger = vi.fn(
    (input: { type: string; function_id: string; config?: unknown }) => {
      triggers.push(input)
      return triggerUnregister
    },
  )

  const client = {
    browserId: 'console-test',
    on,
    registerTrigger,
    trigger: vi.fn(),
    addConnectionStateListener: vi.fn(),
    dispose: vi.fn(async () => {}),
  } as unknown as IiiClient

  return {
    client,
    on,
    triggers,
    offHandler,
    triggerUnregister,
    fire: (fn: string, payload: unknown) => handlers.get(fn)?.(payload),
  }
}

describe('startTurnEventsSubscription', () => {
  it('binds harness::turn-completed scoped to the session', () => {
    const { client, on, triggers } = fakeClient()
    startTurnEventsSubscription(client, 'sess-1', { onCompleted: () => {} })

    expect(on).toHaveBeenCalledWith(
      'iii::console::turn_completed',
      expect.any(Function),
    )
    expect(triggers).toEqual([
      {
        type: 'harness::turn-completed',
        function_id: 'iii::console::turn_completed::console-test',
        config: { session_id: 'sess-1' },
      },
    ])
  })

  it('delivers a turn-completed payload to onCompleted', () => {
    const { client, fire } = fakeClient()
    const onCompleted = vi.fn()
    startTurnEventsSubscription(client, 'sess-1', { onCompleted })

    const event: TurnCompletedEvent = {
      session_id: 'sess-1',
      turn_id: 't-1',
      status: 'completed',
      timestamp: 1,
    }
    fire('iii::console::turn_completed', event)
    expect(onCompleted).toHaveBeenCalledWith(event)
  })

  it('also binds turn-started when an onStarted handler is given', () => {
    const { client, triggers, fire } = fakeClient()
    const onStarted = vi.fn()
    startTurnEventsSubscription(client, 'sess-1', {
      onCompleted: () => {},
      onStarted,
    })

    expect(triggers.map((t) => t.type)).toEqual([
      'harness::turn-completed',
      'harness::turn-started',
    ])
    const event: TurnStartedEvent = {
      session_id: 'sess-1',
      turn_id: 't-1',
      timestamp: 1,
    }
    fire('iii::console::turn_started', event)
    expect(onStarted).toHaveBeenCalledWith(event)
  })

  it('does not bind turn-started without an onStarted handler', () => {
    const { client, triggers } = fakeClient()
    startTurnEventsSubscription(client, 'sess-1', { onCompleted: () => {} })
    expect(triggers).toHaveLength(1)
  })

  it('unregisters handler and trigger on cleanup', () => {
    const { client, offHandler, triggerUnregister } = fakeClient()
    const stop = startTurnEventsSubscription(client, 'sess-1', {
      onCompleted: () => {},
    })
    stop()
    expect(offHandler).toHaveBeenCalledTimes(1)
    expect(triggerUnregister).toHaveBeenCalledTimes(1)
  })
})

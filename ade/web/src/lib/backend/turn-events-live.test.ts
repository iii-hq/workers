import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import type {
  MessageQueuedEvent,
  ObserversChangedEvent,
  TurnCompletedEvent,
  TurnStartedEvent,
} from '@/types/iii-agent-event'
import type { HarnessStatusReport } from './harness-send'
import {
  startObserversChangedSubscription,
  startQueuedEventsSubscription,
  startTurnEventsSubscription,
  turnCompletionFromStatus,
} from './turn-events-live'

function fakeClient() {
  const observers: Array<{
    source: { function_id: string; filters?: unknown }
    target: string
  }> = []
  const handlers = new Map<string, (p: unknown) => void>()
  const offHandler = vi.fn()
  const disposeObserver = vi.fn()

  const on = vi.fn((fn: string, h: (p: unknown) => void) => {
    handlers.set(fn, h)
    return offHandler
  })
  const observe = vi.fn(
    (source: { function_id: string; filters?: unknown }, target: string) => {
      observers.push({ source, target })
      return disposeObserver
    },
  )

  const client = {
    browserId: 'console-test',
    on,
    observe,
    call: vi.fn(),
    addConnectionStateListener: vi.fn(),
    dispose: vi.fn(async () => {}),
  } as unknown as IiiClient

  return {
    client,
    on,
    observers,
    offHandler,
    disposeObserver,
    fire: (fn: string, payload: unknown) => handlers.get(fn)?.(payload),
  }
}

const sessionFilter = (id: string) => ({ 'input.session_id': { $eq: id } })

describe('startTurnEventsSubscription', () => {
  it('observes harness::on-turn-completed scoped to the session', () => {
    const { client, on, observers } = fakeClient()
    startTurnEventsSubscription(client, 'sess-1', { onCompleted: () => {} })

    expect(on).toHaveBeenCalledWith(
      'iii::console::turn_completed',
      expect.any(Function),
    )
    expect(observers).toEqual([
      {
        source: {
          function_id: 'harness::on-turn-completed',
          filters: sessionFilter('sess-1'),
        },
        target: 'iii::console::turn_completed::console-test',
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
    const { client, observers, fire } = fakeClient()
    const onStarted = vi.fn()
    startTurnEventsSubscription(client, 'sess-1', {
      onCompleted: () => {},
      onStarted,
    })

    expect(observers.map((o) => o.source.function_id)).toEqual([
      'harness::on-turn-completed',
      'harness::on-turn-started',
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
    const { client, observers } = fakeClient()
    startTurnEventsSubscription(client, 'sess-1', { onCompleted: () => {} })
    expect(observers).toHaveLength(1)
  })

  it('drops the handler and disposes the observer on cleanup', () => {
    const { client, offHandler, disposeObserver } = fakeClient()
    const stop = startTurnEventsSubscription(client, 'sess-1', {
      onCompleted: () => {},
    })
    stop()
    expect(offHandler).toHaveBeenCalledTimes(1)
    expect(disposeObserver).toHaveBeenCalledTimes(1)
  })
})

describe('startQueuedEventsSubscription', () => {
  it('observes harness::on-message-queued scoped to the session and delivers', () => {
    const { client, observers, fire } = fakeClient()
    const onQueued = vi.fn()
    startQueuedEventsSubscription(client, 'sess-1', onQueued)

    expect(observers).toEqual([
      {
        source: {
          function_id: 'harness::on-message-queued',
          filters: sessionFilter('sess-1'),
        },
        target: 'iii::console::message_queued::console-test',
      },
    ])
    const event: MessageQueuedEvent = {
      session_id: 'sess-1',
      entry_id: 'entry-1',
      queued_at: 1,
      timestamp: 2,
    }
    fire('iii::console::message_queued', event)
    expect(onQueued).toHaveBeenCalledWith(event)
  })

  it('drops the handler and disposes the observer on cleanup', () => {
    const { client, offHandler, disposeObserver } = fakeClient()
    const stop = startQueuedEventsSubscription(client, 'sess-1', () => {})
    stop()
    expect(offHandler).toHaveBeenCalledTimes(1)
    expect(disposeObserver).toHaveBeenCalledTimes(1)
  })
})

describe('startObserversChangedSubscription', () => {
  it('observes harness::on-observers-changed scoped to the session and delivers', () => {
    const { client, observers, fire } = fakeClient()
    const onChanged = vi.fn()
    startObserversChangedSubscription(client, 'sess-1', onChanged)

    expect(observers).toEqual([
      {
        source: {
          function_id: 'harness::on-observers-changed',
          filters: sessionFilter('sess-1'),
        },
        target: 'iii::console::observers_changed::console-test',
      },
    ])
    const event: ObserversChangedEvent = { session_id: 'sess-1', timestamp: 1 }
    fire('iii::console::observers_changed', event)
    expect(onChanged).toHaveBeenCalledWith(event)
  })

  it("drops another session's doorbell (shared handler id)", () => {
    const { client, fire } = fakeClient()
    const onChanged = vi.fn()
    startObserversChangedSubscription(client, 'sess-1', onChanged)

    fire('iii::console::observers_changed', {
      session_id: 'sess-2',
      timestamp: 1,
    })
    expect(onChanged).not.toHaveBeenCalled()
  })

  it('drops the handler and disposes the observer on cleanup', () => {
    const { client, offHandler, disposeObserver } = fakeClient()
    const stop = startObserversChangedSubscription(client, 'sess-1', () => {})
    stop()
    expect(offHandler).toHaveBeenCalledTimes(1)
    expect(disposeObserver).toHaveBeenCalledTimes(1)
  })
})

describe('catch-up support', () => {
  it('exposes the observers of a turn subscription for an ordered catch-up', () => {
    const { client } = fakeClient()
    const stop = startTurnEventsSubscription(client, 'sess-a', {
      onCompleted: () => {},
      onStarted: () => {},
    })
    expect(stop.observers).toHaveLength(2)
    expect(
      startQueuedEventsSubscription(client, 'sess-a', () => {}).observers,
    ).toHaveLength(1)
  })

  const status = (patch: Partial<HarnessStatusReport> = {}) =>
    ({
      session_id: 'sess-a',
      turn_id: 'turn-1',
      status: 'completed',
      step: 3,
      turn_count: 1,
      depth: 0,
      max_turns: 0,
      validation_retries: 0,
      max_validation_retries: 0,
      transient_resumes: 0,
      max_transient_resumes: 0,
      partial_result_available: false,
      pending_function_calls: [],
      children: [],
      expects_wake: false,
      ...patch,
    }) as HarnessStatusReport

  it('recovers a completion lost while the observer was suspended', () => {
    expect(
      turnCompletionFromStatus(
        status({ status: 'failed', result_error: 'boom' }),
        'turn-1',
      ),
    ).toMatchObject({
      session_id: 'sess-a',
      turn_id: 'turn-1',
      status: 'failed',
      result_error: 'boom',
      terminal: true,
    })
  })

  it('recovers nothing while the turn runs, parks for a wake, or is another turn', () => {
    expect(
      turnCompletionFromStatus(status({ status: 'running' }), 'turn-1'),
    ).toBeNull()
    expect(
      turnCompletionFromStatus(status({ expects_wake: true }), 'turn-1'),
    ).toBeNull()
    expect(turnCompletionFromStatus(status(), 'turn-0')).toBeNull()
    expect(turnCompletionFromStatus(null, 'turn-1')).toBeNull()
  })
})

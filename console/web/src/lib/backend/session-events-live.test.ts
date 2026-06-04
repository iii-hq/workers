import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import type { AgentEvent } from '@/types/iii-agent-event'
import {
  extractSessionEvent,
  startSessionEventsSubscription,
} from './session-events-live'

/**
 * Build the raw `agent::events` stream frame the engine delivers to a stream
 * trigger handler: `serde_json::to_value(StreamWrapperMessage)` →
 * `{ type, timestamp, streamName, groupId, id, event: { data } }`, where
 * `event.data` is the AgentEvent the harness wrote via `stream::set`.
 */
function frame(groupId: string, event: AgentEvent, opts?: { flat?: boolean }) {
  if (opts?.flat) {
    return { groupId, streamName: 'agent::events', data: event }
  }
  return {
    type: 'set',
    timestamp: 1,
    streamName: 'agent::events',
    groupId,
    id: `${groupId}-epoch-00000000`,
    event: { data: event },
  }
}

const UPDATE = { type: 'message_update' } as unknown as AgentEvent
const END = { type: 'agent_end', messages: [] } as unknown as AgentEvent

describe('extractSessionEvent', () => {
  it('extracts the inner AgentEvent from a real engine frame (event.data)', () => {
    expect(extractSessionEvent(frame('sess-1', UPDATE), 'sess-1')).toEqual(
      UPDATE,
    )
  })

  it('falls back to a flat `data` field when there is no event wrapper', () => {
    expect(
      extractSessionEvent(frame('sess-1', END, { flat: true }), 'sess-1'),
    ).toEqual(END)
  })

  it('accepts the snake_case group_id key as well as camelCase groupId', () => {
    const snake = { group_id: 'sess-1', data: UPDATE }
    expect(extractSessionEvent(snake, 'sess-1')).toEqual(UPDATE)
  })

  it('drops a frame whose group_id is a different session', () => {
    expect(extractSessionEvent(frame('sess-2', UPDATE), 'sess-1')).toBeNull()
  })

  it('drops a frame with no group_id (cannot confirm ownership)', () => {
    expect(extractSessionEvent({ data: UPDATE }, 'sess-1')).toBeNull()
  })

  it('returns null for null / non-object / payload-less frames', () => {
    expect(extractSessionEvent(null, 'sess-1')).toBeNull()
    expect(extractSessionEvent('nope', 'sess-1')).toBeNull()
    expect(extractSessionEvent({ groupId: 'sess-1' }, 'sess-1')).toBeNull()
  })
})

function fakeClient() {
  const triggers: Array<{
    type: string
    function_id: string
    config?: unknown
  }> = []
  let handler: ((p: unknown) => void) | null = null
  const offHandler = vi.fn()
  const triggerUnregister = vi.fn()

  const on = vi.fn((_fn: string, h: (p: unknown) => void) => {
    handler = h
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
    call: vi.fn(),
    addConnectionStateListener: vi.fn(),
    dispose: vi.fn(async () => {}),
  } as unknown as IiiClient

  return {
    client,
    on,
    registerTrigger,
    triggers,
    offHandler,
    triggerUnregister,
    fire: (f: unknown) => handler?.(f),
  }
}

describe('startSessionEventsSubscription', () => {
  it('registers an iii::-prefixed handler and a stream trigger scoped to the session', () => {
    const { client, on, triggers } = fakeClient()

    startSessionEventsSubscription(client, 'sess-1', () => {})

    expect(on).toHaveBeenCalledWith(
      'iii::console::session_event',
      expect.any(Function),
    )
    expect(triggers).toEqual([
      {
        type: 'stream',
        function_id: 'iii::console::session_event::console-test',
        config: { stream_name: 'agent::events', group_id: 'sess-1' },
      },
    ])
  })

  it('delivers each extracted AgentEvent for this session to onEvent', () => {
    const { client, fire } = fakeClient()
    const onEvent = vi.fn()

    startSessionEventsSubscription(client, 'sess-1', onEvent)
    fire(frame('sess-1', UPDATE))

    expect(onEvent).toHaveBeenCalledTimes(1)
    expect(onEvent).toHaveBeenCalledWith(UPDATE)
  })

  it('does not deliver a frame addressed to another session', () => {
    const { client, fire } = fakeClient()
    const onEvent = vi.fn()

    startSessionEventsSubscription(client, 'sess-1', onEvent)
    fire(frame('sess-2', UPDATE))

    expect(onEvent).not.toHaveBeenCalled()
  })

  it('unregisters the handler and the trigger on cleanup', () => {
    const { client, offHandler, triggerUnregister } = fakeClient()

    const stop = startSessionEventsSubscription(client, 'sess-1', () => {})
    stop()

    expect(offHandler).toHaveBeenCalledTimes(1)
    expect(triggerUnregister).toHaveBeenCalledTimes(1)
  })
})

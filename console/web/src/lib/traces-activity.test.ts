import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import {
  extractTraceActivityIds,
  startTraceActivityFeed,
} from './traces-activity'

function fakeClient() {
  const triggers: Array<{
    type: string
    function_id: string
    config?: unknown
  }> = []
  const handlers = new Map<string, (p: unknown) => void>()
  const offHandlers = new Map<string, ReturnType<typeof vi.fn>>()
  const triggerUnregister = vi.fn()

  const on = vi.fn((fn: string, handler: (p: unknown) => void) => {
    handlers.set(fn, handler)
    const off = vi.fn()
    offHandlers.set(fn, off)
    return off
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
    addConnectionStateListener: vi.fn(() => vi.fn()),
    trigger: vi.fn(),
    dispose: vi.fn(async () => {}),
  } as unknown as IiiClient

  return {
    client,
    on,
    registerTrigger,
    triggers,
    triggerUnregister,
    offHandlers,
    fire: (fn: string, frame: unknown) => handlers.get(fn)?.(frame),
  }
}

describe('extractTraceActivityIds', () => {
  it('reads trace_ids out of the trigger payload, keeping strings only', () => {
    expect(
      extractTraceActivityIds({ trace_ids: ['a', 'b', 7, null, 'c'] }),
    ).toEqual(['a', 'b', 'c'])
  })

  it('returns [] for malformed payloads', () => {
    expect(extractTraceActivityIds(null)).toEqual([])
    expect(extractTraceActivityIds('nope')).toEqual([])
    expect(extractTraceActivityIds({})).toEqual([])
    expect(extractTraceActivityIds({ trace_ids: 'a' })).toEqual([])
  })
})

describe('startTraceActivityFeed', () => {
  it("registers the handler and a `type:'trace'` trigger", () => {
    const { client, on, triggers } = fakeClient()
    startTraceActivityFeed(client, () => {})

    expect(on).toHaveBeenCalledWith(
      'iii::console::trace_activity',
      expect.any(Function),
    )
    expect(triggers).toEqual([
      {
        type: 'trace',
        function_id: 'iii::console::trace_activity::console-test',
        config: {},
      },
    ])
  })

  it('delivers trace ids to onTraceIds (and ignores empty batches)', () => {
    const { client, fire } = fakeClient()
    const onTraceIds = vi.fn()
    startTraceActivityFeed(client, onTraceIds)

    fire('iii::console::trace_activity', { trace_ids: [] })
    expect(onTraceIds).not.toHaveBeenCalled()

    fire('iii::console::trace_activity', { trace_ids: ['t-1', 't-2'] })
    expect(onTraceIds).toHaveBeenCalledTimes(1)
    expect(onTraceIds).toHaveBeenCalledWith(['t-1', 't-2'])
  })

  it('unregisters the handler and trigger on cleanup', () => {
    const { client, offHandlers, triggerUnregister } = fakeClient()
    const stop = startTraceActivityFeed(client, () => {})
    stop()
    expect(
      offHandlers.get('iii::console::trace_activity'),
    ).toHaveBeenCalledTimes(1)
    expect(triggerUnregister).toHaveBeenCalledTimes(1)
  })
})

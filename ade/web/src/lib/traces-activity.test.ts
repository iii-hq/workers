import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import {
  extractTraceActivityIds,
  startTraceActivityFeed,
} from './traces-activity'

function fakeClient() {
  const observers: Array<{ source: unknown; target: string }> = []
  const handlers = new Map<string, (p: unknown) => void>()
  const offHandlers = new Map<string, ReturnType<typeof vi.fn>>()
  const disposeObserver = vi.fn()

  const on = vi.fn((fn: string, handler: (p: unknown) => void) => {
    handlers.set(fn, handler)
    const off = vi.fn()
    offHandlers.set(fn, off)
    return off
  })
  const observe = vi.fn((source: unknown, target: string) => {
    observers.push({ source, target })
    return disposeObserver
  })

  const client = {
    browserId: 'console-test',
    on,
    observe,
    addConnectionStateListener: vi.fn(() => vi.fn()),
    call: vi.fn(),
    dispose: vi.fn(async () => {}),
  } as unknown as IiiClient

  return {
    client,
    on,
    observe,
    observers,
    disposeObserver,
    offHandlers,
    fire: (fn: string, frame: unknown) => handlers.get(fn)?.(frame),
  }
}

describe('extractTraceActivityIds', () => {
  it('reads trace_ids out of the span event, keeping strings only', () => {
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
  it('registers the handler and observes observability::on-span', () => {
    const { client, on, observers } = fakeClient()
    startTraceActivityFeed(client, () => {})

    expect(on).toHaveBeenCalledWith(
      'iii::console::trace_activity',
      expect.any(Function),
    )
    expect(observers).toEqual([
      {
        source: 'observability::on-span',
        target: 'iii::console::trace_activity::console-test',
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

  it('drops the handler and disposes the observer on cleanup', () => {
    const { client, offHandlers, disposeObserver } = fakeClient()
    const stop = startTraceActivityFeed(client, () => {})
    stop()
    expect(
      offHandlers.get('iii::console::trace_activity'),
    ).toHaveBeenCalledTimes(1)
    expect(disposeObserver).toHaveBeenCalledTimes(1)
  })
})

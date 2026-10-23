import type { Host } from '@iii-dev/console-ui'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { isDeprecatedBinding, subscribeLiveBinding } from './live'
import type { LiveBinding, SurfaceRecord } from './types'

afterEach(() => vi.useRealTimers())

describe('A2UI bindings to worker-owned trigger types', () => {
  it('registers the owned trigger type and applies event payloads in event mode', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    const listener = vi.fn()
    const off = subscribeLiveBinding(mock.host, fixtureSurface(), {
      id: 'latest-order',
      trigger_type: 'orders::changed',
      config: { status: 'open' },
      target_path: '/latest',
      event_path: '/order',
    }, listener)

    expect(mock.registerTrigger).toHaveBeenCalledWith(expect.objectContaining({
      type: 'orders::changed',
      config: { status: 'open' },
    }))
    await vi.advanceTimersByTimeAsync(120)
    expect(mock.trigger).not.toHaveBeenCalled()

    mock.emit({ order: { id: 'o-1', status: 'open' }, revision: 4 })
    await vi.advanceTimersByTimeAsync(120)
    expect(mock.trigger).toHaveBeenCalledWith('a2ui::binding::apply', expect.objectContaining({
      binding_id: 'latest-order',
      value: { id: 'o-1', status: 'open' },
    }))
    expect(listener).toHaveBeenCalledWith('/latest', { id: 'o-1', status: 'open' }, 2)
    off()
  })

  it('reads through the provider query on mount and after each notification, coalesced', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    let count = 0
    mock.trigger.mockImplementation(async (functionId: string) => {
      if (functionId !== 'a2ui::binding::refresh') throw new Error(`unexpected ${functionId}`)
      count += 1
      return { revision: 10 + count, value: count, changed: true }
    })
    const listener = vi.fn()
    const binding: LiveBinding = {
      id: 'counter',
      trigger_type: 'demo::counter-changed',
      config: { counter: 'clicks' },
      target_path: '/count',
      query: { function_id: 'demo::counter::get', payload: { counter: 'clicks' }, result_path: '/value' },
    }
    const off = subscribeLiveBinding(mock.host, fixtureSurface(), binding, listener)
    expect(mock.registerTrigger).toHaveBeenCalledTimes(1)

    // Initial read happens after the trigger is registered.
    await vi.advanceTimersByTimeAsync(120)
    expect(mock.trigger).toHaveBeenCalledTimes(1)
    expect(mock.trigger).toHaveBeenLastCalledWith('a2ui::binding::refresh', {
      session_id: 'session-1',
      surface_id: 'surface-1',
      binding_id: 'counter',
    })
    expect(listener).toHaveBeenLastCalledWith('/count', 1, 11)

    // Notifications carry no data the page trusts; three of them coalesce into one read.
    mock.emit({ counter: 'clicks', revision: 2 })
    mock.emit({ counter: 'clicks', revision: 3 })
    mock.emit({ counter: 'clicks', revision: 3 })
    await vi.advanceTimersByTimeAsync(120)
    expect(mock.trigger).toHaveBeenCalledTimes(2)
    expect(listener).toHaveBeenLastCalledWith('/count', 2, 12)
    expect(mock.trigger.mock.calls.every(([id]) => id === 'a2ui::binding::refresh')).toBe(true)
    off()
  })

  it('unregisters the trigger and handler on unbind and ignores later events', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    const off = subscribeLiveBinding(mock.host, fixtureSurface(), {
      id: 'counter',
      trigger_type: 'demo::counter-changed',
      config: { counter: 'clicks' },
      target_path: '/count',
      query: { function_id: 'demo::counter::get' },
    }, vi.fn())
    off()
    expect(mock.unregisterTrigger).toHaveBeenCalledTimes(1)
    expect(mock.handlerCount()).toBe(0)
    mock.emit({ counter: 'clicks' })
    await vi.advanceTimersByTimeAsync(500)
    expect(mock.trigger).not.toHaveBeenCalled()
  })

  it('keeps legacy stream bindings working and reports them as deprecated', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    const legacy: LiveBinding = {
      id: 'events',
      trigger_type: 'stream',
      config: { stream_name: 'agent::events', group_id: 'session-1' },
      target_path: '/events',
    }
    expect(isDeprecatedBinding(legacy)).toBe(true)
    expect(isDeprecatedBinding({ ...legacy, trigger_type: 'orders::changed' })).toBe(false)
    const off = subscribeLiveBinding(mock.host, fixtureSurface(), legacy, vi.fn())
    expect(mock.registerTrigger).toHaveBeenCalledWith(expect.objectContaining({ type: 'stream' }))
    mock.emit({ data: 1 })
    await vi.advanceTimersByTimeAsync(120)
    expect(mock.trigger).toHaveBeenCalledWith('a2ui::binding::apply', expect.objectContaining({ binding_id: 'events' }))
    off()
  })
})

function fixtureSurface(): SurfaceRecord {
  return {
    session_id: 'session-1',
    surface_id: 'surface-1',
    protocol_version: 'v0.9.1',
    catalog_id: 'urn:iii:a2ui:console:v0.1',
    title: 'Surface',
    theme: null,
    send_data_model: false,
    components: [],
    data_model: {},
    revision: 1,
    created_at_ms: 1,
    updated_at_ms: 1,
    last_action: null,
    pinned: false,
    bindings: [],
    history: [],
  }
}

function mockHost() {
  const handlers = new Set<(payload: unknown) => void>()
  const trigger = vi.fn(async (_functionId: string, _payload?: unknown): Promise<unknown> => ({ revision: 2 }))
  const unregisterTrigger = vi.fn()
  const registerTrigger = vi.fn((_input: unknown) => () => unregisterTrigger())
  const on = vi.fn((_id: string, handler: (payload: unknown) => void) => {
    handlers.add(handler)
    return () => handlers.delete(handler)
  })
  const host = {
    iii: { browserId: 'browser-1', trigger, on, registerTrigger },
  } as unknown as Host
  return {
    host,
    trigger,
    registerTrigger,
    unregisterTrigger,
    handlerCount: () => handlers.size,
    emit: (payload: unknown) => {
      for (const handler of handlers) handler(payload)
    },
  }
}

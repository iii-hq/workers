import type { Host } from '@iii-dev/console-ui'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { isDeprecatedBinding, subscribeLiveBinding, subscribeSurfaceEvents } from './live'
import type { LiveBinding, SurfaceRecord } from './types'

afterEach(() => vi.useRealTimers())

describe("A2UI bindings to a worker's event functions", () => {
  it('observes the function with its filters and applies event payloads in event mode', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    const listener = vi.fn()
    const off = subscribeLiveBinding(mock.host, fixtureSurface(), {
      id: 'latest-order',
      function_id: 'orders::on-changed',
      filters: { 'input.status': { $eq: 'open' } },
      target_path: '/latest',
      event_path: '/order',
    }, listener)

    expect(mock.observe).toHaveBeenCalledWith(
      {
        function_id: 'orders::on-changed',
        filters: { 'input.status': { $eq: 'open' } },
        deliver: 'output',
      },
      expect.stringMatching(/^iii::a2ui-binding::.+::browser-1$/),
    )
    await vi.advanceTimersByTimeAsync(120)
    expect(mock.call).not.toHaveBeenCalled()

    mock.emit({ order: { id: 'o-1', status: 'open' }, revision: 4 })
    await vi.advanceTimersByTimeAsync(120)
    expect(mock.call).toHaveBeenCalledWith('a2ui::binding::apply', expect.objectContaining({
      binding_id: 'latest-order',
      value: { id: 'o-1', status: 'open' },
    }))
    expect(listener).toHaveBeenCalledWith('/latest', { id: 'o-1', status: 'open' }, 2)
    off()
  })

  it('passes args to a function that takes them', () => {
    const mock = mockHost()
    const off = subscribeLiveBinding(mock.host, fixtureSurface(), {
      id: 'counter',
      function_id: 'demo::on-counter-changed',
      args: { counter: 'clicks' },
      target_path: '/count',
    }, vi.fn())
    expect(mock.observe).toHaveBeenCalledWith(
      { function_id: 'demo::on-counter-changed', args: { counter: 'clicks' }, deliver: 'output' },
      expect.any(String),
    )
    off()
  })

  it('reads through the provider query on mount and after each notification, coalesced', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    let count = 0
    mock.call.mockImplementation(async (functionId: string) => {
      if (functionId !== 'a2ui::binding::refresh') throw new Error(`unexpected ${functionId}`)
      count += 1
      return { revision: 10 + count, value: count, changed: true }
    })
    const listener = vi.fn()
    const binding: LiveBinding = {
      id: 'counter',
      function_id: 'demo::on-counter-changed',
      args: { counter: 'clicks' },
      target_path: '/count',
      query: { function_id: 'demo::counter::get', payload: { counter: 'clicks' }, result_path: '/value' },
    }
    const off = subscribeLiveBinding(mock.host, fixtureSurface(), binding, listener)
    expect(mock.observe).toHaveBeenCalledTimes(1)

    // Initial read happens after the observer is created.
    await vi.advanceTimersByTimeAsync(120)
    expect(mock.call).toHaveBeenCalledTimes(1)
    expect(mock.call).toHaveBeenLastCalledWith('a2ui::binding::refresh', {
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
    expect(mock.call).toHaveBeenCalledTimes(2)
    expect(listener).toHaveBeenLastCalledWith('/count', 2, 12)
    expect(mock.call.mock.calls.every(([id]) => id === 'a2ui::binding::refresh')).toBe(true)
    off()
  })

  it('reads the state entry a state::on-changed event names', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    mock.call.mockImplementation(async (functionId: string) =>
      functionId === 'state::get' ? { open: 3 } : { revision: 5 },
    )
    const listener = vi.fn()
    const off = subscribeLiveBinding(mock.host, fixtureSurface(), {
      id: 'orders',
      function_id: 'state::on-changed',
      filters: { 'input.scope': { $eq: 'orders' }, 'input.key': { $eq: 'summary' } },
      target_path: '/orders',
    }, listener)
    mock.emit({ type: 'state', event_type: 'state:updated', scope: 'orders', key: 'summary' })
    await vi.advanceTimersByTimeAsync(120)
    expect(mock.call).toHaveBeenCalledWith('state::get', { scope: 'orders', key: 'summary' })
    expect(listener).toHaveBeenCalledWith('/orders', { open: 3 }, 5)
    off()
  })

  it('disposes the observer and handler on unbind and ignores later events', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    const off = subscribeLiveBinding(mock.host, fixtureSurface(), {
      id: 'counter',
      function_id: 'demo::on-counter-changed',
      args: { counter: 'clicks' },
      target_path: '/count',
      query: { function_id: 'demo::counter::get' },
    }, vi.fn())
    off()
    expect(mock.dispose).toHaveBeenCalledTimes(1)
    expect(mock.handlerCount()).toBe(0)
    mock.emit({ counter: 'clicks' })
    await vi.advanceTimersByTimeAsync(500)
    expect(mock.call).not.toHaveBeenCalled()
  })

  it('keeps legacy stream bindings working and reports them as deprecated', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    const legacy: LiveBinding = {
      id: 'events',
      function_id: 'stream::on-changed',
      filters: { 'input.stream_name': { $eq: 'agent::events' }, 'input.group_id': { $eq: 'session-1' } },
      target_path: '/events',
    }
    expect(isDeprecatedBinding(legacy)).toBe(true)
    expect(isDeprecatedBinding({ ...legacy, function_id: 'orders::on-changed' })).toBe(false)
    const off = subscribeLiveBinding(mock.host, fixtureSurface(), legacy, vi.fn())
    expect(mock.observe).toHaveBeenCalledWith(
      expect.objectContaining({ function_id: 'stream::on-changed' }),
      expect.any(String),
    )
    mock.emit({ data: 1 })
    await vi.advanceTimersByTimeAsync(120)
    expect(mock.call).toHaveBeenCalledWith('a2ui::binding::apply', expect.objectContaining({ binding_id: 'events' }))
    off()
  })
})

describe('A2UI surface events', () => {
  it("observes state::on-changed for the session's own a2ui entry", () => {
    const mock = mockHost()
    const off = subscribeSurfaceEvents(mock.host, 'session-1', vi.fn())
    expect(mock.observe).toHaveBeenCalledWith(
      {
        function_id: 'state::on-changed',
        filters: { 'input.scope': { $eq: 'a2ui' }, 'input.key': { $eq: 'session-1' } },
        deliver: 'output',
      },
      'iii::a2ui-ui::events::browser-1',
    )
    off()
    expect(mock.dispose).toHaveBeenCalledTimes(1)
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
  const call = vi.fn(async (_functionId: string, _payload?: unknown): Promise<unknown> => ({ revision: 2 }))
  const dispose = vi.fn()
  const observe = vi.fn((_source: unknown, _target: string) => () => dispose())
  const on = vi.fn((_id: string, handler: (payload: unknown) => void) => {
    handlers.add(handler)
    return () => handlers.delete(handler)
  })
  const host = {
    iii: { browserId: 'browser-1', call, on, observe },
  } as unknown as Host
  return {
    host,
    call,
    observe,
    dispose,
    handlerCount: () => handlers.size,
    emit: (payload: unknown) => {
      for (const handler of handlers) handler(payload)
    },
  }
}

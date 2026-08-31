import type { Host } from '@iii-dev/console-ui'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { getPath, setPath } from './bindings'
import { subscribeLiveBinding, subscribeSurfaceEvents } from './live'
import type { LiveBinding, SurfaceRecord } from './types'

afterEach(() => vi.useRealTimers())

describe('A2UI data bindings', () => {
  it('reads and immutably writes JSON Pointer paths', () => {
    const before = { profile: { name: 'Rohit' } }
    expect(getPath(before, '/profile/name')).toBe('Rohit')
    const after = setPath(before, '/profile/name', 'Ada')
    expect(getPath(after, '/profile/name')).toBe('Ada')
    expect(getPath(before, '/profile/name')).toBe('Rohit')
  })

  it('decodes escaped pointer segments', () => {
    expect(getPath({ 'a/b': { '~key': 7 } }, '/a~1b/~0key')).toBe(7)
  })

  it('updates array members without changing the array shape', () => {
    const before = { items: [{ status: 'pending' }] }
    const after = setPath(before, '/items/0/status', 'ready')
    expect(Array.isArray(getPath(after, '/items'))).toBe(true)
    expect(getPath(after, '/items/0/status')).toBe('ready')
    expect(getPath(before, '/items/0/status')).toBe('pending')
  })

  it('rejects inherited and prototype-mutating pointer segments', () => {
    const before = { safe: true }
    expect(setPath(before, '/__proto__/polluted', true)).toBe(before)
    expect(getPath(before, '/constructor/name')).toBeUndefined()
    expect(({} as { polluted?: boolean }).polluted).toBeUndefined()
  })
})

describe('A2UI live subscriptions', () => {
  it('shares one binding trigger and one durable update across mounted views', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    const surface = fixtureSurface()
    const binding: LiveBinding = {
      id: 'files',
      trigger_type: 'shell::changed',
      config: { path: '/workspace' },
      target_path: '/last_change',
    }
    const first = vi.fn()
    const second = vi.fn()
    const offFirst = subscribeLiveBinding(mock.host, surface, binding, first)
    const offSecond = subscribeLiveBinding(mock.host, surface, binding, second)
    expect(mock.registerTrigger).toHaveBeenCalledTimes(1)

    mock.emit({ path: '/workspace/file.ts' })
    await vi.advanceTimersByTimeAsync(120)
    expect(first).toHaveBeenCalledTimes(1)
    expect(second).toHaveBeenCalledTimes(1)
    expect(first).toHaveBeenCalledWith('/last_change', { path: '/workspace/file.ts' }, 2)
    expect(mock.trigger).toHaveBeenCalledTimes(1)
    expect(mock.trigger).toHaveBeenCalledWith(
      'a2ui::binding::apply',
      expect.objectContaining({ binding_id: 'files' }),
    )

    offFirst()
    expect(mock.unregisterTrigger).not.toHaveBeenCalled()
    offSecond()
    expect(mock.unregisterTrigger).toHaveBeenCalledTimes(1)
  })

  it('shares subscriptions across separately loaded Console bundles', async () => {
    vi.useFakeTimers()
    const firstBundle = await import('./live')
    vi.resetModules()
    const secondBundle = await import('./live')
    const mock = mockHost()
    const surface = fixtureSurface()
    const binding: LiveBinding = {
      id: 'cross-bundle',
      trigger_type: 'shell::changed',
      config: { path: '/workspace' },
      target_path: '/last_change',
    }
    const offFirst = firstBundle.subscribeLiveBinding(mock.host, surface, binding, vi.fn())
    const offSecond = secondBundle.subscribeLiveBinding(mock.host, surface, binding, vi.fn())

    expect(mock.registerTrigger).toHaveBeenCalledTimes(1)
    mock.emit({ path: '/workspace/file.ts' })
    await vi.advanceTimersByTimeAsync(120)
    expect(mock.trigger).toHaveBeenCalledTimes(1)

    offFirst()
    offSecond()
  })

  it('uses collision-resistant local function ids', () => {
    const mock = mockHost()
    const surface = fixtureSurface()
    const first = subscribeLiveBinding(mock.host, surface, {
      id: 'status.live',
      trigger_type: 'shell::changed',
      config: { path: '/workspace' },
      target_path: '/first',
    }, vi.fn())
    const second = subscribeLiveBinding(mock.host, surface, {
      id: 'status-live',
      trigger_type: 'shell::changed',
      config: { path: '/workspace' },
      target_path: '/second',
    }, vi.fn())

    expect(mock.on.mock.calls[0]?.[0]).not.toBe(mock.on.mock.calls[1]?.[0])
    first()
    second()
  })

  it('does not publish a live value when durable persistence fails', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    mock.trigger.mockRejectedValueOnce(new Error('state unavailable'))
    const listener = vi.fn()
    const off = subscribeLiveBinding(mock.host, fixtureSurface(), {
      id: 'durable-first',
      trigger_type: 'shell::changed',
      config: { path: '/workspace' },
      target_path: '/last_change',
    }, listener)

    mock.emit({ path: '/workspace/file.ts' })
    await vi.advanceTimersByTimeAsync(120)
    expect(listener).not.toHaveBeenCalled()
    off()
  })

  it('shares and coalesces exact-session surface refreshes', async () => {
    vi.useFakeTimers()
    const mock = mockHost()
    const first = vi.fn()
    const second = vi.fn()
    const offFirst = subscribeSurfaceEvents(mock.host, 'session-1', first)
    const offSecond = subscribeSurfaceEvents(mock.host, 'session-1', second)
    expect(mock.registerTrigger).toHaveBeenCalledTimes(1)

    const event = { type: 'state', scope: 'a2ui', key: 'session-1' }
    mock.emit(event)
    mock.emit(event)
    await vi.advanceTimersByTimeAsync(50)
    expect(first).toHaveBeenCalledTimes(1)
    expect(second).toHaveBeenCalledTimes(1)

    offFirst()
    offSecond()
    expect(mock.unregisterTrigger).toHaveBeenCalledTimes(1)
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
  const trigger = vi.fn(async () => ({ revision: 2 }))
  const registerTrigger = vi.fn(() => {
    return () => unregisterTrigger()
  })
  const unregisterTrigger = vi.fn()
  const on = vi.fn((_id: string, handler: (payload: unknown) => void) => {
    handlers.add(handler)
    return () => handlers.delete(handler)
  })
  const host = {
    iii: {
      browserId: 'browser-1',
      trigger,
      on,
      registerTrigger,
    },
  } as unknown as Host
  return {
    host,
    trigger,
    on,
    registerTrigger,
    unregisterTrigger,
    emit: (payload: unknown) => {
      for (const handler of handlers) handler(payload)
    },
  }
}

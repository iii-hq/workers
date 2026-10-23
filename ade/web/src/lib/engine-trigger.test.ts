import { describe, expect, it, vi } from 'vitest'
import { subscribeEngineTrigger, uniqueHandlerId } from './engine-trigger'

function fakeClient() {
  const handlers = new Map<string, (payload: unknown) => void>()
  const offHandler = vi.fn()
  const offTrigger = vi.fn()
  const registerTrigger = vi.fn(() => offTrigger)
  return {
    browserId: 'tab1',
    handlers,
    offHandler,
    offTrigger,
    registerTrigger,
    on: vi.fn((id: string, handler: (payload: unknown) => void) => {
      handlers.set(id, handler)
      return offHandler
    }),
  }
}

describe('subscribeEngineTrigger', () => {
  it('binds the trigger type to its own browser-local handler', async () => {
    const client = fakeClient()
    const seen: unknown[] = []
    const off = await subscribeEngineTrigger(
      'browser::chromium-install-progress',
      {},
      (payload) => seen.push(payload),
      { handler: 'iii::console::chromium', client: client as never },
    )
    const [[fnId]] = client.on.mock.calls
    expect(fnId).toMatch(/^iii::console::chromium::/)
    expect(client.registerTrigger).toHaveBeenCalledWith({
      type: 'browser::chromium-install-progress',
      function_id: `${fnId}::tab1`,
      config: {},
    })
    client.handlers.get(fnId)?.({ phase: 'done' })
    expect(seen).toEqual([{ phase: 'done' }])
    off()
    off()
    expect(client.offTrigger).toHaveBeenCalledTimes(1)
    expect(client.offHandler).toHaveBeenCalledTimes(1)
  })

  it('gives two waits on the same trigger their own handlers', async () => {
    const client = fakeClient()
    await subscribeEngineTrigger('compose-operation', {}, () => undefined, {
      client: client as never,
    })
    await subscribeEngineTrigger('compose-operation', {}, () => undefined, {
      client: client as never,
    })
    const ids = client.on.mock.calls.map(([id]) => id)
    expect(new Set(ids).size).toBe(2)
    expect(uniqueHandlerId('x')).not.toBe(uniqueHandlerId('x'))
  })

  it('drops the handler and rejects when the trigger cannot register', async () => {
    const client = fakeClient()
    client.registerTrigger.mockImplementation(() => {
      throw new Error('disposed')
    })
    await expect(
      subscribeEngineTrigger('compose-operation', {}, () => undefined, {
        client: client as never,
      }),
    ).rejects.toThrow('disposed')
    expect(client.offHandler).toHaveBeenCalledTimes(1)
  })
})

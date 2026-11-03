import { describe, expect, it, vi } from 'vitest'
import { subscribeObserved, uniqueHandlerId } from './observe-events'

function fakeClient() {
  const handlers = new Map<string, (payload: unknown) => void>()
  const offHandler = vi.fn()
  const offObserver = vi.fn()
  const observe = vi.fn((..._args: unknown[]) => offObserver)
  return {
    browserId: 'tab1',
    handlers,
    offHandler,
    offObserver,
    observe,
    on: vi.fn((id: string, handler: (payload: unknown) => void) => {
      handlers.set(id, handler)
      return offHandler
    }),
  }
}

describe('subscribeObserved', () => {
  it('observes the function with its own browser-local handler', async () => {
    const client = fakeClient()
    const seen: unknown[] = []
    const off = await subscribeObserved(
      'browser::on-chromium-install-progress',
      {},
      (payload) => seen.push(payload),
      { handler: 'iii::console::chromium', client: client as never },
    )
    const [[fnId]] = client.on.mock.calls
    expect(fnId).toMatch(/^iii::console::chromium::/)
    expect(client.observe).toHaveBeenCalledWith(
      { function_id: 'browser::on-chromium-install-progress' },
      `${fnId}::tab1`,
      undefined,
    )
    client.handlers.get(fnId)?.({ phase: 'done' })
    expect(seen).toEqual([{ phase: 'done' }])
    off()
    off()
    expect(client.offObserver).toHaveBeenCalledTimes(1)
    expect(client.offHandler).toHaveBeenCalledTimes(1)
  })

  it('passes args, filters, namespace and observer options through', async () => {
    const client = fakeClient()
    await subscribeObserved(
      'compose::on-operation-progress',
      {
        namespace: 'compose',
        filters: { 'input.operation_id': { $eq: 'op-1' } },
      },
      () => undefined,
      { client: client as never, observer: { expires_in_ms: 60_000 } },
    )
    const [[source, , options]] = client.observe.mock.calls
    expect(source).toEqual({
      function_id: 'compose::on-operation-progress',
      namespace: 'compose',
      filters: { 'input.operation_id': { $eq: 'op-1' } },
    })
    expect(options).toEqual({ expires_in_ms: 60_000 })
  })

  it('gives two waits on the same function their own handlers', async () => {
    const client = fakeClient()
    await subscribeObserved(
      'compose::on-operation-progress',
      {},
      () => undefined,
      { client: client as never },
    )
    await subscribeObserved(
      'compose::on-operation-progress',
      {},
      () => undefined,
      { client: client as never },
    )
    const ids = client.on.mock.calls.map(([id]) => id)
    expect(new Set(ids).size).toBe(2)
    expect(uniqueHandlerId('x')).not.toBe(uniqueHandlerId('x'))
  })

  it('drops the handler and rejects when the client cannot observe', async () => {
    const client = fakeClient()
    client.observe.mockImplementation(() => {
      throw new Error('disposed')
    })
    await expect(
      subscribeObserved('compose::on-operation-progress', {}, () => undefined, {
        client: client as never,
      }),
    ).rejects.toThrow('disposed')
    expect(client.offHandler).toHaveBeenCalledTimes(1)
  })
})

import { describe, expect, it, vi } from 'vitest'
import { createEngineTraceSource, type EngineClient } from './engine'

function fakeClient() {
  const handlers = new Map<string, (payload: unknown) => void>()
  const connection: Array<(state: string) => void> = []
  const off = vi.fn()
  const offObserver = vi.fn()
  const offConnection = vi.fn()
  const client: EngineClient = {
    browserId: 'b1',
    call: vi.fn(),
    on: (functionId, handler) => {
      handlers.set(functionId, handler)
      return off
    },
    observe: () => offObserver,
    addConnectionStateListener: (listener) => {
      connection.push(listener)
      return offConnection
    },
  }
  const emit = (traceIds: string[]) =>
    handlers.get('iii::console::trace_activity')?.({ trace_ids: traceIds })
  return { client, emit, connection, off, offObserver, offConnection }
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('createEngineTraceSource', () => {
  it('delivers activity and resyncs on connect only, then stops', async () => {
    const fake = fakeClient()
    const source = createEngineTraceSource(() => fake.client)
    const onTraceIds = vi.fn()
    const onResync = vi.fn()
    const stop = source.watch?.(onTraceIds, onResync)
    await flush()

    fake.emit(['t1'])
    expect(onTraceIds).toHaveBeenCalledWith(['t1'])
    for (const listener of fake.connection) listener('disconnected')
    expect(onResync).not.toHaveBeenCalled()
    for (const listener of fake.connection) listener('connected')
    expect(onResync).toHaveBeenCalledTimes(1)

    stop?.()
    expect(fake.off).toHaveBeenCalled()
    expect(fake.offObserver).toHaveBeenCalled()
    expect(fake.offConnection).toHaveBeenCalled()
  })

  it('subscribes nothing when stopped before the client resolves', async () => {
    const fake = fakeClient()
    let resolve: (client: EngineClient) => void = () => {}
    const source = createEngineTraceSource(
      () => new Promise<EngineClient>((r) => (resolve = r)),
    )
    const stop = source.watch?.(vi.fn(), vi.fn())
    stop?.()
    resolve(fake.client)
    await flush()
    expect(fake.connection).toHaveLength(0)
  })

  it('reads fail as errors, and hidden functions as none, without a client', async () => {
    const source = createEngineTraceSource(() =>
      Promise.reject('socket closed'),
    )
    await expect(source.listTraces()).rejects.toBeInstanceOf(Error)
    await expect(source.hiddenFunctionIds?.()).resolves.toEqual(new Set())
    // A watch without a client has nothing to stop, and throws nothing.
    source.watch?.(vi.fn())?.()
  })
})

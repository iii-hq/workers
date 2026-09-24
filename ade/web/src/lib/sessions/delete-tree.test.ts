import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import {
  DELETE_TREE_WAIT_MS,
  deleteSessionTree,
  type SessionTreeDeletionSnapshot,
} from './delete-tree'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((done, fail) => {
    resolve = done
    reject = fail
  })
  return { promise, resolve, reject }
}

const snapshot = (
  status: SessionTreeDeletionSnapshot['status'] = 'deleting',
  operation_id = 'op-new',
  attempt = 1,
): SessionTreeDeletionSnapshot => ({
  operation_id,
  attempt,
  session_id: 'child2',
  status,
  deleted_session_ids: status === 'completed' ? ['child2', 'grandchild1'] : [],
})

let emit: (event: unknown) => void
let client: IiiClient
let accepted: ReturnType<typeof deferred<SessionTreeDeletionSnapshot>>
const offHandler = vi.fn()
const offTrigger = vi.fn()
const order: string[] = []

beforeEach(() => {
  vi.clearAllMocks()
  vi.useFakeTimers()
  order.length = 0
  accepted = deferred<SessionTreeDeletionSnapshot>()
  client = {
    browserId: 'test-browser',
    on: vi.fn((_id, handler) => {
      order.push('handler')
      emit = handler
      return offHandler
    }),
    registerTrigger: vi.fn(() => {
      order.push('subscription')
      return offTrigger
    }),
    trigger: vi.fn((id) => {
      order.push(id)
      return id === 'harness::delete-session-tree'
        ? accepted.promise
        : Promise.resolve(null)
    }),
  } as unknown as IiiClient
  vi.mocked(getIiiClient).mockResolvedValue(client)
})

afterEach(() => {
  vi.useRealTimers()
})

async function start(options?: Parameters<typeof deleteSessionTree>[1]) {
  const result = deleteSessionTree('child2', options)
  const outcome = result.then(
    (value) => ({ value }),
    (error: Error) => ({ error }),
  )
  await Promise.resolve()
  return { result, outcome }
}

function cleaned() {
  expect(offTrigger).toHaveBeenCalledTimes(1)
  expect(offHandler).toHaveBeenCalledTimes(1)
  expect(vi.getTimerCount()).toBe(0)
}

describe('deleteSessionTree', () => {
  it('subscribes before one selected-id command, reads once and waits without polling', async () => {
    const { result } = await start()
    expect(order).toEqual([
      'handler',
      'subscription',
      'harness::delete-session-tree',
    ])
    expect(client.registerTrigger).toHaveBeenCalledWith({
      type: 'harness::session-tree-deletion',
      function_id: expect.stringMatching(/::test-browser$/),
      config: { session_id: 'child2' },
    })
    expect(client.trigger).toHaveBeenCalledWith(
      'harness::delete-session-tree',
      { session_id: 'child2' },
    )
    accepted.resolve(snapshot())
    await vi.advanceTimersByTimeAsync(60_000)
    expect(client.trigger).toHaveBeenCalledTimes(2)
    expect(client.trigger).toHaveBeenLastCalledWith(
      'harness::delete-session-tree-status',
      { operation_id: 'op-new' },
    )
    expect(offHandler).not.toHaveBeenCalled()
    emit(snapshot('completed', 'old-operation'))
    emit({ ...snapshot('completed'), session_id: 'parent' })
    emit({ status: 'completed' })
    expect(offHandler).not.toHaveBeenCalled()
    emit(snapshot('completed'))
    await expect(result).resolves.toEqual(snapshot('completed'))
    cleaned()
  })

  it('buffers a terminal event before acceptance and ignores old operations', async () => {
    const { result } = await start()
    emit(snapshot('failed', 'old-operation'))
    emit(snapshot('completed'))
    expect(offHandler).not.toHaveBeenCalled()
    accepted.resolve(snapshot())
    await expect(result).resolves.toEqual(snapshot('completed'))
    expect(client.trigger).toHaveBeenCalledTimes(2)
    cleaned()
  })

  it('ignores a previous attempt failure before and after retry acceptance', async () => {
    const { result } = await start()
    emit(snapshot('failed', 'op-new', 1))
    vi.mocked(client.trigger).mockResolvedValueOnce(
      snapshot('failed', 'op-new', 1),
    )
    accepted.resolve(snapshot('deleting', 'op-new', 2))
    await vi.advanceTimersByTimeAsync(1)
    emit(snapshot('failed', 'op-new', 1))
    expect(offHandler).not.toHaveBeenCalled()
    emit(snapshot('completed', 'op-new', 2))
    await expect(result).resolves.toEqual(snapshot('completed', 'op-new', 2))
    cleaned()
  })

  it('does not overwrite an early current-attempt completion with an older failure', async () => {
    const { result } = await start()
    emit(snapshot('completed', 'op-new', 2))
    emit(snapshot('failed', 'op-new', 1))
    accepted.resolve(snapshot('deleting', 'op-new', 2))
    await expect(result).resolves.toEqual(snapshot('completed', 'op-new', 2))
    cleaned()
  })

  it.each([undefined, 0, -1, 1.5])(
    'rejects invalid attempt %s without claiming completion',
    async (attempt) => {
      const { result } = await start()
      accepted.resolve({
        ...snapshot('completed'),
        attempt,
      } as SessionTreeDeletionSnapshot)
      await expect(result).rejects.toThrow('Invalid deletion response')
      cleaned()
    },
  )

  it('recovers a missed terminal event with the single catch-up snapshot', async () => {
    const { result } = await start()
    vi.mocked(client.trigger).mockResolvedValueOnce(snapshot('completed'))
    accepted.resolve(snapshot())
    await expect(result).resolves.toEqual(snapshot('completed'))
    cleaned()
  })

  it('does not regress a terminal event when a stale catch-up read arrives', async () => {
    const recovery = deferred<SessionTreeDeletionSnapshot>()
    const { result } = await start()
    vi.mocked(client.trigger).mockReturnValueOnce(recovery.promise)
    accepted.resolve(snapshot())
    await Promise.resolve()
    emit(snapshot('completed'))
    await expect(result).resolves.toEqual(snapshot('completed'))
    recovery.resolve(snapshot())
    await Promise.resolve()
    cleaned()
  })

  it('accepts a terminal command response while still making one recovery read', async () => {
    const { result } = await start()
    accepted.resolve(snapshot('completed'))
    await expect(result).resolves.toEqual(snapshot('completed'))
    expect(client.trigger).toHaveBeenCalledTimes(2)
    cleaned()
  })

  it('keeps waiting for the correlated operation after a mismatched catch-up', async () => {
    const { result } = await start()
    vi.mocked(client.trigger).mockResolvedValueOnce(
      snapshot('completed', 'old'),
    )
    accepted.resolve(snapshot())
    await vi.advanceTimersByTimeAsync(1)
    expect(offHandler).not.toHaveBeenCalled()
    emit(snapshot('completed'))
    await expect(result).resolves.toEqual(snapshot('completed'))
    cleaned()
  })

  it('reports backend failure and retries the same session id idempotently', async () => {
    const first = await start()
    accepted.resolve(snapshot())
    await Promise.resolve()
    emit({ ...snapshot('failed'), error: 'Could not stop descendant' })
    await expect(first.result).rejects.toThrow('Could not stop descendant')
    cleaned()
    accepted = deferred<SessionTreeDeletionSnapshot>()
    const second = await start()
    accepted.resolve(snapshot('completed'))
    await expect(second.result).resolves.toEqual(snapshot('completed'))
    const commands = vi
      .mocked(client.trigger)
      .mock.calls.filter(([id]) => id === 'harness::delete-session-tree')
    expect(commands.map(([, payload]) => payload)).toEqual([
      { session_id: 'child2' },
      { session_id: 'child2' },
    ])
  })

  it('fails closed on an unavailable backend, with no delete or cancellation fallback', async () => {
    const { result } = await start()
    accepted.reject(new Error('function_not_found'))
    await expect(result).rejects.toThrow('function_not_found')
    expect(client.trigger).toHaveBeenCalledTimes(1)
    cleaned()
  })

  it('cleans up a failed catch-up read', async () => {
    const { result } = await start()
    vi.mocked(client.trigger).mockRejectedValueOnce(
      new Error('read unavailable'),
    )
    accepted.resolve(snapshot())
    await expect(result).rejects.toThrow('read unavailable')
    cleaned()
  })

  it('cleans the local handler when binding fails, without sending a command', async () => {
    vi.mocked(client.registerTrigger).mockImplementationOnce(() => {
      throw new Error('trigger unavailable')
    })
    const { result } = await start()
    await expect(result).rejects.toThrow('trigger unavailable')
    expect(offHandler).toHaveBeenCalledTimes(1)
    expect(client.trigger).not.toHaveBeenCalled()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('bounds the client wait without claiming the backend stopped', async () => {
    const { result } = await start()
    accepted.resolve(snapshot())
    await vi.advanceTimersByTimeAsync(DELETE_TREE_WAIT_MS)
    await expect(result).rejects.toThrow('backend may still be running')
    expect(client.trigger).toHaveBeenCalledTimes(2)
    cleaned()
  })

  it('unmount abort cleans listeners without sending backend cancellation', async () => {
    const controller = new AbortController()
    const { result } = await start({ signal: controller.signal })
    controller.abort()
    await expect(result).rejects.toThrow(
      'Backend deletion may still be running',
    )
    accepted.resolve(snapshot())
    await Promise.resolve()
    expect(client.trigger).toHaveBeenCalledTimes(1)
    cleaned()
  })

  it('does not start after unmount while the SDK is still loading', async () => {
    const loading = deferred<IiiClient>()
    vi.mocked(getIiiClient).mockReturnValueOnce(loading.promise)
    const controller = new AbortController()
    const { result } = await start({ signal: controller.signal })
    controller.abort()
    loading.resolve(client)
    await expect(result).rejects.toThrow('Stopped waiting')
    expect(client.trigger).not.toHaveBeenCalled()
    expect(client.on).not.toHaveBeenCalled()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('rejects invalid acceptance instead of assuming completion', async () => {
    const { result } = await start()
    accepted.resolve({ status: 'completed' } as SessionTreeDeletionSnapshot)
    await expect(result).rejects.toThrow('Invalid deletion response')
    cleaned()
  })
})

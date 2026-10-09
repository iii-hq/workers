import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import {
  canForceDelete,
  DELETE_TREE_WAIT_MS,
  deleteSessionTree,
  deletionCommandErrorMessage,
  deletionFailureMessage,
  deletionOutcomes,
  hasDeletionOutcomeCounts,
  SessionTreeDeletionError,
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
    addConnectionStateListener: vi.fn(() => vi.fn()),
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
  it('preserves typed failure diagnostics and sends only the explicit force identity', async () => {
    const { outcome } = await start({
      force: { operation_id: 'op-new', attempt: 1 },
    })
    expect(client.trigger).toHaveBeenCalledWith(
      'harness::delete-session-tree',
      {
        session_id: 'child2',
        mode: 'force',
        operation_id: 'op-new',
        attempt: 1,
      },
    )
    const failed = {
      ...snapshot('failed', 'op-new', 2),
      mode: 'normal' as const,
      failure_code: 'blocked' as const,
      force_eligible: true,
      blockers: [
        {
          kind: 'unknown_completion' as const,
          session_id: 'grandchild1',
          function_id: 'browser::fetch',
        },
      ],
    }
    accepted.resolve(failed)
    const result = await outcome
    expect('error' in result && result.error).toBeInstanceOf(
      SessionTreeDeletionError,
    )
    if ('error' in result)
      expect((result.error as SessionTreeDeletionError).snapshot).toEqual(
        failed,
      )
    expect(canForceDelete(failed)).toBe(true)
    expect(canForceDelete({ ...failed, failure_code: 'failed' })).toBe(false)
    cleaned()
  })

  it('rejoins terminal durable state on reconnect without issuing another command', async () => {
    const { result } = await start()
    accepted.resolve(snapshot())
    await vi.advanceTimersByTimeAsync(1)
    vi.mocked(client.trigger).mockResolvedValueOnce(snapshot('completed'))
    const reconnect = vi.mocked(client.addConnectionStateListener).mock
      .calls[0][0]
    reconnect('connected')
    await expect(result).resolves.toEqual(snapshot('completed'))
    expect(
      vi
        .mocked(client.trigger)
        .mock.calls.filter(([id]) => id === 'harness::delete-session-tree'),
    ).toHaveLength(1)
    cleaned()
  })

  it('reconnects into a newer authoritative attempt and ignores earlier events', async () => {
    const { result } = await start()
    accepted.resolve(snapshot())
    await vi.advanceTimersByTimeAsync(1)
    vi.mocked(client.trigger).mockResolvedValueOnce(
      snapshot('deleting', 'op-new', 2),
    )
    vi.mocked(client.addConnectionStateListener).mock.calls[0][0]('connected')
    await vi.advanceTimersByTimeAsync(1)
    emit(snapshot('completed', 'op-new', 1))
    expect(offHandler).not.toHaveBeenCalled()
    emit(snapshot('completed', 'op-new', 2))
    await expect(result).resolves.toEqual(snapshot('completed', 'op-new', 2))
    cleaned()
  })

  it('refreshes an eligible failure event before exposing force', async () => {
    const { outcome } = await start()
    accepted.resolve(snapshot())
    await vi.advanceTimersByTimeAsync(1)
    const failed: SessionTreeDeletionSnapshot = {
      ...snapshot('failed'),
      mode: 'normal',
      failure_code: 'blocked',
      force_eligible: true,
      blockers: [{ kind: 'unknown_completion', session_id: 'child2' }],
    }
    vi.mocked(client.trigger).mockResolvedValueOnce({
      ...failed,
      force_eligible: false,
      blockers: [],
    })
    emit(failed)
    const result = await outcome
    expect('error' in result && result.error).toBeInstanceOf(
      SessionTreeDeletionError,
    )
    if ('error' in result)
      expect(
        canForceDelete((result.error as SessionTreeDeletionError).snapshot),
      ).toBe(false)
    expect(client.trigger).toHaveBeenLastCalledWith(
      'harness::delete-session-tree-status',
      { operation_id: 'op-new' },
    )
    cleaned()
  })

  it('keeps the typed event but disables force if eligibility refresh fails', async () => {
    const { outcome } = await start()
    accepted.resolve(snapshot())
    await vi.advanceTimersByTimeAsync(1)
    const failed: SessionTreeDeletionSnapshot = {
      ...snapshot('failed'),
      mode: 'normal',
      failure_code: 'blocked',
      force_eligible: true,
      blockers: [{ kind: 'active_processing', session_id: 'child2' }],
    }
    vi.mocked(client.trigger).mockRejectedValueOnce(new Error('offline'))
    emit(failed)
    const result = await outcome
    expect('error' in result && result.error).toBeInstanceOf(
      SessionTreeDeletionError,
    )
    if ('error' in result)
      expect((result.error as SessionTreeDeletionError).snapshot).toEqual({
        ...failed,
        force_eligible: false,
      })
    cleaned()
  })

  it('does not treat an empty failed result as enumeration proof', () => {
    expect(
      hasDeletionOutcomeCounts({
        ...snapshot('failed'),
        remaining_session_ids: [],
      }),
    ).toBe(false)
    expect(
      hasDeletionOutcomeCounts({
        ...snapshot('failed'),
        remaining_session_ids: ['child2'],
      }),
    ).toBe(true)
    expect(
      hasDeletionOutcomeCounts({
        ...snapshot('completed'),
        remaining_session_ids: [],
      }),
    ).toBe(true)
    expect(hasDeletionOutcomeCounts(snapshot('failed'))).toBe(false)
  })

  it('recovers eligible terminal status without a second failing refresh or stale event winning', async () => {
    const failed: SessionTreeDeletionSnapshot = {
      ...snapshot('failed'),
      mode: 'normal',
      failure_code: 'blocked',
      force_eligible: true,
      blockers: [{ kind: 'unknown_completion', session_id: 'child2' }],
    }
    vi.mocked(client.trigger)
      .mockImplementationOnce(async () => {
        emit({ ...failed, force_eligible: false })
        return failed
      })
      .mockRejectedValueOnce(new Error('redundant read failed'))
    const { outcome } = await start({ recover: true })
    const result = await outcome
    expect(
      'error' in result &&
        canForceDelete((result.error as SessionTreeDeletionError).snapshot),
    ).toBe(true)
    expect(client.trigger).toHaveBeenCalledTimes(1)
    cleaned()
  })

  it('reviews exact server operation read-only and never retries an absent operation', async () => {
    const { outcome } = await start({ reviewOperationId: 'server-owner' })
    expect('error' in (await outcome)).toBe(true)
    expect(client.trigger).toHaveBeenCalledExactlyOnceWith(
      'harness::delete-session-tree-status',
      { operation_id: 'server-owner' },
    )
    cleaned()
  })

  it('recovers an existing failed operation without issuing a new attempt', async () => {
    const failed = snapshot('failed')
    vi.mocked(client.trigger).mockResolvedValue(failed)
    const { outcome } = await start({ recover: true })
    expect('error' in (await outcome)).toBe(true)
    expect(
      vi
        .mocked(client.trigger)
        .mock.calls.every(
          ([id]) => id === 'harness::delete-session-tree-status',
        ),
    ).toBe(true)
    cleaned()
  })
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

  it('keeps waiting for the terminal event when the catch-up read fails', async () => {
    const { result } = await start()
    vi.mocked(client.trigger).mockRejectedValueOnce(
      new Error('read unavailable'),
    )
    accepted.resolve(snapshot())
    await vi.advanceTimersByTimeAsync(0)
    expect(offHandler).not.toHaveBeenCalled()
    emit(snapshot('completed'))
    await expect(result).resolves.toMatchObject({ status: 'completed' })
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

describe('deletion outcome reporting', () => {
  const partial: SessionTreeDeletionSnapshot = {
    ...snapshot('failed'),
    mode: 'force',
    failure_code: 'failed',
    deleted_session_ids: ['grandchild1'],
    remaining_session_ids: ['child2'],
    unconfirmed_session_ids: ['child2'],
    error: 'Old generic error: data retained.',
  }

  it('separates confirmed deletes, not deleted sessions and unconfirmed erase outcomes', () => {
    expect(deletionOutcomes(partial)).toEqual({
      notDeleted: [],
      unconfirmed: ['child2'],
    })
    expect(
      deletionOutcomes({ ...partial, unconfirmed_session_ids: [] }),
    ).toEqual({ notDeleted: ['child2'], unconfirmed: [] })
    expect(deletionFailureMessage(partial)).not.toMatch(/retained/i)
    expect(
      canForceDelete({
        ...partial,
        mode: 'normal',
        failure_code: 'blocked',
        force_eligible: true,
        blockers: [{ kind: 'active_processing', session_id: 'child2' }],
      }),
    ).toBe(false)
  })

  it('only calls known deterministic Force rejections unchanged', () => {
    for (const code of ['invalid_request', 'harness/invalid_request']) {
      const error = Object.assign(new Error('stale or topology changed'), {
        code,
      })
      expect(deletionCommandErrorMessage(error, true)).toContain(
        'Force delete was rejected; nothing changed.',
      )
      expect(deletionCommandErrorMessage(error, false)).toBeNull()
    }
    expect(
      deletionCommandErrorMessage(
        new Error('invalid_request in network text'),
        true,
      ),
    ).toBeNull()
    expect(
      deletionCommandErrorMessage(
        Object.assign(new Error('timeout'), { code: 'timeout' }),
        true,
      ),
    ).toBeNull()
  })

  it('requires structured initial-block proof before claiming retention', () => {
    const initial = {
      ...partial,
      mode: 'normal' as const,
      failure_code: 'blocked' as const,
      deleted_session_ids: [],
      unconfirmed_session_ids: [],
      data_retained: true,
    }
    expect(deletionFailureMessage(initial)).toContain(
      'No conversations were deleted; data retained.',
    )
    expect(deletionFailureMessage(initial)).not.toMatch(/pending/i)
    for (const value of [
      { ...initial, data_retained: undefined },
      { ...initial, data_retained: false },
      { ...initial, unconfirmed_session_ids: undefined },
      { ...initial, unconfirmed_session_ids: ['child2'] },
      { ...initial, deleted_session_ids: ['grandchild1'] },
    ])
      expect(deletionFailureMessage(value)).not.toMatch(/retained/i)
  })

  it('rejects invalid retention proof types on the wire', async () => {
    const { outcome } = await start()
    accepted.resolve({
      ...partial,
      data_retained: 'true',
    } as unknown as SessionTreeDeletionSnapshot)
    const result = await outcome
    expect('error' in result && result.error.message).toContain(
      'Completion could not be confirmed',
    )
    cleaned()
  })

  it('treats legacy generic remaining ids as unconfirmed, not retained', () => {
    expect(
      deletionOutcomes({ ...partial, unconfirmed_session_ids: undefined }),
    ).toEqual({ notDeleted: [], unconfirmed: ['child2'] })
  })

  it('preserves the additive field when recovering a failed operation without retrying', async () => {
    vi.mocked(client.trigger).mockResolvedValue(partial)
    const { outcome } = await start({ recover: true })
    const result = await outcome
    expect('error' in result && result.error).toBeInstanceOf(
      SessionTreeDeletionError,
    )
    if ('error' in result)
      expect((result.error as SessionTreeDeletionError).snapshot).toEqual(
        partial,
      )
    expect(
      vi
        .mocked(client.trigger)
        .mock.calls.every(
          ([id]) => id === 'harness::delete-session-tree-status',
        ),
    ).toBe(true)
    cleaned()
  })

  it('rejects unconfirmed ids outside remaining instead of exposing an inconsistent snapshot', async () => {
    const { outcome } = await start()
    accepted.resolve({ ...partial, unconfirmed_session_ids: ['foreign'] })
    const result = await outcome
    expect('error' in result && result.error.message).toContain(
      'Completion could not be confirmed',
    )
    cleaned()
  })

  it('does not reinterpret failed catch-up as retention or backend completion', async () => {
    const { outcome } = await start()
    vi.mocked(client.trigger).mockRejectedValueOnce(
      new Error('refresh offline'),
    )
    accepted.resolve(snapshot())
    await vi.advanceTimersByTimeAsync(DELETE_TREE_WAIT_MS)
    const result = await outcome
    expect('error' in result && result.error.message).toContain(
      'backend may still be running',
    )
    cleaned()
  })
})

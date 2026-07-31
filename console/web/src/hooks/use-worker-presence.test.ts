import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import { readWorkerPresence } from './use-worker-presence'

function fakeClient(
  values: Record<string, unknown | (() => never)>,
): IiiClient {
  const trigger = vi.fn(async (functionId: string) => {
    const value = values[functionId]
    if (typeof value === 'function') return value()
    return value
  })
  return { trigger } as unknown as IiiClient
}

const connectedEngine = {
  workers: [{ name: 'harness', status: 'connected' }],
}

describe('readWorkerPresence', () => {
  it('requires an engine connection before reporting the worker present', async () => {
    const client = fakeClient({
      'engine::workers::list': connectedEngine,
      'worker::status': {
        installed: true,
        running: true,
        stderr_tail: [],
        stdout_tail: [],
      },
    })

    await expect(readWorkerPresence(client, 'harness')).resolves.toEqual({
      present: true,
      state: 'connected',
      detail: null,
    })
  })

  it('reports a running manager process that has not registered as starting', async () => {
    const client = fakeClient({
      'engine::workers::list': { workers: [] },
      'worker::status': {
        installed: true,
        running: true,
        stderr_tail: [],
        stdout_tail: ['booting'],
      },
    })

    await expect(readWorkerPresence(client, 'harness')).resolves.toMatchObject({
      present: false,
      state: 'starting',
    })
  })

  it('surfaces a stopped worker and prefers stderr diagnostics', async () => {
    const client = fakeClient({
      'engine::workers::list': { workers: [] },
      'worker::status': {
        installed: true,
        running: false,
        stderr_tail: ['boot failed'],
        stdout_tail: ['last normal line'],
      },
    })

    await expect(readWorkerPresence(client, 'harness')).resolves.toEqual({
      present: false,
      state: 'stopped',
      detail: 'boot failed',
    })
  })

  it('keeps an explicitly absent worker distinct from an unreadable backend', async () => {
    const absent = fakeClient({
      'engine::workers::list': { workers: [] },
      'worker::status': {
        installed: false,
        running: false,
        stderr_tail: [],
        stdout_tail: [],
      },
    })
    await expect(readWorkerPresence(absent, 'harness')).resolves.toMatchObject({
      state: 'absent',
    })

    const unreadable = fakeClient({
      'engine::workers::list': () => {
        throw new Error('engine offline')
      },
      'worker::status': () => {
        throw new Error('manager offline')
      },
    })
    await expect(
      readWorkerPresence(unreadable, 'harness'),
    ).resolves.toMatchObject({
      present: false,
      state: 'unknown',
    })

    const managerUnavailable = fakeClient({
      'engine::workers::list': { workers: [] },
      'worker::status': () => {
        throw new Error('worker manager restarting')
      },
    })
    await expect(
      readWorkerPresence(managerUnavailable, 'harness'),
    ).resolves.toMatchObject({
      state: 'unknown',
      detail: 'could not read worker-manager presence',
    })
  })
})

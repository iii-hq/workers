import { describe, expect, it, vi } from 'vitest'
import type {
  IIIConnectionState,
  IiiClient,
  RegisterTriggerInput,
} from '@/lib/iii-client'
import {
  createWorkerPresenceWatcher,
  type WorkerPresenceWatcher,
} from './use-worker-presence'

interface WorkerRow {
  id: string
  name: string
}

function fakeClient(initialWorkers: WorkerRow[] = []) {
  let workers = initialWorkers
  const handlers = new Map<string, (payload: unknown) => void | Promise<void>>()
  const connectionHandlers = new Set<(state: IIIConnectionState) => void>()
  const triggerRegistrations: RegisterTriggerInput[] = []
  const offHandler = vi.fn()
  const offTrigger = vi.fn()
  const offConnection = vi.fn()

  const client: IiiClient = {
    browserId: 'presence-browser',
    trigger: vi.fn(async (functionId: string) => {
      if (functionId !== 'engine::workers::list') {
        throw new Error(`unexpected function: ${functionId}`)
      }
      return { workers }
    }) as IiiClient['trigger'],
    on: vi.fn((functionId, handler) => {
      handlers.set(functionId, handler)
      return () => {
        handlers.delete(functionId)
        offHandler()
      }
    }),
    registerTrigger: vi.fn((input) => {
      triggerRegistrations.push(input)
      return offTrigger
    }),
    addConnectionStateListener: vi.fn((handler) => {
      connectionHandlers.add(handler)
      return () => {
        connectionHandlers.delete(handler)
        offConnection()
      }
    }),
    dispose: vi.fn(async () => {}),
  }

  return {
    client,
    handlers,
    triggerRegistrations,
    offHandler,
    offTrigger,
    offConnection,
    setWorkers(next: WorkerRow[]) {
      workers = next
    },
    connect() {
      for (const handler of connectionHandlers) {
        handler('connected')
      }
    },
  }
}

function watcher(
  client: IiiClient,
  onChange: (state: {
    present: boolean
    loading: boolean
    revision: number
  }) => void,
): WorkerPresenceWatcher {
  return createWorkerPresenceWatcher({
    client,
    workerName: 'llm-router',
    localFnId: 'console::llm-router-watch::presence::test',
    initial: { present: false, loading: true, revision: 0 },
    onChange,
  })
}

async function settle(): Promise<void> {
  await Promise.resolve()
  await Promise.resolve()
}

describe('worker presence watcher', () => {
  it('re-probes while absent when the browser reconnects', async () => {
    const fake = fakeClient()
    const states: Array<{
      present: boolean
      loading: boolean
      revision: number
    }> = []
    const presence = watcher(fake.client, (state) => states.push(state))

    await expect(presence.refresh()).resolves.toBe(false)
    expect(states.at(-1)).toEqual({
      present: false,
      loading: false,
      revision: 0,
    })

    fake.setWorkers([{ id: 'router-b', name: 'llm-router' }])
    fake.connect()
    await settle()

    expect(states.at(-1)).toEqual({
      present: true,
      loading: false,
      revision: 1,
    })
  })

  it('uses engine worker catalogue ticks to detect a present-to-present restart', async () => {
    const fake = fakeClient([{ id: 'router-a', name: 'llm-router' }])
    const states: Array<{
      present: boolean
      loading: boolean
      revision: number
    }> = []
    const presence = watcher(fake.client, (state) => states.push(state))

    await presence.refresh()
    expect(states.at(-1)?.revision).toBe(1)
    expect(fake.triggerRegistrations).toEqual([
      {
        type: 'engine::workers-available',
        function_id:
          'console::llm-router-watch::presence::test::presence-browser',
        config: {},
      },
    ])

    fake.setWorkers([{ id: 'router-b', name: 'llm-router' }])
    await fake.handlers.get('console::llm-router-watch::presence::test')?.({})
    await settle()

    expect(states.at(-1)).toEqual({
      present: true,
      loading: false,
      revision: 2,
    })
  })

  it('drops an older presence response after a newer probe wins', async () => {
    let resolveFirst: ((value: { workers: WorkerRow[] }) => void) | undefined
    const firstResponse = new Promise<{ workers: WorkerRow[] }>((resolve) => {
      resolveFirst = resolve
    })
    const fake = fakeClient()
    vi.mocked(fake.client.trigger)
      .mockImplementationOnce(() => firstResponse)
      .mockResolvedValueOnce({
        workers: [{ id: 'router-new', name: 'llm-router' }],
      })
    const states: Array<{
      present: boolean
      loading: boolean
      revision: number
    }> = []
    const presence = watcher(fake.client, (state) => states.push(state))

    const oldProbe = presence.refresh()
    await expect(presence.refresh()).resolves.toBe(true)
    resolveFirst?.({ workers: [] })
    await expect(oldProbe).resolves.toBe(false)

    expect(states.at(-1)).toEqual({
      present: true,
      loading: false,
      revision: 1,
    })
  })

  it('does not let an in-flight probe undo a lifecycle removal', async () => {
    let resolveProbe: ((value: { workers: WorkerRow[] }) => void) | undefined
    const probeResponse = new Promise<{ workers: WorkerRow[] }>((resolve) => {
      resolveProbe = resolve
    })
    const fake = fakeClient()
    vi.mocked(fake.client.trigger).mockImplementationOnce(() => probeResponse)
    const states: Array<{
      present: boolean
      loading: boolean
      revision: number
    }> = []
    const presence = watcher(fake.client, (state) => states.push(state))

    const probe = presence.refresh()
    presence.markAbsent()
    resolveProbe?.({ workers: [{ id: 'router-old', name: 'llm-router' }] })
    await expect(probe).resolves.toBe(true)

    expect(states.at(-1)).toEqual({
      present: false,
      loading: false,
      revision: 0,
    })
  })

  it('forces a consumer revision on reconnect and disposes every binding', async () => {
    const fake = fakeClient([{ id: 'router-a', name: 'llm-router' }])
    const states: Array<{
      present: boolean
      loading: boolean
      revision: number
    }> = []
    const presence = watcher(fake.client, (state) => states.push(state))

    await presence.refresh()
    fake.connect()
    await settle()
    expect(states.at(-1)?.revision).toBe(2)

    presence.dispose()
    expect(fake.offConnection).toHaveBeenCalledOnce()
    expect(fake.offTrigger).toHaveBeenCalledOnce()
    expect(fake.offHandler).toHaveBeenCalledOnce()
  })
})

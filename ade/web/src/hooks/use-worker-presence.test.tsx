// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { describe, expect, it, vi } from 'vitest'

type Reply = { workers: Array<{ name: string }> }
type ComposeList = {
  projects: Array<{
    file?: string
    containers: Array<{ container: string; state: string }>
  }>
}

const client = vi.hoisted(() => {
  const handlers = new Map<string, (payload: unknown) => void>()
  const reads: Array<(reply: Reply) => void> = []
  const state = {
    // What `compose::list` answers: no project by default.
    compose: { projects: [] } as ComposeList,
    // What `compose::up` answers.
    up: { status: 'ok' } as { status: 'ok' | 'failed' },
  }
  return {
    handlers,
    reads,
    state,
    browserId: 'tab',
    on: (id: string, handler: (payload: unknown) => void) => {
      handlers.set(id, handler)
      return () => handlers.delete(id)
    },
    observe: vi.fn(() => vi.fn()),
    // Every `engine::workers::list` read stays pending until the test
    // resolves it, so replies can be delivered in any order. An add fails;
    // compose answers at once.
    call: vi.fn((fn: string) => {
      if (fn === 'compose::add') return Promise.reject(new Error('timed out'))
      if (fn === 'compose::list') return Promise.resolve(state.compose)
      if (fn === 'compose::up') return Promise.resolve(state.up)
      return new Promise<Reply>((resolve) => reads.push(resolve))
    }),
  }
})
vi.mock('@/lib/iii-client', () => ({ getIiiClient: async () => client }))

import { useHarnessStatus } from './use-harness-status'
import { useWorkerPresence } from './use-worker-presence'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const flush = () => act(async () => {})
const listed = (...names: string[]): Reply => ({
  workers: names.map((name) => ({ name })),
})
/** The hook's `engine::on-workers-changed` handler (not its compose one). */
const workersHandler = () =>
  [...client.handlers.entries()].find(([id]) => !id.includes('compose'))?.[1]
const reset = () => {
  client.handlers.clear()
  client.reads.length = 0
  client.call.mockClear()
  client.state.compose = { projects: [] }
  client.state.up = { status: 'ok' }
}

async function outOfOrderReplies(
  usePresent: () => boolean,
  name: string,
): Promise<boolean[]> {
  reset()
  const seen: boolean[] = []
  function Probe() {
    seen.push(usePresent())
    return null
  }
  const root = createRoot(document.createElement('div'))
  await act(async () => root.render(<Probe />))
  await flush()
  await act(async () => client.reads.shift()?.(listed()))

  const handler = workersHandler()
  handler?.({ event: 'worker_connected', worker_id: 'w-1' })
  handler?.({ event: 'worker_metadata_updated', worker_id: 'w-1' })
  await flush()
  expect(client.reads).toHaveLength(2)
  const [older, newer] = client.reads.splice(0)
  // The newer read sees the worker; the older one answers last.
  await act(async () => newer?.(listed(name)))
  await act(async () => older?.(listed()))

  act(() => root.unmount())
  return seen
}

describe('presence re-reads', () => {
  it('useWorkerPresence keeps the newest read when replies arrive out of order', async () => {
    const seen = await outOfOrderReplies(
      () =>
        useWorkerPresence({
          workerName: 'shell',
          watchFnId: 'ade::test-watch',
          enabled: true,
        }).present,
      'shell',
    )
    expect(seen.at(-1)).toBe(true)
  })

  it('useHarnessStatus keeps the newest read when replies arrive out of order', async () => {
    const seen = await outOfOrderReplies(
      () => useHarnessStatus(true).present,
      'harness',
    )
    expect(seen.at(-1)).toBe(true)
  })

  it('useHarnessStatus lets a re-read issued during the install re-check win', async () => {
    reset()
    let status: ReturnType<typeof useHarnessStatus> | undefined
    function Probe() {
      status = useHarnessStatus(true)
      return null
    }
    const root = createRoot(document.createElement('div'))
    await act(async () => root.render(<Probe />))
    await flush()
    await act(async () => client.reads.shift()?.(listed()))

    // The add fails, so install re-checks presence; an engine::on-workers-changed
    // event starts a newer read before that re-check answers.
    await act(async () => status?.install())
    await flush()
    const handler = workersHandler()
    handler?.({ event: 'worker_disconnected', worker_id: 'w-1' })
    await flush()
    expect(client.reads).toHaveLength(2)
    const [recheck, newer] = client.reads.splice(0)
    await act(async () => recheck?.(listed('harness')))
    await act(async () => newer?.(listed()))

    expect(status?.present).toBe(false)
    act(() => root.unmount())
  })
})

describe('a declared but stopped harness', () => {
  const declared = (state: string): ComposeList => ({
    projects: [
      {
        file: '/proj/worker-compose.yaml',
        containers: [{ container: 'harness', state }],
      },
    ],
  })

  async function mount() {
    let status: ReturnType<typeof useHarnessStatus> | undefined
    function Probe() {
      status = useHarnessStatus(true)
      return null
    }
    const root = createRoot(document.createElement('div'))
    await act(async () => root.render(<Probe />))
    await flush()
    return { root, status: () => status }
  }

  it('reads as stopped, not missing, and starts with compose::up', async () => {
    reset()
    client.state.compose = declared('stopped')
    const { root, status } = await mount()
    await act(async () => client.reads.shift()?.(listed()))

    expect(status()?.present).toBe(false)
    expect(status()?.stopped).toBe(true)
    expect(status()?.installing).toBe(false)

    await act(async () => status()?.start())
    await flush()
    expect(client.call).toHaveBeenCalledWith(
      'compose::up',
      { file: '/proj/worker-compose.yaml', container: 'harness' },
      expect.anything(),
    )
    expect(status()?.present).toBe(true)
    expect(status()?.error).toBeNull()
    act(() => root.unmount())
  })

  it('surfaces a failed start under the start action and retries it', async () => {
    reset()
    client.state.compose = declared('failed')
    client.state.up = { status: 'failed' }
    const { root, status } = await mount()
    await act(async () => client.reads.shift()?.(listed()))

    await act(async () => status()?.start())
    await flush()
    // The failure re-checks presence before it is surfaced.
    await act(async () => client.reads.shift()?.(listed()))
    expect(status()?.error).toBeTruthy()
    expect(status()?.action).toBe('start')
    expect(status()?.present).toBe(false)

    client.call.mockClear()
    client.state.up = { status: 'ok' }
    await act(async () => status()?.retry())
    await flush()
    expect(client.call.mock.calls.map(([fn]) => fn)).toContain('compose::up')
    expect(client.call.mock.calls.map(([fn]) => fn)).not.toContain(
      'compose::add',
    )
    expect(status()?.present).toBe(true)
    act(() => root.unmount())
  })

  it('shows a start in progress while compose brings it up on its own', async () => {
    reset()
    client.state.compose = declared('starting')
    const { root, status } = await mount()
    await act(async () => client.reads.shift()?.(listed()))

    expect(status()?.stopped).toBe(false)
    expect(status()?.installing).toBe(true)
    expect(status()?.action).toBe('start')
    act(() => root.unmount())
  })

  it('is not stopped once the engine lists it or compose does not declare it', async () => {
    reset()
    client.state.compose = declared('stopped')
    const { root, status } = await mount()
    await act(async () => client.reads.shift()?.(listed('harness')))
    expect(status()?.stopped).toBe(false)
    expect(status()?.present).toBe(true)
    act(() => root.unmount())

    reset()
    const second = await mount()
    await act(async () => client.reads.shift()?.(listed()))
    expect(second.status()?.stopped).toBe(false)
    expect(second.status()?.present).toBe(false)
    act(() => second.root.unmount())
  })
})

// @vitest-environment jsdom

import { act } from 'react'
import { createRoot } from 'react-dom/client'
import { describe, expect, it, vi } from 'vitest'

type Reply = { workers: Array<{ name: string }> }

const client = vi.hoisted(() => {
  const handlers = new Map<string, (payload: unknown) => void>()
  const reads: Array<(reply: Reply) => void> = []
  return {
    handlers,
    reads,
    browserId: 'tab',
    on: (id: string, handler: (payload: unknown) => void) => {
      handlers.set(id, handler)
      return () => handlers.delete(id)
    },
    registerTrigger: vi.fn(() => vi.fn()),
    // Every `engine::workers::list` read stays pending until the test
    // resolves it, so replies can be delivered in any order.
    trigger: vi.fn(() => new Promise<Reply>((resolve) => reads.push(resolve))),
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

async function outOfOrderReplies(
  usePresent: () => boolean,
  name: string,
): Promise<boolean[]> {
  client.handlers.clear()
  client.reads.length = 0
  const seen: boolean[] = []
  function Probe() {
    seen.push(usePresent())
    return null
  }
  const root = createRoot(document.createElement('div'))
  await act(async () => root.render(<Probe />))
  await flush()
  await act(async () => client.reads.shift()?.(listed()))

  const [handler] = client.handlers.values()
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
          watchFnId: 'console::test-watch',
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
})

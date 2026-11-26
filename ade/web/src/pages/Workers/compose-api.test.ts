import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ExtensionIii } from '@/types/injectable-ui'
import { fetchComposeStatus } from './api/workers'
import { composeApi } from './compose-api'

const { call } = vi.hoisted(() => ({ call: vi.fn() }))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call }),
}))

const activeFile = '/projects/custom-stack.yaml'
const engineWorker = {
  id: 'worker-1',
  name: 'harness',
  status: 'connected',
  function_count: 1,
  connected_at_ms: 1,
  active_invocations: 0,
}
const status = {
  file: activeFile,
  containers: [{ container: 'harness', state: 'ready', pid: 42 }],
}

beforeEach(() => {
  call.mockReset()
  call.mockImplementation(async (fn: string) => {
    if (fn === 'ade::compose::status') return status
    if (fn === 'engine::workers::list') return { workers: [engineWorker] }
    if (fn === 'ade::compose::project')
      return { file: activeFile, containers: [] }
    return {}
  })
})

describe('Compose project scope', () => {
  it('bootstraps through ADE so its inherited compose file is used', async () => {
    const api = composeApi({ call } as unknown as ExtensionIii, () => undefined)
    const snapshot = await api.snapshot()
    expect(snapshot.status.file).toBe(activeFile)
    expect(snapshot.workers).toEqual([
      expect.objectContaining({ name: 'harness', managementKind: 'compose' }),
    ])
    expect(call).toHaveBeenCalledWith(
      'ade::compose::status',
      {},
      { timeoutMs: 10_000 },
    )
    expect(call.mock.calls.some(([fn]) => fn === 'compose::status')).toBe(false)
  })

  it('scopes both status reads and lifecycle calls to the selected file', async () => {
    const selected = '/projects/another-stack.yaml'
    const api = composeApi({ call } as unknown as ExtensionIii, () => selected)
    await api.snapshot()
    const statuses = call.mock.calls.filter(
      ([fn]) => fn === 'ade::compose::status',
    )
    expect(statuses).toHaveLength(2)
    for (const [, payload] of statuses)
      expect(payload).toEqual({ file: selected })
    expect(call).toHaveBeenCalledWith(
      'ade::compose::project',
      { file: selected },
      { timeoutMs: 15_000 },
    )
    await api.lifecycle('restart', 'harness')
    expect(call).toHaveBeenCalledWith(
      'compose::restart',
      { file: selected, container: 'harness' },
      { timeoutMs: 600_000 },
    )
  })

  it('retains standalone engine workers when Compose is unavailable', async () => {
    call.mockImplementation(async (fn: string) => {
      if (fn === 'ade::compose::status') throw new Error('unavailable')
      if (fn === 'engine::workers::list') return { workers: [engineWorker] }
      return {}
    })
    expect(await fetchComposeStatus()).toBeNull()
    const api = composeApi({ call } as unknown as ExtensionIii, () => undefined)
    expect(await api.snapshot()).toMatchObject({
      composed: false,
      workers: [
        expect.objectContaining({
          name: 'harness',
          managementKind: 'standalone',
        }),
      ],
    })
  })
})

import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkerInfoResponse } from '@/components/chat/engine/parsers'
import { getIiiClient } from '@/lib/iii-client'
import {
  getConfiguration,
  getConfigurationSchema,
  listConfigurations,
  setConfiguration,
} from '@/pages/Configuration/tabs/WorkersTab/api'
import { fetchEngineWorkerInfo, fetchEngineWorkersList } from '../api/workers'
import { loadWorkerConfigurationReference } from './worker-configuration-reference'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))
vi.mock('../api/workers', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../api/workers')>()),
  fetchEngineWorkerInfo: vi.fn(),
  fetchEngineWorkersList: vi.fn(),
}))
vi.mock('@/pages/Configuration/tabs/WorkersTab/api', () => ({
  getConfiguration: vi.fn(),
  getConfigurationSchema: vi.fn(),
  listConfigurations: vi.fn(),
  setConfiguration: vi.fn(),
}))

const schemaEntry = {
  id: 'actual-project-console',
  name: 'ADE',
  description: 'Registered ADE settings',
  schema: {
    type: 'object',
    properties: {
      http_port: {
        type: 'integer',
        default: 3113,
        description: 'Listener port',
      },
    },
  },
}
const trigger = vi.fn()
const client: Awaited<ReturnType<typeof getIiiClient>> = {
  browserId: 'configuration-reference-test',
  trigger,
  on: () => () => {},
  registerTrigger: () => () => {},
  addConnectionStateListener: () => () => {},
  dispose: async () => {},
}
const workerInfo = (functions: string[]): WorkerInfoResponse => ({
  worker: {
    id: 'ade-instance',
    name: 'ade',
    namespace: 'worker-space',
    status: 'connected',
    function_count: functions.length,
    internal: false,
    connected_at_ms: 0,
    active_invocations: 0,
  },
  functions: functions.map((function_id) => ({
    function_id,
    worker_name: 'ade',
    namespace: 'worker-space',
  })),
  trigger_types: [],
  registered_triggers: [],
})

describe('loadWorkerConfigurationReference', () => {
  beforeEach(() => {
    vi.resetAllMocks()
    vi.mocked(getIiiClient).mockResolvedValue(client)
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(workerInfo([]))
    vi.mocked(fetchEngineWorkersList).mockResolvedValue({
      workers: [workerInfo([]).worker],
    })
    vi.mocked(listConfigurations).mockResolvedValue([schemaEntry])
    vi.mocked(getConfigurationSchema).mockResolvedValue(schemaEntry)
    trigger.mockResolvedValue({ id: schemaEntry.id })
  })

  it('loads the advertised instance ID rather than guessing from the worker name', async () => {
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(
      workerInfo(['console::status', 'console::configuration-id']),
    )
    await expect(loadWorkerConfigurationReference('ade')).resolves.toEqual({
      kind: 'schema',
      entry: schemaEntry,
    })
    expect(fetchEngineWorkerInfo).toHaveBeenCalledWith('ade')
    expect(trigger).toHaveBeenCalledExactlyOnceWith(
      'console::configuration-id',
      {},
      { namespace: 'worker-space' },
    )
    expect(getConfigurationSchema).toHaveBeenCalledExactlyOnceWith(
      'actual-project-console',
    )
    expect(listConfigurations).not.toHaveBeenCalled()
    expect(getConfiguration).not.toHaveBeenCalled()
    expect(setConfiguration).not.toHaveBeenCalled()
  })

  it('offers the actual catalog when no identity is advertised', async () => {
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(
      workerInfo(['ade::configuration-id-shadow', 'ade::status']),
    )
    await expect(loadWorkerConfigurationReference('ade')).resolves.toEqual({
      kind: 'catalog',
      reason: 'missing',
      entries: [schemaEntry],
    })
    expect(listConfigurations).toHaveBeenCalledOnce()
    expect(trigger).not.toHaveBeenCalled()
    expect(getConfigurationSchema).not.toHaveBeenCalled()
    expect(getConfiguration).not.toHaveBeenCalled()
    expect(setConfiguration).not.toHaveBeenCalled()
  })

  it('requires catalog selection instead of choosing one of several identities', async () => {
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(
      workerInfo(['first::configuration-id', 'second::configuration-id']),
    )
    await expect(loadWorkerConfigurationReference('ade')).resolves.toEqual({
      kind: 'catalog',
      reason: 'ambiguous',
      entries: [schemaEntry],
    })
    expect(trigger).not.toHaveBeenCalled()
    expect(getConfigurationSchema).not.toHaveBeenCalled()
  })

  it('rejects an advertised identity that returns an empty ID', async () => {
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(
      workerInfo(['console::configuration-id']),
    )
    trigger.mockResolvedValue({ id: '  ' })
    await expect(loadWorkerConfigurationReference('ade')).rejects.toThrow(
      /entry ID/i,
    )
    expect(getConfigurationSchema).not.toHaveBeenCalled()
    expect(listConfigurations).not.toHaveBeenCalled()
  })

  it('surfaces schema read failures without fetching values or writing configuration', async () => {
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(
      workerInfo(['console::configuration-id']),
    )
    vi.mocked(getConfigurationSchema).mockRejectedValue(
      new Error('Schema service unavailable'),
    )
    await expect(loadWorkerConfigurationReference('ade')).rejects.toThrow(
      'Schema service unavailable',
    )
    expect(trigger).toHaveBeenCalledExactlyOnceWith(
      'console::configuration-id',
      {},
      { namespace: 'worker-space' },
    )
    expect(getConfiguration).not.toHaveBeenCalled()
    expect(setConfiguration).not.toHaveBeenCalled()
    expect(listConfigurations).not.toHaveBeenCalled()
  })

  it('does not turn a worker lookup failure into an empty successful reference', async () => {
    vi.mocked(fetchEngineWorkerInfo).mockRejectedValue(
      new Error('Worker lookup failed'),
    )
    await expect(loadWorkerConfigurationReference('ade')).rejects.toThrow(
      'Worker lookup failed',
    )
    expect(listConfigurations).not.toHaveBeenCalled()
    expect(trigger).not.toHaveBeenCalled()
  })

  it('rejects a schema for a different entry instead of mislabeling it as the worker reference', async () => {
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(
      workerInfo(['console::configuration-id']),
    )
    vi.mocked(getConfigurationSchema).mockResolvedValue({
      ...schemaEntry,
      id: 'other-worker',
    })
    await expect(loadWorkerConfigurationReference('ade')).rejects.toThrow(
      /does not match.*entry ID/i,
    )
  })

  it('requires explicit catalog selection for default and namespaced live workers with the same name', async () => {
    const worker = workerInfo([]).worker
    vi.mocked(fetchEngineWorkersList).mockResolvedValue({
      workers: [
        { ...worker, id: 'default-ade', namespace: 'default' },
        { ...worker, id: 'project-ade' },
      ],
    })
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(
      workerInfo(['console::configuration-id']),
    )
    await expect(loadWorkerConfigurationReference('ade')).resolves.toEqual({
      kind: 'catalog',
      reason: 'ambiguous-worker',
      entries: [schemaEntry],
    })
    expect(fetchEngineWorkerInfo).not.toHaveBeenCalled()
    expect(trigger).not.toHaveBeenCalled()
    expect(getConfigurationSchema).not.toHaveBeenCalled()
  })

  it('counts only live matching names without blocking on unrelated duplicate workers', async () => {
    const worker = workerInfo([]).worker
    vi.mocked(fetchEngineWorkersList).mockResolvedValue({
      workers: [
        worker,
        { ...worker, id: 'other-one', name: 'other' },
        { ...worker, id: 'other-two', name: 'other' },
        { ...worker, id: 'old-ade', status: 'disconnected' },
      ],
    })
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(
      workerInfo(['console::configuration-id']),
    )
    await expect(loadWorkerConfigurationReference('ade')).resolves.toEqual({
      kind: 'schema',
      entry: schemaEntry,
    })
    expect(fetchEngineWorkersList).toHaveBeenCalledOnce()
    expect(trigger).toHaveBeenCalledExactlyOnceWith(
      'console::configuration-id',
      {},
      { namespace: 'worker-space' },
    )
  })

  it('does not claim a specific reference for a disconnected worker', async () => {
    vi.mocked(fetchEngineWorkersList).mockResolvedValue({
      workers: [{ ...workerInfo([]).worker, status: 'disconnected' }],
    })
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(
      workerInfo(['console::configuration-id']),
    )
    await expect(loadWorkerConfigurationReference('ade')).resolves.toEqual({
      kind: 'catalog',
      reason: 'unavailable',
      entries: [schemaEntry],
    })
    expect(fetchEngineWorkerInfo).not.toHaveBeenCalled()
    expect(trigger).not.toHaveBeenCalled()
  })

  it('requires catalog selection when the advertised namespace is unavailable', async () => {
    const info = workerInfo(['console::configuration-id'])
    delete info.worker.namespace
    delete info.functions[0].namespace
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(info)
    await expect(loadWorkerConfigurationReference('ade')).resolves.toEqual({
      kind: 'catalog',
      reason: 'unverified',
      entries: [schemaEntry],
    })
    expect(trigger).not.toHaveBeenCalled()
  })

  it('does not invoke an advertised identity scoped outside the resolved worker', async () => {
    const info = workerInfo(['console::configuration-id'])
    info.functions[0].namespace = 'different-space'
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(info)
    await expect(loadWorkerConfigurationReference('ade')).resolves.toEqual({
      kind: 'catalog',
      reason: 'unverified',
      entries: [schemaEntry],
    })
    expect(trigger).not.toHaveBeenCalled()
  })

  it('requires the info result to identify the unique live worker before invoking its identity', async () => {
    const info = workerInfo(['console::configuration-id'])
    info.worker.id = 'other-ade-instance'
    vi.mocked(fetchEngineWorkerInfo).mockResolvedValue(info)
    await expect(loadWorkerConfigurationReference('ade')).resolves.toEqual({
      kind: 'catalog',
      reason: 'unverified',
      entries: [schemaEntry],
    })
    expect(trigger).not.toHaveBeenCalled()
  })
})

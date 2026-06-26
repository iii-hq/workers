import { getIiiClient } from '@/lib/iii-client'
import {
  type WorkerInfoResponse,
  type WorkersListResponse,
  workerInfoResponseSchema,
  workersListResponseSchema,
} from '@/components/chat/engine/parsers'
import {
  type WorkerEntry,
  type WorkerListResponse,
  workerListResponseSchema,
} from '@/components/chat/worker/parsers'
import type { ConfigurationSchemaView } from '@/pages/Configuration/tabs/WorkersTab/api'
import { listConfigurations } from '@/pages/Configuration/tabs/WorkersTab/api'

export const WORKERS_RPC = {
  engineList: 'engine::workers::list',
  engineInfo: 'engine::workers::info',
  supervisorList: 'worker::list',
  supervisorStop: 'worker::stop',
  configList: 'configuration::list',
} as const

export async function fetchEngineWorkersList(): Promise<WorkersListResponse> {
  const client = await getIiiClient()
  const raw = await client.trigger<unknown>(WORKERS_RPC.engineList, {})
  const parsed = workersListResponseSchema.safeParse(raw)
  return parsed.success ? parsed.data : { workers: [] }
}

export async function fetchEngineWorkerInfo(
  name: string,
): Promise<WorkerInfoResponse | null> {
  const client = await getIiiClient()
  const raw = await client.trigger<unknown>(WORKERS_RPC.engineInfo, { name })
  const parsed = workerInfoResponseSchema.safeParse(raw)
  return parsed.success ? parsed.data : null
}

export async function fetchSupervisorWorkersList(): Promise<WorkerListResponse> {
  const client = await getIiiClient()
  const raw = await client.trigger<unknown>(WORKERS_RPC.supervisorList, {})
  const parsed = workerListResponseSchema.safeParse(raw)
  return parsed.success ? parsed.data : { workers: [] }
}

export async function fetchConfigurationIds(): Promise<ConfigurationSchemaView[]> {
  return listConfigurations()
}

export async function stopSupervisorWorker(name: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(WORKERS_RPC.supervisorStop, { name, yes: true })
}

export interface RawWorkersSnapshot {
  engineWorkers: WorkersListResponse['workers']
  supervisorWorkers: WorkerEntry[]
  configurations: ConfigurationSchemaView[]
  infoByName: Map<string, WorkerInfoResponse['worker']>
}

/** Bounded parallel map — avoids stampeding the engine on large fleets. */
export async function mapWithConcurrency<T, R>(
  items: T[],
  limit: number,
  fn: (item: T) => Promise<R>,
): Promise<R[]> {
  const results: R[] = new Array(items.length)
  let index = 0

  async function worker(): Promise<void> {
    while (index < items.length) {
      const i = index++
      results[i] = await fn(items[i] as T)
    }
  }

  const workers = Array.from(
    { length: Math.min(limit, items.length) },
    () => worker(),
  )
  await Promise.all(workers)
  return results
}

export async function fetchRawWorkersSnapshot(): Promise<RawWorkersSnapshot> {
  const [engineList, supervisorList, configurations] = await Promise.all([
    fetchEngineWorkersList(),
    fetchSupervisorWorkersList(),
    fetchConfigurationIds(),
  ])

  const connected = engineList.workers.filter(
    (w) => w.status.toLowerCase() === 'connected' && w.name,
  )
  const names = connected
    .map((w) => w.name as string)
    .filter((name, i, arr) => arr.indexOf(name) === i)

  const infoEntries = await mapWithConcurrency(names, 8, async (name) => {
    const info = await fetchEngineWorkerInfo(name)
    return [name, info?.worker ?? null] as const
  })

  const infoByName = new Map<string, WorkerInfoResponse['worker']>()
  for (const [name, worker] of infoEntries) {
    if (worker) infoByName.set(name, worker)
  }

  return {
    engineWorkers: engineList.workers,
    supervisorWorkers: supervisorList.workers,
    configurations,
    infoByName,
  }
}

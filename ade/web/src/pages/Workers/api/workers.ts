import { z } from 'zod'
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
import { functionRegistered } from '@/lib/function-presence'
import { getIiiClient } from '@/lib/iii-client'

export const WORKERS_RPC = {
  engineList: 'engine::workers::list',
  engineInfo: 'engine::workers::info',
  supervisorList: 'worker::list',
  supervisorStop: 'worker::stop',
  composeStatus: 'compose::status',
} as const

export const composeContainerSchema = z.object({
  container: z.string(),
  state: z.enum(['starting', 'ready', 'restarting', 'failed', 'stopped']),
  owned: z.boolean().optional(),
  pid: z.number().nullable().optional(),
  last_error: z.string().nullable().optional(),
})
export type ComposeContainer = z.infer<typeof composeContainerSchema>

export const composeStatusSchema = z.object({
  namespace: z.string().nullable().optional(),
  file: z.string().nullable().optional(),
  state_dir: z.string().nullable().optional(),
  daemon_pid: z.number().nullable().optional(),
  containers: z.array(composeContainerSchema).default([]),
})
export type ComposeStatus = z.infer<typeof composeStatusSchema>

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
  // `worker::list` is the legacy supervisor surface; a compose-managed engine
  // has no provider for it. The caller already tolerates the miss, but the
  // engine logged `Function not found` on every Workers-page refresh.
  if (!(await functionRegistered(WORKERS_RPC.supervisorList))) {
    return { workers: [] }
  }
  const client = await getIiiClient()
  const raw = await client.trigger<unknown>(WORKERS_RPC.supervisorList, {})
  const parsed = workerListResponseSchema.safeParse(raw)
  return parsed.success ? parsed.data : { workers: [] }
}

export async function fetchComposeStatus(): Promise<ComposeStatus | null> {
  const client = await getIiiClient()
  try {
    const raw = await client.trigger<unknown>(
      WORKERS_RPC.composeStatus,
      {},
      { timeoutMs: 10_000 },
    )
    const parsed = composeStatusSchema.safeParse(raw)
    return parsed.success ? parsed.data : null
  } catch {
    return null
  }
}

export async function stopSupervisorWorker(name: string): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(WORKERS_RPC.supervisorStop, { name, yes: true })
}

export interface RawWorkersSnapshot {
  engineWorkers: WorkersListResponse['workers']
  supervisorWorkers: WorkerEntry[]
  infoByName: Map<string, WorkerInfoResponse['worker']>
  compose: ComposeStatus | null
}

/** The engine's own workers (configuration, iii-observability, …) report `available`, not `connected`. */
export function isEngineLive(status: string | undefined): boolean {
  const s = status?.toLowerCase()
  return s === 'connected' || s === 'available'
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

  const workers = Array.from({ length: Math.min(limit, items.length) }, () =>
    worker(),
  )
  await Promise.all(workers)
  return results
}

export async function fetchRawWorkersSnapshot(): Promise<RawWorkersSnapshot> {
  // Supervisor and compose reads are enrichment: an engine
  // without worker::list (a compose-managed engine, or one booted with no
  // supervisor) must not blank the whole page - the connected fleet from
  // engine::workers::list still renders, just without supervisor actions.
  const [engineList, supervisorList, compose] = await Promise.all([
    fetchEngineWorkersList(),
    fetchSupervisorWorkersList().catch(
      (): WorkerListResponse => ({ workers: [] }),
    ),
    fetchComposeStatus(),
  ])

  const connected = engineList.workers.filter(
    (w) => isEngineLive(w.status) && w.name,
  )
  const names = connected
    .map((w) => w.name as string)
    .filter((name, i, arr) => arr.indexOf(name) === i)

  const infoEntries = await mapWithConcurrency(names, 8, async (name) => {
    const info = await fetchEngineWorkerInfo(name).catch(() => null)
    return [name, info?.worker ?? null] as const
  })

  const infoByName = new Map<string, WorkerInfoResponse['worker']>()
  for (const [name, worker] of infoEntries) {
    if (worker) infoByName.set(name, worker)
  }

  return {
    engineWorkers: engineList.workers,
    supervisorWorkers: supervisorList.workers,
    infoByName,
    compose,
  }
}

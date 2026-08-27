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
import { errText } from '@/lib/errors'
import { getIiiClient } from '@/lib/iii-client'
import type { ConfigurationSchemaView } from '@/pages/Configuration/tabs/WorkersTab/api'
import { listConfigurations } from '@/pages/Configuration/tabs/WorkersTab/api'
import type { ComposeAction } from '../types'

export const WORKERS_RPC = {
  engineList: 'engine::workers::list',
  engineInfo: 'engine::workers::info',
  supervisorList: 'worker::list',
  supervisorStop: 'worker::stop',
  configList: 'configuration::list',
  composeStatus: 'compose::status',
  composeUp: 'compose::up',
  composeDown: 'compose::down',
  composeRestart: 'compose::restart',
} as const

export const composeContainerSchema = z.object({
  container: z.string(),
  state: z.enum(['starting', 'ready', 'failed', 'stopped']),
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

const composeOpResultSchema = z.object({
  status: z.string().optional(),
  changed: z.boolean().optional(),
  containers: z
    .array(
      z.object({
        container: z.string(),
        changed: z.boolean().optional(),
        state: z.string().optional(),
        error: z.unknown().optional(),
      }),
    )
    .optional(),
})

const composeAnswerSchema = composeOpResultSchema.extend({
  restarted: composeOpResultSchema.nullable().optional(),
  up: composeOpResultSchema.nullable().optional(),
  down: composeOpResultSchema.nullable().optional(),
})

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

export async function fetchConfigurationIds(): Promise<
  ConfigurationSchemaView[]
> {
  return listConfigurations()
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

const COMPOSE_FN: Record<ComposeAction, string> = {
  start: WORKERS_RPC.composeUp,
  stop: WORKERS_RPC.composeDown,
  restart: WORKERS_RPC.composeRestart,
}

export async function composeContainerAction(
  action: ComposeAction,
  container: string,
): Promise<void> {
  const client = await getIiiClient()
  const raw = await client.trigger<unknown>(
    COMPOSE_FN[action],
    { container },
    { timeoutMs: 600_000 },
  )
  const answer = composeAnswerSchema.safeParse(raw)
  if (!answer.success) return
  const { restarted, up, down, ...top } = answer.data
  const failures = [top, restarted, up, down]
    .flatMap((result) => result?.containers ?? [])
    .filter((entry) => entry.error)
    .map((entry) => `${entry.container}: ${errText(entry.error)}`)
  if (failures.length > 0) throw new Error(failures.join('\n'))
  if (top.status === 'failed') {
    throw new Error(`compose ${action} ${container} failed`)
  }
}

export interface RawWorkersSnapshot {
  engineWorkers: WorkersListResponse['workers']
  supervisorWorkers: WorkerEntry[]
  configurations: ConfigurationSchemaView[]
  infoByName: Map<string, WorkerInfoResponse['worker']>
  compose: ComposeStatus | null
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
  // Supervisor, configuration, and compose reads are enrichment: an engine
  // without worker::list (a compose-managed engine, or one booted with no
  // supervisor) must not blank the whole page - the connected fleet from
  // engine::workers::list still renders, just without stop/config detail.
  const [engineList, supervisorList, configurations, compose] =
    await Promise.all([
      fetchEngineWorkersList(),
      fetchSupervisorWorkersList().catch(
        (): WorkerListResponse => ({ workers: [] }),
      ),
      fetchConfigurationIds().catch((): ConfigurationSchemaView[] => []),
      fetchComposeStatus(),
    ])

  const connected = engineList.workers.filter(
    (w) => w.status.toLowerCase() === 'connected' && w.name,
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
    configurations,
    infoByName,
    compose,
  }
}

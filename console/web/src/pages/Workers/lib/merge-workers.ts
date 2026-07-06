import type { WorkerSummary } from '@/components/chat/engine/parsers'
import type { WorkerEntry } from '@/components/chat/worker/parsers'
import type { ConfigurationSchemaView } from '@/pages/Configuration/tabs/WorkersTab/api'
import type { RawWorkersSnapshot } from '../api/workers'
import type {
  WorkerConnectionStatus,
  WorkerManagementKind,
  WorkerRow,
} from '../types'

const STOP_REASON = {
  config: 'workers declared in config.yaml are managed by the engine',
  internal: 'internal engine workers cannot be stopped from the console',
  standalone:
    'standalone workers must be stopped from the process that started them',
  notRunning: 'worker is not running',
} as const

function configIdSet(configurations: ConfigurationSchemaView[]): Set<string> {
  const ids = new Set<string>()
  for (const c of configurations) {
    ids.add(c.id)
    if (c.name) ids.add(c.name)
  }
  return ids
}

function supervisorMap(entries: WorkerEntry[]): Map<string, WorkerEntry> {
  const map = new Map<string, WorkerEntry>()
  for (const entry of entries) {
    map.set(entry.name, entry)
  }
  return map
}

function deriveManagementKind(
  name: string,
  internal: boolean,
  configIds: Set<string>,
  supervisor: WorkerEntry | undefined,
): WorkerManagementKind {
  if (internal) return 'internal'
  if (configIds.has(name)) return 'config'
  if (supervisor) return 'supervisor'
  return 'standalone'
}

function deriveConnectionStatus(
  engineStatus: string | undefined,
  supervisor: WorkerEntry | undefined,
): WorkerConnectionStatus {
  if (engineStatus?.toLowerCase() === 'connected') return 'connected'
  if (supervisor?.running) return 'connected'
  if (supervisor && !supervisor.running) return 'stopped'
  return 'disconnected'
}

function deriveStopState(
  kind: WorkerManagementKind,
  status: WorkerConnectionStatus,
  supervisor: WorkerEntry | undefined,
): Pick<WorkerRow, 'stopEnabled' | 'stopDisabledReason'> {
  if (kind === 'supervisor' && status === 'connected' && supervisor?.running) {
    return { stopEnabled: true, stopDisabledReason: null }
  }
  if (kind === 'config') {
    return { stopEnabled: false, stopDisabledReason: STOP_REASON.config }
  }
  if (kind === 'internal') {
    return { stopEnabled: false, stopDisabledReason: STOP_REASON.internal }
  }
  if (kind === 'standalone') {
    return { stopEnabled: false, stopDisabledReason: STOP_REASON.standalone }
  }
  return { stopEnabled: false, stopDisabledReason: STOP_REASON.notRunning }
}

function engineRowToWorkerRow(
  summary: WorkerSummary,
  configIds: Set<string>,
  supervisors: Map<string, WorkerEntry>,
  infoByName: Map<
    string,
    { pid?: number; internal: boolean; tag?: string | null }
  >,
): WorkerRow {
  const name = summary.name ?? summary.id
  const info = infoByName.get(name)
  const internal = info?.internal ?? false
  const supervisor = supervisors.get(name)
  const managementKind = deriveManagementKind(
    name,
    internal,
    configIds,
    supervisor,
  )
  const status = deriveConnectionStatus(summary.status, supervisor)
  const stop = deriveStopState(managementKind, status, supervisor)

  return {
    id: summary.id,
    name,
    runtime: summary.runtime ?? null,
    ipAddress: summary.ip_address ?? null,
    version: summary.version ?? null,
    pid: info?.pid ?? supervisor?.pid ?? null,
    tag: info?.tag ?? summary.tag ?? null,
    managementKind,
    status,
    ...stop,
  }
}

function syntheticSupervisorRow(
  entry: WorkerEntry,
  configIds: Set<string>,
): WorkerRow {
  const managementKind = configIds.has(entry.name) ? 'config' : 'supervisor'
  const status: WorkerConnectionStatus = entry.running ? 'connected' : 'stopped'
  const stop = deriveStopState(managementKind, status, entry)

  return {
    id: `supervisor:${entry.name}`,
    name: entry.name,
    runtime: null,
    ipAddress: null,
    version: entry.version ?? null,
    pid: entry.pid ?? null,
    tag: null,
    managementKind,
    status,
    ...stop,
  }
}

/** Merge engine catalogue, supervisor list, and configuration registry into table rows. */
export function mergeWorkers(snapshot: RawWorkersSnapshot): WorkerRow[] {
  const configIds = configIdSet(snapshot.configurations)
  const supervisors = supervisorMap(snapshot.supervisorWorkers)
  const seen = new Set<string>()
  const rows: WorkerRow[] = []

  for (const engineWorker of snapshot.engineWorkers) {
    const name = engineWorker.name ?? engineWorker.id
    seen.add(name)
    rows.push(
      engineRowToWorkerRow(
        engineWorker,
        configIds,
        supervisors,
        snapshot.infoByName,
      ),
    )
  }

  for (const [name, entry] of supervisors) {
    if (seen.has(name)) continue
    rows.push(syntheticSupervisorRow(entry, configIds))
    seen.add(name)
  }

  rows.sort((a, b) => a.name.localeCompare(b.name))
  return rows
}

export async function fetchMergedWorkers(): Promise<WorkerRow[]> {
  const { fetchRawWorkersSnapshot } = await import('../api/workers')
  const snapshot = await fetchRawWorkersSnapshot()
  return mergeWorkers(snapshot)
}

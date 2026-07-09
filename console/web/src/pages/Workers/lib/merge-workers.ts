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

interface ConfigurationLookup {
  byId: Map<string, string>
  byName: Map<string, string>
}

function configurationLookup(
  configurations: ConfigurationSchemaView[],
): ConfigurationLookup {
  const byId = new Map<string, string>()
  const byName = new Map<string, string>()
  for (const c of configurations) {
    byId.set(c.id, c.id)
    if (c.name) byName.set(c.name, c.id)
  }
  return { byId, byName }
}

function resolveConfigurationId(
  name: string,
  lookup: ConfigurationLookup,
): string | null {
  return lookup.byId.get(name) ?? lookup.byName.get(name) ?? null
}

function supervisorMap(entries: WorkerEntry[]): Map<string, WorkerEntry> {
  const map = new Map<string, WorkerEntry>()
  for (const entry of entries) {
    map.set(entry.name, entry)
  }
  return map
}

function deriveManagementKind(
  internal: boolean,
  configurationId: string | null,
  supervisor?: WorkerEntry,
): WorkerManagementKind {
  if (internal) return 'internal'
  if (configurationId) return 'config'
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
  configurations: ConfigurationLookup,
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
  const configurationId = resolveConfigurationId(name, configurations)
  const managementKind = deriveManagementKind(
    internal,
    configurationId,
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
    configurationId,
    ...stop,
  }
}

function syntheticSupervisorRow(
  entry: WorkerEntry,
  configurations: ConfigurationLookup,
): WorkerRow {
  const configurationId = resolveConfigurationId(entry.name, configurations)
  const managementKind = configurationId ? 'config' : 'supervisor'
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
    configurationId,
    ...stop,
  }
}

/** Merge engine catalogue, supervisor list, and configuration registry into table rows. */
export function mergeWorkers(snapshot: RawWorkersSnapshot): WorkerRow[] {
  const configurations = configurationLookup(snapshot.configurations)
  const supervisors = supervisorMap(snapshot.supervisorWorkers)
  const seen = new Set<string>()
  const rows: WorkerRow[] = []

  for (const engineWorker of snapshot.engineWorkers) {
    const name = engineWorker.name ?? engineWorker.id
    seen.add(name)
    rows.push(
      engineRowToWorkerRow(
        engineWorker,
        configurations,
        supervisors,
        snapshot.infoByName,
      ),
    )
  }

  for (const [name, entry] of supervisors) {
    if (seen.has(name)) continue
    rows.push(syntheticSupervisorRow(entry, configurations))
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

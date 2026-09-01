import type { WorkerSummary } from '@/components/chat/engine/parsers'
import type { WorkerEntry } from '@/components/chat/worker/parsers'
import type { ComposeContainer, RawWorkersSnapshot } from '../api/workers'
import {
  isComposeRunning,
  type WorkerConnectionStatus,
  type WorkerManagementKind,
  type WorkerRow,
} from '../types'

const STOP_REASON = {
  internal: 'internal engine workers cannot be stopped from the console',
  standalone:
    'standalone workers must be stopped from the process that started them',
  notRunning: 'worker is not running',
} as const

export interface ComposeSummary {
  namespace: string | null
  file: string | null
  daemonPid: number | null
  ready: number
  total: number
}

export interface WorkersView {
  rows: WorkerRow[]
  compose: ComposeSummary | null
}

function supervisorMap(entries: WorkerEntry[]): Map<string, WorkerEntry> {
  const map = new Map<string, WorkerEntry>()
  for (const entry of entries) {
    map.set(entry.name, entry)
  }
  return map
}

function composeMap(
  compose: RawWorkersSnapshot['compose'],
): Map<string, ComposeContainer> {
  const map = new Map<string, ComposeContainer>()
  for (const container of compose?.containers ?? []) {
    map.set(container.container, container)
  }
  return map
}

function deriveManagementKind(
  internal: boolean,
  supervisor?: WorkerEntry,
  compose?: ComposeContainer,
): WorkerManagementKind {
  if (internal) return 'internal'
  if (compose) return 'compose'
  if (supervisor) return 'supervisor'
  return 'standalone'
}

function deriveConnectionStatus(
  engineStatus: string | undefined,
  supervisor: WorkerEntry | undefined,
  compose: ComposeContainer | undefined,
): WorkerConnectionStatus {
  if (engineStatus?.toLowerCase() === 'connected') return 'connected'
  if (compose) {
    if (compose.state === 'ready') return 'connected'
    if (compose.state === 'starting') return 'starting'
    if (compose.state === 'failed') return 'failed'
    return 'stopped'
  }
  if (supervisor?.running) return 'connected'
  if (supervisor && !supervisor.running) return 'stopped'
  return 'disconnected'
}

function deriveStopState(
  kind: WorkerManagementKind,
  status: WorkerConnectionStatus,
  supervisor: WorkerEntry | undefined,
): Pick<WorkerRow, 'stopEnabled' | 'stopDisabledReason'> {
  if (kind === 'compose') {
    return { stopEnabled: false, stopDisabledReason: null }
  }
  if (kind === 'supervisor' && status === 'connected' && supervisor?.running) {
    return { stopEnabled: true, stopDisabledReason: null }
  }
  if (kind === 'internal') {
    return { stopEnabled: false, stopDisabledReason: STOP_REASON.internal }
  }
  if (kind === 'standalone') {
    return { stopEnabled: false, stopDisabledReason: STOP_REASON.standalone }
  }
  return { stopEnabled: false, stopDisabledReason: STOP_REASON.notRunning }
}

function composeFields(
  compose: ComposeContainer | undefined,
): Pick<WorkerRow, 'composeState' | 'lastError'> {
  return {
    composeState: compose?.state ?? null,
    lastError: compose?.last_error ?? null,
  }
}

function composePid(compose: ComposeContainer | undefined): number | null {
  if (!compose || !isComposeRunning(compose.state)) return null
  return compose.pid ?? null
}

function engineRowToWorkerRow(
  summary: WorkerSummary,
  supervisors: Map<string, WorkerEntry>,
  composes: Map<string, ComposeContainer>,
  infoByName: Map<
    string,
    { pid?: number; internal: boolean; tag?: string | null }
  >,
): WorkerRow {
  const name = summary.name ?? summary.id
  const info = infoByName.get(name)
  const internal = info?.internal ?? false
  const supervisor = supervisors.get(name)
  const compose = composes.get(name)
  const managementKind = deriveManagementKind(internal, supervisor, compose)
  const status = deriveConnectionStatus(summary.status, supervisor, compose)
  const stop = deriveStopState(managementKind, status, supervisor)

  return {
    id: summary.id,
    name,
    runtime: summary.runtime ?? null,
    ipAddress: summary.ip_address ?? null,
    version: summary.version ?? null,
    pid: info?.pid ?? composePid(compose) ?? supervisor?.pid ?? null,
    tag: info?.tag ?? summary.tag ?? null,
    managementKind,
    status,
    ...stop,
    ...composeFields(compose),
  }
}

function syntheticSupervisorRow(entry: WorkerEntry): WorkerRow {
  const managementKind = 'supervisor'
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
    ...composeFields(undefined),
  }
}

function syntheticComposeRow(container: ComposeContainer): WorkerRow {
  const name = container.container
  const status = deriveConnectionStatus(undefined, undefined, container)

  return {
    id: `compose:${name}`,
    name,
    runtime: null,
    ipAddress: null,
    version: null,
    pid: composePid(container),
    tag: null,
    managementKind: 'compose',
    status,
    stopEnabled: false,
    stopDisabledReason: null,
    ...composeFields(container),
  }
}

export function summarizeCompose(
  compose: RawWorkersSnapshot['compose'],
): ComposeSummary | null {
  if (!compose) return null
  const containers = compose.containers
  return {
    namespace: compose.namespace ?? null,
    file: compose.file ?? null,
    daemonPid: compose.daemon_pid ?? null,
    ready: containers.filter((c) => c.state === 'ready').length,
    total: containers.length,
  }
}

/** Merge engine catalogue, supervisor list, and compose status into table rows. */
export function mergeWorkers(snapshot: RawWorkersSnapshot): WorkerRow[] {
  const supervisors = supervisorMap(snapshot.supervisorWorkers)
  const composes = composeMap(snapshot.compose)
  const seen = new Set<string>()
  const rows: WorkerRow[] = []

  for (const engineWorker of snapshot.engineWorkers) {
    const name = engineWorker.name ?? engineWorker.id
    seen.add(name)
    rows.push(
      engineRowToWorkerRow(
        engineWorker,
        supervisors,
        composes,
        snapshot.infoByName,
      ),
    )
  }

  for (const [name, container] of composes) {
    if (seen.has(name)) continue
    rows.push(syntheticComposeRow(container))
    seen.add(name)
  }

  for (const [name, entry] of supervisors) {
    if (seen.has(name)) continue
    rows.push(syntheticSupervisorRow(entry))
    seen.add(name)
  }

  rows.sort((a, b) => a.name.localeCompare(b.name))
  return rows
}

export function mergeWorkersView(snapshot: RawWorkersSnapshot): WorkersView {
  return {
    rows: mergeWorkers(snapshot),
    compose: summarizeCompose(snapshot.compose),
  }
}

export async function fetchWorkersView(): Promise<WorkersView> {
  const { fetchRawWorkersSnapshot } = await import('../api/workers')
  return mergeWorkersView(await fetchRawWorkersSnapshot())
}

export async function fetchMergedWorkers(): Promise<WorkerRow[]> {
  return (await fetchWorkersView()).rows
}

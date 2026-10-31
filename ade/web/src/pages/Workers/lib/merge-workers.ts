import type { WorkerSummary } from '@/components/chat/engine/parsers'
import {
  type ComposeContainer,
  isEngineLive,
  type RawWorkersSnapshot,
} from '../api/workers'
import {
  isComposeRunning,
  type WorkerConnectionStatus,
  type WorkerManagementKind,
  type WorkerRow,
} from '../types'

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
  compose?: ComposeContainer,
): WorkerManagementKind {
  if (internal) return 'internal'
  if (compose) return 'compose'
  return 'standalone'
}

function deriveConnectionStatus(
  engineStatus: string | undefined,
  compose: ComposeContainer | undefined,
): WorkerConnectionStatus {
  if (isEngineLive(engineStatus)) return 'connected'
  if (compose) {
    if (compose.state === 'ready') return 'connected'
    if (compose.state === 'starting') return 'starting'
    if (compose.state === 'failed') return 'failed'
    return 'stopped'
  }
  return 'disconnected'
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
  composes: Map<string, ComposeContainer>,
  infoByName: Map<
    string,
    { pid?: number; internal: boolean; tag?: string | null }
  >,
): WorkerRow {
  const name = summary.name ?? summary.id
  const info = infoByName.get(name)
  const internal = info?.internal ?? false
  const compose = composes.get(name)

  return {
    id: summary.id,
    name,
    runtime: summary.runtime ?? null,
    ipAddress: summary.ip_address ?? null,
    version: summary.version ?? null,
    pid: info?.pid ?? composePid(compose) ?? null,
    tag: info?.tag ?? summary.tag ?? null,
    managementKind: deriveManagementKind(internal, compose),
    status: deriveConnectionStatus(summary.status, compose),
    ...composeFields(compose),
  }
}

function syntheticComposeRow(container: ComposeContainer): WorkerRow {
  const name = container.container

  return {
    id: `compose:${name}`,
    name,
    runtime: null,
    ipAddress: null,
    version: null,
    pid: composePid(container),
    tag: null,
    managementKind: 'compose',
    status: deriveConnectionStatus(undefined, container),
    ...composeFields(container),
  }
}

/** Merge engine catalogue and compose status into table rows. */
export function mergeWorkers(snapshot: RawWorkersSnapshot): WorkerRow[] {
  const composes = composeMap(snapshot.compose)
  const seen = new Set<string>()
  const rows: WorkerRow[] = []

  for (const engineWorker of snapshot.engineWorkers) {
    const name = engineWorker.name ?? engineWorker.id
    seen.add(name)
    rows.push(engineRowToWorkerRow(engineWorker, composes, snapshot.infoByName))
  }

  for (const [name, container] of composes) {
    if (seen.has(name)) continue
    rows.push(syntheticComposeRow(container))
    seen.add(name)
  }

  rows.sort((a, b) => a.name.localeCompare(b.name))
  return rows
}

/** How the worker is managed — drives badge copy and lifecycle affordances. */
export type WorkerManagementKind =
  | 'compose'
  | 'supervisor'
  | 'standalone'
  | 'internal'

/** Connection / liveness status shown in the table. */
export type WorkerConnectionStatus =
  | 'connected'
  | 'starting'
  | 'failed'
  | 'disconnected'
  | 'stopped'

export type ComposeContainerState = 'starting' | 'ready' | 'failed' | 'stopped'

export type ComposeAction = 'start' | 'stop' | 'restart'

/** View-model row for the runtime workers table (no transport types). */
export interface WorkerRow {
  /** Stable row key — engine id when present, otherwise worker name. */
  id: string
  name: string
  runtime: string | null
  ipAddress: string | null
  version: string | null
  pid: number | null
  tag: string | null
  managementKind: WorkerManagementKind
  status: WorkerConnectionStatus
  /** When true the supervisor stop action is enabled (supervisor-managed + running). */
  stopEnabled: boolean
  /** Shown when stop is disabled. */
  stopDisabledReason: string | null
  composeState: ComposeContainerState | null
  lastError: string | null
}

export interface WorkersFilterState {
  search: string
  tag: string | null
  runtime: string | null
  management: WorkerManagementKind | null
}

export const MANAGEMENT_ORDER: readonly WorkerManagementKind[] = [
  'compose',
  'supervisor',
  'standalone',
  'internal',
]

export const MANAGEMENT_LABEL: Record<WorkerManagementKind, string> = {
  compose: 'compose',
  supervisor: 'managed',
  standalone: 'standalone',
  internal: 'internal',
}

export function isComposeRunning(state: ComposeContainerState): boolean {
  return state === 'ready' || state === 'starting'
}

export function composeActions(row: WorkerRow): ComposeAction[] {
  if (row.managementKind !== 'compose' || row.composeState === null) return []
  return isComposeRunning(row.composeState)
    ? ['stop', 'restart']
    : ['start', 'restart']
}

export function filterWorkerRows(
  rows: WorkerRow[],
  filters: WorkersFilterState,
): WorkerRow[] {
  const q = filters.search.trim().toLowerCase()
  return rows.filter((row) => {
    if (filters.tag && row.tag !== filters.tag) return false
    if (filters.runtime && row.runtime !== filters.runtime) return false
    if (filters.management && row.managementKind !== filters.management)
      return false
    if (!q) return true
    const haystack = [
      row.name,
      row.runtime,
      row.ipAddress,
      row.version,
      row.tag,
      row.managementKind,
      row.status,
      row.pid?.toString(),
    ]
      .filter(Boolean)
      .join(' ')
      .toLowerCase()
    return haystack.includes(q)
  })
}

export function distinctTags(rows: WorkerRow[]): string[] {
  const tags = new Set<string>()
  for (const row of rows) {
    if (row.tag) tags.add(row.tag)
  }
  return [...tags].sort()
}

export function distinctRuntimes(rows: WorkerRow[]): string[] {
  const runtimes = new Set<string>()
  for (const row of rows) {
    if (row.runtime) runtimes.add(row.runtime)
  }
  return [...runtimes].sort()
}

export function distinctManagement(rows: WorkerRow[]): WorkerManagementKind[] {
  const present = new Set(rows.map((row) => row.managementKind))
  return MANAGEMENT_ORDER.filter((kind) => present.has(kind))
}

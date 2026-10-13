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

export type ComposeContainerState =
  | 'starting'
  | 'ready'
  | 'restarting'
  | 'failed'
  | 'stopped'

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

export const MANAGEMENT_LABEL: Record<WorkerManagementKind, string> = {
  compose: 'compose',
  supervisor: 'managed',
  standalone: 'standalone',
  internal: 'internal',
}

export function isComposeRunning(state: ComposeContainerState): boolean {
  return state === 'ready' || state === 'starting'
}

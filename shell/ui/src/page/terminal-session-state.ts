import type { LocalTerminalLease } from './terminal-leases'

export type TerminalStatus =
  | 'connecting'
  | 'ready'
  | 'reconnecting'
  | 'disconnected'
  | 'exited'
  | 'error'

export interface TerminalSessionState {
  status: TerminalStatus
  cwd: string
  error: string | null
  notice: string | null
  sessionId: string | null
  lastSequence: number
}

export const REPLAY_TRUNCATION_NOTICE =
  '\r\n[terminal output truncated; showing retained output]\r\n'

export type TerminalSessionAction =
  | {
      type: 'connected'
      sessionId: string
      cwd: string
    }
  | {
      type: 'disconnected'
      error: string
    }
  | {
      type: 'reconnecting'
      error: string
    }
  | {
      type: 'exited'
      error: string | null
    }
  | {
      type: 'replay-truncated'
    }
  | {
      type: 'connecting'
      cwd: string
    }
  | {
      type: 'frame-applied'
      sequence: number
    }
  | {
      type: 'failed'
      error: string
    }
  | {
      type: 'closed'
    }

export type TerminalLeaseAction =
  | {
      type: 'restarted'
      lease: LocalTerminalLease
    }
  | {
      type: 'closed'
    }

export type PtySessionStatus =
  | 'attached'
  | 'detached'
  | {
      exited: {
        exit_code: number | null
        signal: string | null
        error: string | null
      }
    }

export interface TerminalConnectionAttempt {
  generation: number
  requestId: string
}

export interface TerminalConnectionCoordinator {
  begin(): TerminalConnectionAttempt
  invalidate(): void
  isCurrent(generation: number): boolean
  complete(generation: number): void
}

export function createTerminalConnectionCoordinator(
  createRequestId: () => string = () => crypto.randomUUID(),
): TerminalConnectionCoordinator {
  let generation = 0
  let requestId = createRequestId()
  return {
    begin() {
      generation += 1
      return { generation, requestId }
    },
    invalidate() {
      generation += 1
    },
    isCurrent(candidate) {
      return generation === candidate
    },
    complete(candidate) {
      if (generation === candidate) requestId = createRequestId()
    },
  }
}

export function normalizeTerminalDimensions(
  cols: number,
  rows: number,
): { cols: number; rows: number } {
  return {
    cols: Math.min(500, Math.max(2, cols)),
    rows: Math.min(500, Math.max(2, rows)),
  }
}

export function shouldDetachStaleConnection(
  cancelled: boolean,
  generation: number,
  currentGeneration: number,
): boolean {
  return cancelled && generation === currentGeneration
}

export function createTerminalSessionState(root: string): TerminalSessionState {
  return {
    status: 'connecting',
    cwd: root,
    error: null,
    notice: null,
    sessionId: null,
    lastSequence: 0,
  }
}

export function reduceTerminalSessionState(
  state: TerminalSessionState,
  action: TerminalSessionAction,
): TerminalSessionState {
  switch (action.type) {
    case 'connected':
      return {
        ...state,
        status: 'ready',
        cwd: action.cwd,
        error: null,
        sessionId: action.sessionId,
      }
    case 'disconnected':
      return {
        ...state,
        status: 'disconnected',
        error: action.error,
        sessionId: null,
      }
    case 'reconnecting':
      return {
        ...state,
        status: 'reconnecting',
        error: action.error,
      }
    case 'exited':
      return {
        ...state,
        status: action.error ? 'error' : 'exited',
        error: action.error,
      }
    case 'replay-truncated':
      return {
        ...state,
        notice: REPLAY_TRUNCATION_NOTICE,
      }
    case 'connecting':
      return createTerminalSessionState(action.cwd)
    case 'frame-applied':
      return {
        ...state,
        lastSequence: Math.max(state.lastSequence, action.sequence),
      }
    case 'failed':
      return {
        ...state,
        status: 'error',
        error: action.error,
      }
    case 'closed':
      return {
        ...state,
        status: 'disconnected',
        error: null,
        sessionId: null,
      }
    default: {
      const exhaustive: never = action
      return exhaustive
    }
  }
}

export function reduceTerminalLease(
  _lease: LocalTerminalLease | null,
  action: TerminalLeaseAction,
): LocalTerminalLease | null {
  switch (action.type) {
    case 'restarted':
      return action.lease
    case 'closed':
      return null
    default: {
      const exhaustive: never = action
      return exhaustive
    }
  }
}

export function exitedStatus(status: PtySessionStatus): {
  exit_code: number | null
  signal: string | null
  error: string | null
} | null {
  if (typeof status === 'object') return status.exited
  switch (status) {
    case 'attached':
    case 'detached':
      return null
    default: {
      const exhaustive: never = status
      return exhaustive
    }
  }
}

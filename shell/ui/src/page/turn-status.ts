export function isActiveHarnessStatus(status: string | undefined): boolean {
  return status === 'running' || status === 'awaiting_functions'
}

interface HarnessStatusSnapshot {
  session_id?: string
  turn_id?: string
  status?: string
}

export function canActivateHarnessTurn(
  turnId: string,
  completedTurnIds: ReadonlySet<string>,
): boolean {
  return !completedTurnIds.has(turnId)
}

/**
 * Resolve an active turn only when no newer lifecycle event has been observed
 * since the status request began. Live start/completion events are
 * authoritative. The completed-turn set also covers completion observed
 * before the request and later duplicate delivery for the same turn.
 */
export function activeTurnFromStatus(
  status: HarnessStatusSnapshot | null | undefined,
  conversationId: string,
  requestedAtGeneration: number,
  currentGeneration: number,
  completedTurnIds: ReadonlySet<string>,
): string | null {
  if (requestedAtGeneration !== currentGeneration) return null
  if (status?.session_id !== conversationId || typeof status.turn_id !== 'string') return null
  if (!canActivateHarnessTurn(status.turn_id, completedTurnIds)) return null
  return isActiveHarnessStatus(status.status) ? status.turn_id : null
}

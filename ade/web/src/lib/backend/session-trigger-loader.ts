import { serialRefresh } from './serial-refresh'
import type { SessionTriggerInfo } from './triggers'

export interface SessionTriggerLoader {
  refresh: () => void
  dispose: () => void
}

export function startSessionTriggerLoader({
  sessionId,
  listTriggers,
  onTriggersChanged,
  onSnapshot,
}: {
  sessionId: string
  listTriggers: (sessionId: string) => Promise<SessionTriggerInfo[]>
  onTriggersChanged?: (sessionId: string, onEvent: () => void) => () => void
  onSnapshot: (rows: SessionTriggerInfo[]) => void
}): SessionTriggerLoader {
  const loader = serialRefresh(() => listTriggers(sessionId), onSnapshot)
  let disposed = false
  const refresh = () => {
    if (!disposed) loader.refresh()
  }

  // Subscribe before the first read so a mutation in the setup gap cannot be
  // missed. All registration receipts consume this one shared snapshot.
  const off = onTriggersChanged?.(sessionId, refresh)
  refresh()

  return {
    refresh,
    dispose: () => {
      if (disposed) return
      disposed = true
      off?.()
      loader.reset()
    },
  }
}

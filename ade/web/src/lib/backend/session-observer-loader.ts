import type { SessionObserverInfo } from './observers'
import { serialRefresh } from './serial-refresh'

export interface SessionObserverLoader {
  refresh: () => void
  dispose: () => void
}

export function startSessionObserverLoader({
  sessionId,
  listObservers,
  onObserversChanged,
  onSnapshot,
}: {
  sessionId: string
  listObservers: (sessionId: string) => Promise<SessionObserverInfo[]>
  onObserversChanged?: (sessionId: string, onEvent: () => void) => () => void
  onSnapshot: (rows: SessionObserverInfo[]) => void
}): SessionObserverLoader {
  const loader = serialRefresh(() => listObservers(sessionId), onSnapshot)
  let disposed = false
  const refresh = () => {
    if (!disposed) loader.refresh()
  }

  // Subscribe before the first read so a mutation in the setup gap cannot be
  // missed. All observe receipts consume this one shared snapshot.
  const off = onObserversChanged?.(sessionId, refresh)
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

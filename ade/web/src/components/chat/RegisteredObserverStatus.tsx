import { createContext, type ReactNode, useContext, useMemo } from 'react'
import type { SessionObserverInfo } from '@/lib/backend/observers'

interface RegisteredObserverSnapshot {
  loaded: boolean
  observersById: ReadonlyMap<string, SessionObserverInfo>
}

const RegisteredObserverStatusContext =
  createContext<RegisteredObserverSnapshot | null>(null)

export function RegisteredObserverStatusProvider({
  children,
  loaded,
  observersById,
}: RegisteredObserverSnapshot & { children: ReactNode }) {
  const value = useMemo(
    () => ({ loaded, observersById }),
    [loaded, observersById],
  )

  return (
    <RegisteredObserverStatusContext.Provider value={value}>
      {children}
    </RegisteredObserverStatusContext.Provider>
  )
}

export function useRegisteredObserverActive({
  observerId,
  registered,
}: {
  observerId?: string
  registered: boolean
}): boolean {
  const snapshot = useContext(RegisteredObserverStatusContext)

  if (!registered || !observerId || !snapshot?.loaded) return registered

  const observer = snapshot.observersById.get(observerId)
  return Boolean(observer && observer.fired !== true)
}

import { createContext, type ReactNode, useContext, useMemo } from 'react'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'

interface RegisteredTriggerSnapshot {
  loaded: boolean
  triggersById: ReadonlyMap<string, SessionTriggerInfo>
}

const RegisteredTriggerStatusContext =
  createContext<RegisteredTriggerSnapshot | null>(null)

export function RegisteredTriggerStatusProvider({
  children,
  loaded,
  triggersById,
}: RegisteredTriggerSnapshot & { children: ReactNode }) {
  const value = useMemo(
    () => ({ loaded, triggersById }),
    [loaded, triggersById],
  )

  return (
    <RegisteredTriggerStatusContext.Provider value={value}>
      {children}
    </RegisteredTriggerStatusContext.Provider>
  )
}

export function useRegisteredTriggerActive({
  subscriptionId,
  registered,
}: {
  subscriptionId?: string
  registered: boolean
}): boolean {
  const snapshot = useContext(RegisteredTriggerStatusContext)

  if (!registered || !subscriptionId || !snapshot?.loaded) return registered

  const trigger = snapshot.triggersById.get(subscriptionId)
  return Boolean(trigger && trigger.fired !== true)
}

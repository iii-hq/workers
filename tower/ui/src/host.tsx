import type { Host } from '@iii-dev/console-ui'
import { createContext, useContext } from 'react'

export const TowerHost = createContext<Host | null>(null)
export function useTowerHost() {
  const host = useContext(TowerHost)
  if (!host) throw new Error('Tower must be rendered with its console host')
  return host
}

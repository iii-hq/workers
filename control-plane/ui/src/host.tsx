import type { Host } from '@iii-dev/console-ui'
import { createContext, useContext } from 'react'

export const ControlPlaneHost = createContext<Host | null>(null)
export function useControlPlaneHost() {
  const host = useContext(ControlPlaneHost)
  if (!host) throw new Error('Control plane must be rendered with its console host')
  return host
}

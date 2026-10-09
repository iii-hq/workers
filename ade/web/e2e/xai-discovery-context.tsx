import { createContext, useContext } from 'react'
import type { ProviderListEntry } from '../src/lib/models-catalog'

export const FixtureContext = createContext<{
  presentProviders: ProviderListEntry[]
  refreshModels: (id?: string) => Promise<void>
  refreshingModels: boolean
} | null>(null)
export const useConversationsCtxOptional = () => useContext(FixtureContext)

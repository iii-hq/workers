import { createContext, useContext, type ReactNode } from 'react'
import {
  useConversations,
  type ConversationsApi,
} from '@/hooks/use-conversations'
import { useModelPickerSource } from '@/hooks/use-model-picker-source'
import { getDefaultBackend } from '@/lib/backend'
import type { ChatBackend } from '@/lib/backend'
import type { ModelOption } from '@/types/chat'

const backend = getDefaultBackend()

interface ConversationsContextValue extends ConversationsApi {
  backend: ChatBackend
  modelOptions: ModelOption[]
  catalogLoading: boolean
}

const ConversationsContext = createContext<ConversationsContextValue | null>(
  null,
)

interface ConversationsProviderProps {
  children: ReactNode
}

/**
 * Hoists conversation + model-picker state above the route boundary so that
 * the `#/chat` page and the side-by-side `ChatDock` can render the same
 * conversations without re-instantiating storage state or duplicating
 * streaming controllers.
 */
export function ConversationsProvider({ children }: ConversationsProviderProps) {
  const { modelOptions, catalogKeys, catalogLoading } = useModelPickerSource(
    backend.id,
  )
  const api = useConversations(catalogKeys, !catalogLoading)

  const value: ConversationsContextValue = {
    ...api,
    backend,
    modelOptions,
    catalogLoading,
  }

  return (
    <ConversationsContext.Provider value={value}>
      {children}
    </ConversationsContext.Provider>
  )
}

export function useConversationsCtx(): ConversationsContextValue {
  const ctx = useContext(ConversationsContext)
  if (!ctx) {
    throw new Error(
      'useConversationsCtx must be used inside <ConversationsProvider>',
    )
  }
  return ctx
}

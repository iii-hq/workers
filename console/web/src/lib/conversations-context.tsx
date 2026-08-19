import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from 'react'
import {
  isApprovalGateAvailable,
  useApprovalGateStatus,
} from '@/hooks/use-approval-gate-status'
import {
  type ConversationsApi,
  useConversations,
} from '@/hooks/use-conversations'
import {
  type HarnessStatus,
  isHarnessAvailable,
  useHarnessStatus,
} from '@/hooks/use-harness-status'
import { isMemoryAvailable, useMemoryStatus } from '@/hooks/use-memory-status'
import { useModelPickerSource } from '@/hooks/use-model-picker-source'
import { isShellAvailable, useShellStatus } from '@/hooks/use-shell-status'
import {
  isWorktreeAvailable,
  useWorktreeStatus,
} from '@/hooks/use-worktree-status'
import type { ChatBackend } from '@/lib/backend'
import { getDefaultBackend } from '@/lib/backend'
import type { IiiClient } from '@/lib/iii-client'
import {
  type ProviderListEntry,
  refreshProviderModels,
} from '@/lib/models-catalog'
import { type ConversationAdapter, startUiLoader } from '@/lib/ui-loader'
import type { ModelOption } from '@/types/chat'
import type { ConsoleApi } from '@/types/injectable-ui'

const backend = getDefaultBackend()

interface ConversationsContextValue extends ConversationsApi {
  backend: ChatBackend
  modelOptions: ModelOption[]
  catalogLoading: boolean
  /** Providers present as harness workers (configured or not). */
  presentProviders: ProviderListEntry[]
  /**
   * Ask every present provider that supports model listing to re-pull its
   * upstream model list, then re-read the catalog so the picker reflects the
   * refreshed models.
   */
  refreshModels: () => Promise<void>
  refreshingModels: boolean
  /**
   * Presence of the `harness` worker plus the in-app install lifecycle.
   * Powers the chat empty state (install / configure / ready). Only live on
   * the real backend.
   */
  harnessStatus: HarnessStatus
  /**
   * Whether the optional standalone `approval-gate` worker is connected. Gates
   * all approval UI + `approval::*` RPC: false → the gate isn't installed, so
   * the console hides the permission-mode picker and runs calls ungated rather
   * than triggering "function not found". Only meaningful on the real backend.
   */
  approvalGateAvailable: boolean
  /**
   * Whether the optional `shell` worker is connected. Gates the chat
   * working-directory picker + status banner: the picker browses via the
   * shell-served `coder::*` functions and the chosen dir scopes shell exec/file
   * calls, so with shell absent the controls would call functions that don't
   * exist. Only meaningful on the real backend.
   */
  shellAvailable: boolean
  /**
   * Whether the optional `worktree` worker is connected. Gates the picker's
   * worktrees tab, the working-directory worktree badge, claim/release, and
   * the landed / land-blocked live events. Only meaningful on the real
   * backend.
   */
  worktreeAvailable: boolean
  /**
   * Whether the optional `memory` worker is connected. Gates the Memory
   * page nav entry and its `memory::*` RPC. Only meaningful on the real
   * backend.
   */
  memoryAvailable: boolean
}

const ConversationsContext = createContext<ConversationsContextValue | null>(
  null,
)

export interface InjectableUiRuntime {
  client: IiiClient
  api: ConsoleApi
}

interface ConversationsProviderProps {
  children: ReactNode
  injectableUiRuntime?: Promise<InjectableUiRuntime>
  /** Called when an injected page asks for a conversation, so the host can
      place the chat pane when none is open. */
  onConversationRequested?: (sessionId: string) => void
}

/**
 * Hoists conversation + model-picker state above the route boundary so that
 * the `#/chat` page and the side-by-side `ChatDock` can render the same
 * conversations without re-instantiating storage state or duplicating
 * streaming controllers.
 */
export function ConversationsProvider({
  children,
  injectableUiRuntime,
  onConversationRequested,
}: ConversationsProviderProps) {
  const harnessStatus = useHarnessStatus(backend.id === 'real')
  const harnessAvailable = isHarnessAvailable(harnessStatus)
  const approvalGateAvailable = isApprovalGateAvailable(
    useApprovalGateStatus(backend.id === 'real'),
  )
  const shellAvailable = isShellAvailable(useShellStatus(backend.id === 'real'))
  const worktreeAvailable = isWorktreeAvailable(
    useWorktreeStatus(backend.id === 'real'),
  )
  const memoryAvailable = isMemoryAvailable(
    useMemoryStatus(backend.id === 'real'),
  )
  const {
    modelOptions,
    catalogKeys,
    catalogLoading,
    presentProviders,
    refresh,
  } = useModelPickerSource(backend.id, harnessAvailable)
  // Conversations are backed by the session-manager worker on the real
  // backend; mocks stay in-memory.
  const api = useConversations(
    catalogKeys,
    !catalogLoading,
    backend.id === 'real',
  )

  const [refreshingModels, setRefreshingModels] = useState(false)
  const refreshModels = useCallback(async () => {
    if (!harnessAvailable) return
    setRefreshingModels(true)
    try {
      if (backend.id === 'real') {
        // Refresh the present providers that can list models. With no present
        // providers this is a no-op for discovery; the catalog re-read below
        // still runs.
        const ids = presentProviders
          .filter((p) => p.supports_model_listing)
          .map((p) => p.id)
        await refreshProviderModels(ids)
      }
      await refresh()
    } finally {
      setRefreshingModels(false)
    }
  }, [harnessAvailable, refresh, presentProviders])

  const selectConversationRef = useRef(api.select)
  selectConversationRef.current = api.select
  const conversationsRef = useRef(api.conversations)
  conversationsRef.current = api.conversations
  const activeIdRef = useRef(api.activeId)
  activeIdRef.current = api.activeId
  const conversationRequestedRef = useRef(onConversationRequested)
  conversationRequestedRef.current = onConversationRequested
  const conversationAdapterRef = useRef<ConversationAdapter | null>(null)
  if (!conversationAdapterRef.current) {
    conversationAdapterRef.current = {
      selectConversation(sessionId) {
        const id = sessionId.trim()
        if (!id) return
        selectConversationRef.current(id)
        // Selecting is only half of it: a page that started a turn wants the
        // operator to see it, and the chat pane may not be on screen at all.
        conversationRequestedRef.current?.(id)
      },
      composerModel(conversationId) {
        const requested = conversationId?.trim()
        const id = requested || activeIdRef.current
        if (!id) return null
        const model = conversationsRef.current.find(
          (conversation) => conversation.id === id,
        )?.model
        return typeof model === 'string' && model.trim() ? model.trim() : null
      },
    }
  }

  useEffect(() => {
    if (!injectableUiRuntime) return
    let active = true
    let stop: (() => void) | undefined
    void injectableUiRuntime
      .then(({ client, api: consoleApi }) => {
        if (!active || !conversationAdapterRef.current) return
        stop = startUiLoader(client, consoleApi, conversationAdapterRef.current)
      })
      .catch(() => undefined)
    return () => {
      active = false
      stop?.()
    }
  }, [injectableUiRuntime])

  const value: ConversationsContextValue = {
    ...api,
    backend,
    modelOptions,
    catalogLoading,
    presentProviders,
    refreshModels,
    refreshingModels,
    harnessStatus,
    approvalGateAvailable,
    shellAvailable,
    worktreeAvailable,
    memoryAvailable,
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

/**
 * Non-throwing variant for components that may render outside the provider
 * (e.g. Storybook). Returns `null` when there is no surrounding provider.
 */
export function useConversationsCtxOptional(): ConversationsContextValue | null {
  return useContext(ConversationsContext)
}

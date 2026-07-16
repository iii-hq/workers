import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useState,
} from 'react'
import {
  isApprovalGateAvailable,
  useApprovalGateStatus,
} from '@/hooks/use-approval-gate-status'
import {
  isBrowserAvailable,
  useBrowserStatus,
} from '@/hooks/use-browser-status'
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
import {
  type ProviderListEntry,
  refreshProviderModels,
} from '@/lib/models-catalog'
import type { ModelOption } from '@/types/chat'

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
   * Whether the optional `browser` worker is connected. Gates the Browser
   * page nav entry, its `browser::*` RPC, and the chat's browser
   * session-start notices. Only meaningful on the real backend.
   */
  browserAvailable: boolean
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

interface ConversationsProviderProps {
  children: ReactNode
}

/**
 * Hoists conversation + model-picker state above the route boundary so that
 * the `#/chat` page and the side-by-side `ChatDock` can render the same
 * conversations without re-instantiating storage state or duplicating
 * streaming controllers.
 */
export function ConversationsProvider({
  children,
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
  const browserAvailable = isBrowserAvailable(
    useBrowserStatus(backend.id === 'real'),
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
    browserAvailable,
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

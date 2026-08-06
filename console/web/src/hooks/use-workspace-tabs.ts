/**
 * Server-persisted workspace tabs (see `lib/workspace-tabs.ts` for the
 * model). Tabs live in the engine's `console` configuration entry, so the
 * layout follows the engine, and the query polls on an interval — a tab
 * created in another browser shows up here within seconds without a
 * reload.
 *
 * Every mutation applies OPTIMISTICALLY to the shared `['consoleConfig']`
 * cache first (the strip must react to a close/create/rename in the same
 * frame as the click), then writes through the same read-modify-write
 * funnel the traces saved views use; the next poll reconciles with
 * whatever the server actually stored.
 *
 * When the configuration worker (or the `console` entry) is unreachable,
 * tabs degrade to localStorage so the strip keeps working offline.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useCallback, useState } from 'react'
import {
  type ConsoleConfigValue,
  fetchConsoleConfigValue,
  setConsoleConfigValue,
} from '@/lib/console-config'
import { moveItem } from '@/lib/reorder'
import {
  defaultTabs,
  newTabId,
  parseActiveTabId,
  parseWorkspaceTabs,
  resolveActiveTab,
  type TabScreen,
  tabColumns,
  type WorkspaceTab,
  withActiveTabId,
  withColumnAdded,
  withColumnRemoved,
  withScreenDetached,
  withWorkspaceTabs,
} from '@/lib/workspace-tabs'

const CONSOLE_CONFIG_QUERY_KEY = ['consoleConfig']
const LOCAL_KEY = 'iii-workspace-tabs'

interface LocalState {
  tabs: WorkspaceTab[]
  activeTabId: string
}

function loadLocal(): LocalState {
  const fallback: LocalState = {
    tabs: defaultTabs(),
    activeTabId: defaultTabs()[0].id,
  }
  if (typeof window === 'undefined') return fallback
  try {
    const raw = window.localStorage.getItem(LOCAL_KEY)
    if (!raw) return fallback
    const parsed = JSON.parse(raw) as Record<string, unknown>
    const tabs = parseWorkspaceTabs({ workspace: parsed })
    if (tabs.length === 0) return fallback
    const activeTabId = parseActiveTabId({ workspace: parsed })
    return { tabs, activeTabId: resolveActiveTab(tabs, activeTabId).id }
  } catch {
    return fallback
  }
}

function persistLocal(state: LocalState): void {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(
      LOCAL_KEY,
      JSON.stringify({ tabs: state.tabs, activeTabId: state.activeTabId }),
    )
  } catch {
    // best-effort
  }
}

export interface UseWorkspaceTabsReturn {
  tabs: WorkspaceTab[]
  activeTabId: string
  activeTab: WorkspaceTab
  activateTab: (id: string) => void
  /** New tab with the chosen column count and (optionally) screens. */
  createTab: (opts: { columns: 1 | 2; screens?: TabScreen[] }) => void
  closeTab: (id: string) => void
  renameTab: (id: string, name: string) => void
  /** Move the tab at `from` to position `to` (indexes into `tabs`). */
  reorderTab: (from: number, to: number) => void
  /** Attach (or replace) the screen a tab column shows. */
  attachScreen: (id: string, column: number, screen: TabScreen) => void
  /** Blank a column's screen (column stays, shows the attach affordance). */
  detachScreen: (id: string, column: number) => void
  /** Grow the tab by one empty column on that side (≤ MAX_COLUMNS). */
  addColumn: (id: string, side: 'left' | 'right') => void
  /** Drop one column (the last one never goes). */
  removeColumn: (id: string, column: number) => void
  /** Persist drag-to-resize column fractions (index-aligned). */
  resizeColumns: (id: string, sizes: number[]) => void
}

export function useWorkspaceTabs(): UseWorkspaceTabsReturn {
  const qc = useQueryClient()

  // Shares the traces saved-views cache entry; refetchInterval keeps the
  // strip reactive to writes from other browsers/tabs.
  const { data } = useQuery<ConsoleConfigValue | null>({
    queryKey: CONSOLE_CONFIG_QUERY_KEY,
    queryFn: fetchConsoleConfigValue,
    staleTime: 3_000,
    refetchInterval: 5_000,
    refetchOnWindowFocus: true,
    retry: 1,
  })
  const available = data !== null && data !== undefined

  const [local, setLocal] = useState<LocalState>(loadLocal)

  // Selection made in THIS browser; wins over the server pointer so a slow
  // write never lags the strip (same pattern as useTraceViews).
  const [chosenTabId, setChosenTabId] = useState<string | null>(null)

  const serverTabs = available ? parseWorkspaceTabs(data) : []
  const tabs = available
    ? serverTabs.length > 0
      ? serverTabs
      : defaultTabs()
    : local.tabs

  const serverActiveId = available ? parseActiveTabId(data) : undefined
  const pointer =
    chosenTabId ?? (available ? serverActiveId : local.activeTabId)
  // No pointer (or one at a tab another browser closed) lands on the
  // chat+traces tab, falling back to the first tab.
  const activeTab = resolveActiveTab(tabs, pointer)
  const activeTabId = activeTab.id

  const mutation = useMutation({
    mutationFn: async (
      transform: (value: ConsoleConfigValue) => ConsoleConfigValue,
    ) => {
      const current = (await fetchConsoleConfigValue()) ?? {}
      const next = transform(current)
      await setConsoleConfigValue(next)
      return next
    },
    onSuccess: (next) => {
      qc.setQueryData(CONSOLE_CONFIG_QUERY_KEY, next)
    },
  })

  const persist = useCallback(
    (nextTabs: WorkspaceTab[], nextActiveId: string) => {
      if (available) {
        const transform = (value: ConsoleConfigValue) =>
          withActiveTabId(withWorkspaceTabs(value, nextTabs), nextActiveId)
        // Optimistic: the strip reflects the change in this frame; the
        // server write (and the next poll) reconcile behind it.
        qc.setQueryData<ConsoleConfigValue | null>(
          CONSOLE_CONFIG_QUERY_KEY,
          (old) => transform(old ?? {}),
        )
        mutation.mutateAsync(transform).catch(() => {})
      } else {
        const nextState = { tabs: nextTabs, activeTabId: nextActiveId }
        setLocal(nextState)
        persistLocal(nextState)
      }
    },
    [available, mutation, qc],
  )

  const activateTab = useCallback(
    (id: string) => {
      setChosenTabId(id)
      persist(tabs, id)
    },
    [persist, tabs],
  )

  const createTab = useCallback(
    (opts: { columns: 1 | 2; screens?: TabScreen[] }) => {
      const screens = (opts.screens ?? []).slice(0, opts.columns)
      const tab: WorkspaceTab = {
        id: newTabId(),
        columns: opts.columns,
        screens,
      }
      setChosenTabId(tab.id)
      persist([...tabs, tab], tab.id)
    },
    [persist, tabs],
  )

  const closeTab = useCallback(
    (id: string) => {
      if (tabs.length <= 1) return
      const remaining = tabs.filter((t) => t.id !== id)
      const nextActive =
        id === activeTabId ? remaining[remaining.length - 1].id : activeTabId
      setChosenTabId(nextActive)
      persist(remaining, nextActive)
    },
    [persist, tabs, activeTabId],
  )

  const renameTab = useCallback(
    (id: string, name: string) => {
      const trimmed = name.trim()
      persist(
        tabs.map((t) => {
          if (t.id !== id) return t
          const { name: _prev, ...rest } = t
          return trimmed ? { ...rest, name: trimmed } : rest
        }),
        activeTabId,
      )
    },
    [persist, tabs, activeTabId],
  )

  const reorderTab = useCallback(
    (from: number, to: number) => {
      if (from === to) return
      persist(moveItem(tabs, from, to), activeTabId)
    },
    [persist, tabs, activeTabId],
  )

  const attachScreen = useCallback(
    (id: string, column: number, screen: TabScreen) => {
      persist(
        tabs.map((t) => {
          if (t.id !== id) return t
          const columns = tabColumns(t)
          const screens: (TabScreen | null)[] = Array.from(
            { length: columns },
            (_, i) => t.screens[i] ?? null,
          )
          if (column < 0 || column >= columns) return t
          screens[column] = screen
          return { ...t, columns, screens }
        }),
        activeTabId,
      )
    },
    [persist, tabs, activeTabId],
  )

  const detachScreen = useCallback(
    (id: string, column: number) => {
      persist(
        tabs.map((t) => (t.id === id ? withScreenDetached(t, column) : t)),
        activeTabId,
      )
    },
    [persist, tabs, activeTabId],
  )

  const addColumn = useCallback(
    (id: string, side: 'left' | 'right') => {
      persist(
        tabs.map((t) => (t.id === id ? withColumnAdded(t, side) : t)),
        activeTabId,
      )
    },
    [persist, tabs, activeTabId],
  )

  const removeColumn = useCallback(
    (id: string, column: number) => {
      persist(
        tabs.map((t) => (t.id === id ? withColumnRemoved(t, column) : t)),
        activeTabId,
      )
    },
    [persist, tabs, activeTabId],
  )

  const resizeColumns = useCallback(
    (id: string, sizes: number[]) => {
      persist(
        tabs.map((t) =>
          t.id === id && sizes.length === tabColumns(t) ? { ...t, sizes } : t,
        ),
        activeTabId,
      )
    },
    [persist, tabs, activeTabId],
  )

  return {
    tabs,
    activeTabId,
    activeTab,
    activateTab,
    createTab,
    closeTab,
    renameTab,
    reorderTab,
    attachScreen,
    detachScreen,
    addColumn,
    removeColumn,
    resizeColumns,
  }
}

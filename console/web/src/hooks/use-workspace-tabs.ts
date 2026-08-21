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

import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  type ConfigTransform,
  SerializedConfigWriter,
} from '@/hooks/lib/serialized-config-writer'
import {
  type ConsoleConfigValue,
  fetchConsoleConfigValue,
  setConsoleConfigValue,
} from '@/lib/console-config'
import { moveItem } from '@/lib/reorder'
import {
  adjacentTabId,
  defaultTabs,
  type LocalActivation,
  newTabId,
  parseActivation,
  parseActiveTabId,
  parseWorkspaceTabs,
  resolveActiveTab,
  resolvePointer,
  shouldFlushPendingWrite,
  type TabScreen,
  tabColumns,
  tabPaneIds,
  type WorkspaceLayoutSource,
  type WorkspaceState,
  type WorkspaceTab,
  withActiveTabId,
  withColumnAdded,
  withPaneRemoved,
  withScreenDetached,
  withTabClosed,
  withWorkspaceScreenOpened,
  withWorkspaceTabs,
  workspaceLayoutSource,
} from '@/lib/workspace-tabs'

const CONSOLE_CONFIG_QUERY_KEY = ['consoleConfig']
const LOCAL_KEY = 'iii-workspace-tabs'
const SESSION_ACTIVE_KEY = 'iii-workspace-active'

type LocalState = WorkspaceState

export type WorkspaceTransform = (state: LocalState) => LocalState

function workspaceState(value: ConsoleConfigValue): LocalState {
  const parsedTabs = parseWorkspaceTabs(value)
  const tabs = parsedTabs.length > 0 ? parsedTabs : defaultTabs()
  const activeTabId = resolveActiveTab(tabs, parseActiveTabId(value)).id
  return { tabs, activeTabId }
}

/** Lift a transform onto the raw value; the pointer is re-stamped only when it moved. */
export function workspaceConfigTransform(
  update: WorkspaceTransform,
  now: () => number = Date.now,
): ConfigTransform {
  return (value) => {
    const next = update(workspaceState(value))
    const withTabs = withWorkspaceTabs(value, next.tabs)
    if (parseActiveTabId(value) === next.activeTabId) return withTabs
    return withActiveTabId(withTabs, next.activeTabId, 'browser', now())
  }
}

function loadSessionActivation(): LocalActivation | null {
  if (typeof window === 'undefined') return null
  try {
    const raw = window.sessionStorage.getItem(SESSION_ACTIVE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as Partial<LocalActivation>
    if (typeof parsed.tabId !== 'string' || parsed.tabId.length === 0) {
      return null
    }
    return {
      tabId: parsed.tabId,
      at: typeof parsed.at === 'number' ? parsed.at : 0,
    }
  } catch {
    return null
  }
}

function persistSessionActivation(activation: LocalActivation): void {
  if (typeof window === 'undefined') return
  try {
    window.sessionStorage.setItem(
      SESSION_ACTIVE_KEY,
      JSON.stringify(activation),
    )
  } catch {
    // best-effort
  }
}

/** Keep a newly inserted pane's identity stable across optimistic replays. */
function stabilizeAddedPane(
  before: WorkspaceTab[],
  after: WorkspaceTab[],
  stablePaneId: { current: string | null },
): WorkspaceTab[] {
  const beforeById = new Map(before.map((tab) => [tab.id, tab]))
  return after.map((tab) => {
    const previous = beforeById.get(tab.id)
    if (!previous || tabColumns(tab) !== tabColumns(previous) + 1) return tab

    const previousIds = new Set(tabPaneIds(previous))
    const nextIds = tabPaneIds(tab)
    const addedIndex = nextIds.findIndex((id) => !previousIds.has(id))
    if (addedIndex < 0) return tab

    stablePaneId.current ??= nextIds[addedIndex]
    if (nextIds[addedIndex] === stablePaneId.current) return tab
    nextIds[addedIndex] = stablePaneId.current
    return { ...tab, paneIds: nextIds }
  })
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
  /** `pending` until the first server answer; then `server`, or `local`
      while the configuration entry is unreachable and `tabs` is the
      localStorage copy. Flips `local` to `server` when a later poll succeeds. */
  layoutSource: WorkspaceLayoutSource
  tabs: WorkspaceTab[]
  activeTabId: string
  activeTab: WorkspaceTab
  activateTab: (id: string) => void
  /** New tab with the chosen column count and (optionally) screens. */
  createTab: (opts: { columns: 1 | 2; screens?: TabScreen[] }) => void
  closeTab: (id: string) => void
  renameTab: (id: string, name: string) => void
  /** Activate the next (`1`) or previous (`-1`) tab, wrapping around. */
  activateAdjacent: (steps: 1 | -1) => void
  /** Move the tab at `from` to position `to` (indexes into `tabs`). */
  reorderTab: (from: number, to: number) => void
  /** Attach (or replace) the screen a tab column shows. */
  attachScreen: (id: string, column: number, screen: TabScreen) => void
  /** Blank a column's screen (column stays, shows the attach affordance). */
  detachScreen: (id: string, column: number) => void
  /** Grow the tab by one empty column on that side (up to the safety ceiling). */
  addColumn: (id: string, side: 'left' | 'right') => void
  /** Drop one pane by stable identity (the last one never goes). */
  removeColumn: (id: string, paneId: string) => void
  /** Persist drag-to-resize column fractions (index-aligned). */
  resizeColumns: (id: string, sizes: number[]) => void
  /** Reuse an existing screen or place it beside chat without replacing panes. */
  openScreen: (screen: TabScreen) => void
  /** Open relative to a specific tab without stealing a later tab selection. */
  openScreenInTab: (id: string, screen: TabScreen) => void
}

export function useWorkspaceTabs(): UseWorkspaceTabsReturn {
  const qc = useQueryClient()
  const writerRef = useRef<SerializedConfigWriter | null>(null)
  const writer =
    writerRef.current ??
    new SerializedConfigWriter({
      readRemote: fetchConsoleConfigValue,
      writeRemote: setConsoleConfigValue,
      readCached: () =>
        qc.getQueryData<ConsoleConfigValue | null>(CONSOLE_CONFIG_QUERY_KEY),
      publish: (value) => {
        qc.setQueryData(CONSOLE_CONFIG_QUERY_KEY, value)
      },
      cancelReads: () => {
        void qc.cancelQueries({
          queryKey: CONSOLE_CONFIG_QUERY_KEY,
          exact: true,
        })
      },
    })
  writerRef.current = writer

  // Shares the traces saved-views cache entry; refetchInterval keeps the
  // strip reactive to writes from other browsers/tabs.
  const { data, isFetched } = useQuery<ConsoleConfigValue | null>({
    queryKey: CONSOLE_CONFIG_QUERY_KEY,
    queryFn: () => writer.readForQuery(),
    staleTime: 3_000,
    refetchInterval: 5_000,
    refetchOnWindowFocus: true,
    retry: 1,
  })
  const available = data !== null && data !== undefined
  const layoutSource = workspaceLayoutSource(isFetched, available)

  const [local, setLocal] = useState<LocalState>(loadLocal)

  // This browser tab's own selection (sessionStorage); see resolvePointer.
  const [localChoice, setLocalChoice] = useState<LocalActivation | null>(
    loadSessionActivation,
  )
  const choose = useCallback((id: string) => {
    const activation = { tabId: id, at: Date.now() }
    persistSessionActivation(activation)
    setLocalChoice(activation)
  }, [])

  const serverTabs = available ? parseWorkspaceTabs(data) : []
  const tabs = available
    ? serverTabs.length > 0
      ? serverTabs
      : defaultTabs()
    : local.tabs

  const serverActivation = available ? parseActivation(data) : undefined
  const pointer = available
    ? resolvePointer(localChoice, serverActivation)
    : (localChoice?.tabId ?? local.activeTabId)
  // No pointer (or one at a tab another browser closed) lands on the
  // chat+traces tab, falling back to the first tab.
  const activeTab = resolveActiveTab(tabs, pointer)
  const activeTabId = activeTab.id

  // A write fired before the first server answer arrives (a keyboard
  // shortcut, a worker's panel-open) must not be lost when `tabs` flips from
  // the local copy to the server layout. Pre-hydration transforms are
  // composed and replayed through the server path once hydrated — composed,
  // because each is a delta rather than a full snapshot.
  const pendingWriteRef = useRef<WorkspaceTransform | null>(null)

  const persist = useCallback(
    (update: WorkspaceTransform) => {
      if (layoutSource === 'pending') {
        const previous = pendingWriteRef.current
        pendingWriteRef.current = previous
          ? (state) => update(previous(state))
          : update
        setLocal((current) => update(current))
        return
      }
      if (available) {
        // Optimistic: the strip reflects the change in this frame; the
        // serialized server writes rebase it behind the UI.
        const optimistic = writer.enqueue(
          workspaceConfigTransform(update),
          data ?? {},
        )
        qc.setQueryData(CONSOLE_CONFIG_QUERY_KEY, optimistic)
      } else {
        setLocal((current) => {
          const next = update(current)
          persistLocal(next)
          return next
        })
      }
    },
    [available, data, layoutSource, qc, writer],
  )

  useEffect(() => {
    const pending = pendingWriteRef.current
    if (!shouldFlushPendingWrite(layoutSource, pending !== null)) return
    pendingWriteRef.current = null
    if (pending !== null) persist(pending)
  }, [layoutSource, persist])

  const activateTab = useCallback(
    (id: string) => {
      choose(id)
      persist((state) =>
        state.tabs.some((tab) => tab.id === id)
          ? { ...state, activeTabId: id }
          : state,
      )
    },
    [choose, persist],
  )

  const activateAdjacent = useCallback(
    (steps: 1 | -1) => {
      const id = adjacentTabId(tabs, activeTabId, steps)
      if (id) activateTab(id)
    },
    [activateTab, activeTabId, tabs],
  )

  const createTab = useCallback(
    (opts: { columns: 1 | 2; screens?: TabScreen[] }) => {
      const screens = (opts.screens ?? []).slice(0, opts.columns)
      const tab: WorkspaceTab = {
        id: newTabId(),
        columns: opts.columns,
        screens,
      }
      choose(tab.id)
      persist((state) => ({
        tabs: state.tabs.some((existing) => existing.id === tab.id)
          ? state.tabs
          : [...state.tabs, tab],
        activeTabId: tab.id,
      }))
    },
    [choose, persist],
  )

  const closeTab = useCallback(
    (id: string) => {
      if (tabs.length <= 1) return
      const preview = withTabClosed({ tabs, activeTabId }, id)
      if (preview.activeTabId !== activeTabId) choose(preview.activeTabId)
      persist((state) => withTabClosed(state, id))
    },
    [choose, persist, tabs, activeTabId],
  )

  const renameTab = useCallback(
    (id: string, name: string) => {
      const trimmed = name.trim()
      persist((state) => ({
        ...state,
        tabs: state.tabs.map((tab) => {
          if (tab.id !== id) return tab
          const { name: _prev, ...rest } = tab
          return trimmed ? { ...rest, name: trimmed } : rest
        }),
      }))
    },
    [persist],
  )

  const reorderTab = useCallback(
    (from: number, to: number) => {
      if (from === to) return
      const movingId = tabs[from]?.id
      const targetId = tabs[to]?.id
      if (!movingId || !targetId) return

      persist((state) => {
        const currentFrom = state.tabs.findIndex((tab) => tab.id === movingId)
        const currentTo = state.tabs.findIndex((tab) => tab.id === targetId)
        if (currentFrom < 0 || currentTo < 0 || currentFrom === currentTo) {
          return state
        }
        return {
          ...state,
          tabs: moveItem(state.tabs, currentFrom, currentTo),
        }
      })
    },
    [persist, tabs],
  )

  const attachScreen = useCallback(
    (id: string, column: number, screen: TabScreen) => {
      persist((state) => ({
        ...state,
        tabs: state.tabs.map((tab) => {
          if (tab.id !== id) return tab
          const columns = tabColumns(tab)
          const screens: (TabScreen | null)[] = Array.from(
            { length: columns },
            (_, i) => tab.screens[i] ?? null,
          )
          if (column < 0 || column >= columns) return tab
          screens[column] = screen
          return { ...tab, columns, screens }
        }),
      }))
    },
    [persist],
  )

  const detachScreen = useCallback(
    (id: string, column: number) => {
      persist((state) => ({
        ...state,
        tabs: state.tabs.map((tab) =>
          tab.id === id ? withScreenDetached(tab, column) : tab,
        ),
      }))
    },
    [persist],
  )

  const addColumn = useCallback(
    (id: string, side: 'left' | 'right') => {
      const stablePaneId = { current: null as string | null }
      persist((state) => ({
        ...state,
        tabs: state.tabs.map((tab) => {
          if (tab.id !== id) return tab
          const next = withColumnAdded(tab, side)
          if (next === tab) return tab
          const paneIds = tabPaneIds(next)
          const addedIndex = side === 'left' ? 0 : paneIds.length - 1
          stablePaneId.current ??= paneIds[addedIndex]
          paneIds[addedIndex] = stablePaneId.current
          return { ...next, paneIds }
        }),
      }))
    },
    [persist],
  )

  const removeColumn = useCallback(
    (id: string, paneId: string) => {
      persist((state) => ({
        ...state,
        tabs: state.tabs.map((tab) => {
          if (tab.id !== id) return tab
          return withPaneRemoved(tab, paneId)
        }),
      }))
    },
    [persist],
  )

  const resizeColumns = useCallback(
    (id: string, sizes: number[]) => {
      persist((state) => ({
        ...state,
        tabs: state.tabs.map((tab) =>
          tab.id === id && sizes.length === tabColumns(tab)
            ? { ...tab, sizes }
            : tab,
        ),
      }))
    },
    [persist],
  )

  const openScreen = useCallback(
    (screen: TabScreen) => {
      const newScreenTabId = newTabId()
      const stablePaneId = { current: null as string | null }
      const update: WorkspaceTransform = (state) => {
        const next = withWorkspaceScreenOpened(
          state.tabs,
          state.activeTabId,
          screen,
          () => newScreenTabId,
        )
        return {
          ...next,
          tabs: stabilizeAddedPane(state.tabs, next.tabs, stablePaneId),
        }
      }
      const preview = update({ tabs, activeTabId })
      if (preview.activeTabId !== activeTabId) choose(preview.activeTabId)
      persist(update)
    },
    [activeTabId, choose, persist, tabs],
  )

  const openScreenInTab = useCallback(
    (id: string, screen: TabScreen) => {
      const newScreenTabId = newTabId()
      const stablePaneId = { current: null as string | null }
      persist((state) => {
        const requestedTabExists = state.tabs.some((tab) => tab.id === id)
        const next = withWorkspaceScreenOpened(
          state.tabs,
          requestedTabExists ? id : state.activeTabId,
          screen,
          () => newScreenTabId,
        )
        return {
          tabs: stabilizeAddedPane(state.tabs, next.tabs, stablePaneId),
          activeTabId: requestedTabExists
            ? state.activeTabId
            : next.activeTabId,
        }
      })
    },
    [persist],
  )

  return {
    layoutSource,
    tabs,
    activeTabId,
    activeTab,
    activateTab,
    createTab,
    closeTab,
    renameTab,
    activateAdjacent,
    reorderTab,
    attachScreen,
    detachScreen,
    addColumn,
    removeColumn,
    resizeColumns,
    openScreen,
    openScreenInTab,
  }
}

export const MAX_TERMINAL_PANES_PER_TAB = 4
export const MAX_TERMINAL_SESSIONS = 16

export interface TerminalPaneState {
  id: string
  cwd: string
}

export interface TerminalTabState {
  id: string
  title: string
  layout: TerminalLayoutNode
}

export interface TerminalWorkspaceState {
  tabs: TerminalTabState[]
  panes: Record<string, TerminalPaneState>
  activeTabId: string | null
  focusedPaneId: string | null
}

export type TerminalLayoutNode =
  | { type: 'pane'; paneId: string }
  | {
      type: 'split'
      id: string
      direction: 'horizontal' | 'vertical'
      ratio: number
      first: TerminalLayoutNode
      second: TerminalLayoutNode
    }

export type TerminalWorkspaceAction =
  | { type: 'workspace-restored'; state: TerminalWorkspaceState }
  | { type: 'tab-created'; tabId: string; paneId: string; root: string }
  | { type: 'tab-selected'; tabId: string }
  | { type: 'tab-renamed'; tabId: string; title: string }
  | { type: 'tab-closed'; tabId: string }
  | {
      type: 'pane-split'
      paneId: string
      newPaneId: string
      splitId: string
      direction: 'horizontal' | 'vertical'
    }
  | { type: 'pane-focused'; paneId: string }
  | { type: 'pane-closed'; paneId: string }
  | { type: 'split-resized'; splitId: string; ratio: number }

const DEFAULT_SPLIT_RATIO = 0.5
const MIN_SPLIT_RATIO = 0.2
const MAX_SPLIT_RATIO = 0.8

function isNonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0
}

export function clampSplitRatio(ratio: number): number {
  return Math.min(MAX_SPLIT_RATIO, Math.max(MIN_SPLIT_RATIO, ratio))
}

export function countTerminalPanes(layout: TerminalLayoutNode): number {
  if (layout.type === 'pane') return 1
  return countTerminalPanes(layout.first) + countTerminalPanes(layout.second)
}

function findTabIndexById(
  state: TerminalWorkspaceState,
  tabId: string,
): number {
  return state.tabs.findIndex((tab) => tab.id === tabId)
}

function findTabByPaneId(
  state: TerminalWorkspaceState,
  paneId: string,
): TerminalTabState | undefined {
  return state.tabs.find((tab) => paneIdsInLayout(tab.layout).includes(paneId))
}

function paneIdsInLayout(layout: TerminalLayoutNode): string[] {
  if (layout.type === 'pane') return [layout.paneId]
  return [...paneIdsInLayout(layout.first), ...paneIdsInLayout(layout.second)]
}

function splitIdsInLayout(layout: TerminalLayoutNode): string[] {
  if (layout.type === 'pane') return []
  return [
    layout.id,
    ...splitIdsInLayout(layout.first),
    ...splitIdsInLayout(layout.second),
  ]
}

function firstPaneIdInLayout(layout: TerminalLayoutNode): string {
  if (layout.type === 'pane') return layout.paneId
  return firstPaneIdInLayout(layout.first)
}

function replacePaneInLayout(
  layout: TerminalLayoutNode,
  paneId: string,
  replacement: TerminalLayoutNode,
): TerminalLayoutNode {
  if (layout.type === 'pane') {
    return layout.paneId === paneId ? replacement : layout
  }
  const first = replacePaneInLayout(layout.first, paneId, replacement)
  const second = replacePaneInLayout(layout.second, paneId, replacement)
  if (first === layout.first && second === layout.second) return layout
  return { ...layout, first, second }
}

function removePaneFromLayout(
  layout: TerminalLayoutNode,
  paneId: string,
): TerminalLayoutNode | null {
  if (layout.type === 'pane') {
    return layout.paneId === paneId ? null : layout
  }
  const first = removePaneFromLayout(layout.first, paneId)
  const second = removePaneFromLayout(layout.second, paneId)
  if (first === null) return second
  if (second === null) return first
  return { ...layout, first, second }
}

function resizeSplitInLayout(
  layout: TerminalLayoutNode,
  splitId: string,
  ratio: number,
): TerminalLayoutNode {
  if (layout.type === 'pane') return layout
  if (layout.id === splitId) {
    return { ...layout, ratio: clampSplitRatio(ratio) }
  }
  return {
    ...layout,
    first: resizeSplitInLayout(layout.first, splitId, ratio),
    second: resizeSplitInLayout(layout.second, splitId, ratio),
  }
}

function nearestTabIdAfterClose(
  tabs: TerminalTabState[],
  closedIndex: number,
): string | null {
  if (tabs.length === 0) return null
  const right = tabs[closedIndex]
  if (right) return right.id
  return tabs[closedIndex - 1]?.id ?? tabs[0]?.id ?? null
}

function tabTitleForIndex(index: number): string {
  return `zsh ${index + 1}`
}

export function createTerminalWorkspace(root: string): TerminalWorkspaceState {
  const tabId = 'tab-1'
  const paneId = 'pane-1'
  return {
    tabs: [
      {
        id: tabId,
        title: tabTitleForIndex(0),
        layout: { type: 'pane', paneId },
      },
    ],
    panes: {
      [paneId]: { id: paneId, cwd: root },
    },
    activeTabId: tabId,
    focusedPaneId: paneId,
  }
}

function isValidLayoutNode(
  node: unknown,
  paneIds: Set<string>,
): node is TerminalLayoutNode {
  if (!node || typeof node !== 'object') return false
  const raw = node as Record<string, unknown>
  if (raw.type === 'pane') {
    return isNonEmptyString(raw.paneId) && paneIds.has(raw.paneId)
  }
  if (raw.type !== 'split') return false
  if (
    !isNonEmptyString(raw.id) ||
    (raw.direction !== 'horizontal' && raw.direction !== 'vertical') ||
    typeof raw.ratio !== 'number'
  ) {
    return false
  }
  return (
    isValidLayoutNode(raw.first, paneIds) &&
    isValidLayoutNode(raw.second, paneIds)
  )
}

export function normalizeTerminalWorkspace(
  value: unknown,
  root: string,
): TerminalWorkspaceState {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return createTerminalWorkspace(root)
  }
  const raw = value as Record<string, unknown>
  if (!Array.isArray(raw.tabs) || raw.tabs.length === 0) {
    return createTerminalWorkspace(root)
  }

  const panes: Record<string, TerminalPaneState> = {}
  for (const [paneId, paneValue] of Object.entries(raw.panes ?? {})) {
    if (!isNonEmptyString(paneId)) continue
    if (
      !paneValue ||
      typeof paneValue !== 'object' ||
      Array.isArray(paneValue)
    ) {
      continue
    }
    const paneRaw = paneValue as Record<string, unknown>
    const cwd =
      typeof paneRaw.cwd === 'string' && paneRaw.cwd.length > 0
        ? paneRaw.cwd
        : root
    panes[paneId] = { id: paneId, cwd }
  }

  const paneIds = new Set(Object.keys(panes))
  const usedPaneIds = new Set<string>()
  const usedSplitIds = new Set<string>()
  const tabs: TerminalTabState[] = []
  for (const tabValue of raw.tabs) {
    if (!tabValue || typeof tabValue !== 'object' || Array.isArray(tabValue)) {
      continue
    }
    const tabRaw = tabValue as Record<string, unknown>
    if (!isNonEmptyString(tabRaw.id) || !isNonEmptyString(tabRaw.title)) {
      continue
    }
    if (!isValidLayoutNode(tabRaw.layout, paneIds)) continue
    if (countTerminalPanes(tabRaw.layout) > MAX_TERMINAL_PANES_PER_TAB) continue
    const layoutPaneIds = paneIdsInLayout(tabRaw.layout)
    const layoutSplitIds = splitIdsInLayout(tabRaw.layout)
    if (new Set(layoutPaneIds).size !== layoutPaneIds.length) continue
    if (new Set(layoutSplitIds).size !== layoutSplitIds.length) continue
    if (layoutPaneIds.some((paneId) => usedPaneIds.has(paneId))) continue
    if (layoutSplitIds.some((splitId) => usedSplitIds.has(splitId))) continue
    for (const paneId of layoutPaneIds) usedPaneIds.add(paneId)
    for (const splitId of layoutSplitIds) usedSplitIds.add(splitId)
    tabs.push({
      id: tabRaw.id,
      title: tabRaw.title,
      layout: normalizeLayoutRatios(tabRaw.layout),
    })
  }

  const referencedPanes = Object.fromEntries(
    [...usedPaneIds].map((paneId) => [paneId, panes[paneId]]),
  )
  if (
    tabs.length === 0 ||
    Object.keys(referencedPanes).length > MAX_TERMINAL_SESSIONS
  ) {
    return createTerminalWorkspace(root)
  }

  const activeTabId =
    isNonEmptyString(raw.activeTabId) &&
    tabs.some((tab) => tab.id === raw.activeTabId)
      ? raw.activeTabId
      : tabs[0].id

  const activeTab = tabs.find((tab) => tab.id === activeTabId) ?? tabs[0]
  const focusedPaneId =
    isNonEmptyString(raw.focusedPaneId) &&
    usedPaneIds.has(raw.focusedPaneId) &&
    paneIdsInLayout(activeTab.layout).includes(raw.focusedPaneId)
      ? raw.focusedPaneId
      : firstPaneIdInLayout(activeTab.layout)

  return {
    tabs,
    panes: referencedPanes,
    activeTabId,
    focusedPaneId,
  }
}

function normalizeLayoutRatios(layout: TerminalLayoutNode): TerminalLayoutNode {
  if (layout.type === 'pane') return layout
  return {
    ...layout,
    ratio: clampSplitRatio(layout.ratio),
    first: normalizeLayoutRatios(layout.first),
    second: normalizeLayoutRatios(layout.second),
  }
}

export function reduceTerminalWorkspace(
  state: TerminalWorkspaceState,
  action: TerminalWorkspaceAction,
): TerminalWorkspaceState {
  switch (action.type) {
    case 'workspace-restored':
      return action.state
    case 'tab-created': {
      const tabIndex = findTabIndexById(state, action.tabId)
      if (tabIndex >= 0) return state
      if (action.paneId in state.panes) return state
      if (Object.keys(state.panes).length >= MAX_TERMINAL_SESSIONS) return state
      return {
        ...state,
        tabs: [
          ...state.tabs,
          {
            id: action.tabId,
            title: tabTitleForIndex(state.tabs.length),
            layout: { type: 'pane', paneId: action.paneId },
          },
        ],
        panes: {
          ...state.panes,
          [action.paneId]: { id: action.paneId, cwd: action.root },
        },
        activeTabId: action.tabId,
        focusedPaneId: action.paneId,
      }
    }
    case 'tab-selected': {
      const tab = state.tabs.find((entry) => entry.id === action.tabId)
      if (!tab) return state
      return {
        ...state,
        activeTabId: tab.id,
        focusedPaneId: firstPaneIdInLayout(tab.layout),
      }
    }
    case 'tab-renamed': {
      const tabIndex = findTabIndexById(state, action.tabId)
      if (tabIndex < 0) return state
      const tabs = [...state.tabs]
      tabs[tabIndex] = { ...tabs[tabIndex], title: action.title }
      return { ...state, tabs }
    }
    case 'tab-closed': {
      const tabIndex = findTabIndexById(state, action.tabId)
      if (tabIndex < 0) return state
      const closingTab = state.tabs[tabIndex]
      const removedPaneIds = paneIdsInLayout(closingTab.layout)
      const panes = { ...state.panes }
      for (const paneId of removedPaneIds) {
        delete panes[paneId]
      }
      const tabs = state.tabs.filter((tab) => tab.id !== action.tabId)
      if (tabs.length === 0) {
        return {
          tabs: [],
          panes: {},
          activeTabId: null,
          focusedPaneId: null,
        }
      }
      const activeTabId =
        state.activeTabId === action.tabId
          ? nearestTabIdAfterClose(tabs, tabIndex)
          : state.activeTabId
      const activeTab = tabs.find((tab) => tab.id === activeTabId) ?? tabs[0]
      let focusedPaneId = state.focusedPaneId
      if (
        !focusedPaneId ||
        removedPaneIds.includes(focusedPaneId) ||
        !tabs.some((tab) =>
          paneIdsInLayout(tab.layout).includes(focusedPaneId as string),
        )
      ) {
        focusedPaneId = firstPaneIdInLayout(activeTab.layout)
      }
      return {
        tabs,
        panes,
        activeTabId: activeTab.id,
        focusedPaneId,
      }
    }
    case 'pane-split': {
      const tab = findTabByPaneId(state, action.paneId)
      if (!tab) return state
      if (action.newPaneId in state.panes) return state
      if (
        state.tabs.some((entry) =>
          splitIdsInLayout(entry.layout).includes(action.splitId),
        )
      ) {
        return state
      }
      if (countTerminalPanes(tab.layout) >= MAX_TERMINAL_PANES_PER_TAB)
        return state
      if (Object.keys(state.panes).length >= MAX_TERMINAL_SESSIONS) return state
      const sourcePane = state.panes[action.paneId]
      if (!sourcePane) return state
      const replacement: TerminalLayoutNode = {
        type: 'split',
        id: action.splitId,
        direction: action.direction,
        ratio: DEFAULT_SPLIT_RATIO,
        first: { type: 'pane', paneId: action.paneId },
        second: { type: 'pane', paneId: action.newPaneId },
      }
      const layout = replacePaneInLayout(tab.layout, action.paneId, replacement)
      const tabs = state.tabs.map((entry) =>
        entry.id === tab.id ? { ...entry, layout } : entry,
      )
      return {
        ...state,
        tabs,
        panes: {
          ...state.panes,
          [action.newPaneId]: {
            id: action.newPaneId,
            cwd: sourcePane.cwd,
          },
        },
        focusedPaneId: action.newPaneId,
      }
    }
    case 'pane-focused': {
      const tab = findTabByPaneId(state, action.paneId)
      if (!tab) return state
      return {
        ...state,
        activeTabId: tab.id,
        focusedPaneId: action.paneId,
      }
    }
    case 'pane-closed': {
      const tab = findTabByPaneId(state, action.paneId)
      if (!tab) return state
      if (countTerminalPanes(tab.layout) === 1) {
        return reduceTerminalWorkspace(state, {
          type: 'tab-closed',
          tabId: tab.id,
        })
      }
      const layout = removePaneFromLayout(tab.layout, action.paneId)
      if (!layout) return state
      const panes = { ...state.panes }
      delete panes[action.paneId]
      const tabs = state.tabs.map((entry) =>
        entry.id === tab.id ? { ...entry, layout } : entry,
      )
      const focusedPaneId =
        state.focusedPaneId === action.paneId
          ? firstPaneIdInLayout(layout)
          : state.focusedPaneId
      return {
        ...state,
        tabs,
        panes,
        focusedPaneId,
      }
    }
    case 'split-resized': {
      const tabs = state.tabs.map((tab) => ({
        ...tab,
        layout: resizeSplitInLayout(tab.layout, action.splitId, action.ratio),
      }))
      return { ...state, tabs }
    }
    default: {
      const _exhaustive: never = action
      return _exhaustive
    }
  }
}

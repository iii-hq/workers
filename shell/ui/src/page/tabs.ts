/* Editor tab-strip state — VS Code-style preview semantics, as pure
   transitions so the behavior is unit-testable. A tab shows either a
   file (its real content, editable) or a diff (one source against one
   path); both live in the same strip and follow the same rules:

   - single click opens a PREVIEW tab (italic): at most one exists, and
     the next preview replaces it
   - double click (or editing the file) PINS the tab — pinned tabs only
     close explicitly
*/

import { type DiffSource, diffSourceKey, diffSourcePersists, parseDiffSource } from './diff-source'

export type TabTarget =
  | { kind: 'file'; path: string }
  | { kind: 'diff'; path: string; source: DiffSource }

export interface OpenTab {
  /** Stable identity: `file:<path>` or `diff:<source>:<path>`. */
  id: string
  target: TabTarget
  pinned: boolean
}

export interface TabsState {
  tabs: OpenTab[]
  /** Id of the active tab; null = nothing open. */
  active: string | null
}

export const EMPTY_TABS: TabsState = { tabs: [], active: null }

export function tabIdFor(target: TabTarget): string {
  return target.kind === 'file'
    ? `file:${target.path}`
    : `diff:${diffSourceKey(target.source)}:${target.path}`
}

export function fileTarget(path: string): TabTarget {
  return { kind: 'file', path }
}

export function diffTarget(path: string, source: DiffSource): TabTarget {
  return { kind: 'diff', path, source }
}

export function findTab(state: TabsState, id: string): OpenTab | undefined {
  return state.tabs.find((tab) => tab.id === id)
}

export function activeTab(state: TabsState): OpenTab | null {
  return state.active === null ? null : (findTab(state, state.active) ?? null)
}

/** Single click: activate if open, else open as the (single) preview tab —
    replacing the current preview in place, never touching pinned tabs. */
export function openPreview(state: TabsState, target: TabTarget): TabsState {
  const id = tabIdFor(target)
  if (findTab(state, id)) return { ...state, active: id }
  const previewIndex = state.tabs.findIndex((t) => !t.pinned)
  const tabs = [...state.tabs]
  const tab: OpenTab = { id, target, pinned: false }
  if (previewIndex === -1) tabs.push(tab)
  else tabs[previewIndex] = tab
  return { tabs, active: id }
}

/** Double click: open pinned — promotes an existing tab (preview or not). */
export function openPinned(state: TabsState, target: TabTarget): TabsState {
  const id = tabIdFor(target)
  if (findTab(state, id)) {
    return {
      tabs: state.tabs.map((t) => (t.id === id ? { ...t, pinned: true } : t)),
      active: id,
    }
  }
  return { tabs: [...state.tabs, { id, target, pinned: true }], active: id }
}

/** Pin in place (tab double-click, or the file became dirty). */
export function pinTab(state: TabsState, id: string): TabsState {
  if (!state.tabs.some((t) => t.id === id && !t.pinned)) return state
  return { ...state, tabs: state.tabs.map((t) => (t.id === id ? { ...t, pinned: true } : t)) }
}

/** Close: the neighbor (right, else left) becomes active. */
export function closeTab(state: TabsState, id: string): TabsState {
  const index = state.tabs.findIndex((t) => t.id === id)
  if (index === -1) return state
  const tabs = state.tabs.filter((t) => t.id !== id)
  let active = state.active
  if (state.active === id) {
    const neighbor = tabs[index] ?? tabs[index - 1]
    active = neighbor ? neighbor.id : null
  }
  return { tabs, active }
}

export function activateTab(state: TabsState, id: string): TabsState {
  if (!state.tabs.some((t) => t.id === id)) return state
  return { ...state, active: id }
}

/** The neighbour in strip order, wrapping at either end. */
export function cycleTab(state: TabsState, delta: 1 | -1): TabsState {
  if (state.tabs.length < 2 || state.active === null) return state
  const index = state.tabs.findIndex((t) => t.id === state.active)
  if (index === -1) return state
  const next = state.tabs[(index + delta + state.tabs.length) % state.tabs.length]
  return { ...state, active: next.id }
}

/** Every open tab that shows `path` in some form (file, diffs). */
export function tabsForPath(state: TabsState, path: string): OpenTab[] {
  return state.tabs.filter((t) => t.target.path === path)
}

/** The file tab's id for a path, whether or not it is open. */
export function fileTabId(path: string): string {
  return tabIdFor(fileTarget(path))
}

/** Restore from persisted state, dropping malformed entries. The shape
    before diff tabs was `{ path, pinned }`; those still open as files. */
export function restoreTabs(open: unknown, active: unknown): TabsState {
  const tabs: OpenTab[] = []
  if (Array.isArray(open)) {
    for (const entry of open) {
      if (!entry || typeof entry !== 'object') continue
      const raw = entry as Record<string, unknown>
      const target = restoreTarget(raw)
      if (target === null) continue
      const id = tabIdFor(target)
      if (tabs.some((t) => t.id === id)) continue
      tabs.push({ id, target, pinned: raw.pinned === true })
    }
  }
  const activeId =
    typeof active === 'string'
      ? (tabs.find((t) => t.id === active) ?? tabs.find((t) => t.target.kind === 'file' && t.target.path === active))?.id ?? null
      : null
  return { tabs, active: activeId ?? tabs[0]?.id ?? null }
}

function restoreTarget(raw: Record<string, unknown>): TabTarget | null {
  const path = typeof raw.path === 'string' ? raw.path : null
  if (path === null || path === '') return null
  if (raw.kind === 'diff') {
    const source = parseDiffSource(raw.source)
    return source === null ? null : diffTarget(path, source)
  }
  return fileTarget(path)
}

/** The persisted form: one row per tab, change tabs left out. */
export function persistedTabs(state: TabsState): { kind: string; path: string; source?: DiffSource; pinned: boolean }[] {
  return state.tabs
    .filter((t) => t.target.kind === 'file' || diffSourcePersists(t.target.source))
    .map((t) =>
      t.target.kind === 'file'
        ? { kind: 'file', path: t.target.path, pinned: t.pinned }
        : { kind: 'diff', path: t.target.path, source: t.target.source, pinned: t.pinned },
    )
}

/** `/a/b/c/d` → `c/d` — the short display form for a project root. */
export function lastSegments(path: string, count = 2): string {
  const segments = path.split('/').filter((s) => s !== '')
  if (segments.length === 0) return path
  return segments.slice(-count).join('/')
}

export function basename(path: string): string {
  const idx = path.lastIndexOf('/')
  return idx === -1 ? path : path.slice(idx + 1)
}

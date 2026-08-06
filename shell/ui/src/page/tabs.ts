/* Editor tab-strip state — VS Code-style preview semantics, as pure
   transitions so the behavior is unit-testable:

   - single click opens a PREVIEW tab (italic): at most one exists, and
     the next preview replaces it
   - double click (or editing the file) PINS the tab — pinned tabs only
     close explicitly
*/

export interface OpenTab {
  path: string
  pinned: boolean
}

export interface TabsState {
  tabs: OpenTab[]
  /** Path of the active tab; null = nothing open. */
  active: string | null
}

export const EMPTY_TABS: TabsState = { tabs: [], active: null }

/** Single click: activate if open, else open as the (single) preview tab —
    replacing the current preview in place, never touching pinned tabs. */
export function openPreview(state: TabsState, path: string): TabsState {
  const existing = state.tabs.find((t) => t.path === path)
  if (existing) return { ...state, active: path }
  const previewIndex = state.tabs.findIndex((t) => !t.pinned)
  const tabs = [...state.tabs]
  if (previewIndex === -1) {
    tabs.push({ path, pinned: false })
  } else {
    tabs[previewIndex] = { path, pinned: false }
  }
  return { tabs, active: path }
}

/** Double click: open pinned — promotes an existing tab (preview or not). */
export function openPinned(state: TabsState, path: string): TabsState {
  const existing = state.tabs.find((t) => t.path === path)
  if (existing) {
    return {
      tabs: state.tabs.map((t) =>
        t.path === path ? { ...t, pinned: true } : t,
      ),
      active: path,
    }
  }
  return {
    tabs: [...state.tabs, { path, pinned: true }],
    active: path,
  }
}

/** Pin in place (tab double-click, or the file became dirty). */
export function pinTab(state: TabsState, path: string): TabsState {
  if (!state.tabs.some((t) => t.path === path && !t.pinned)) return state
  return {
    ...state,
    tabs: state.tabs.map((t) => (t.path === path ? { ...t, pinned: true } : t)),
  }
}

/** Close: the neighbor (right, else left) becomes active. */
export function closeTab(state: TabsState, path: string): TabsState {
  const index = state.tabs.findIndex((t) => t.path === path)
  if (index === -1) return state
  const tabs = state.tabs.filter((t) => t.path !== path)
  let active = state.active
  if (state.active === path) {
    const neighbor = tabs[index] ?? tabs[index - 1]
    active = neighbor ? neighbor.path : null
  }
  return { tabs, active }
}

export function activateTab(state: TabsState, path: string): TabsState {
  if (!state.tabs.some((t) => t.path === path)) return state
  return { ...state, active: path }
}

/** Restore from persisted state, dropping malformed entries. */
export function restoreTabs(
  open: unknown,
  active: unknown,
): TabsState {
  const tabs: OpenTab[] = []
  if (Array.isArray(open)) {
    for (const entry of open) {
      if (!entry || typeof entry !== 'object') continue
      const { path, pinned } = entry as Record<string, unknown>
      if (typeof path !== 'string' || path === '') continue
      if (tabs.some((t) => t.path === path)) continue
      tabs.push({ path, pinned: pinned === true })
    }
  }
  const activePath =
    typeof active === 'string' && tabs.some((t) => t.path === active)
      ? active
      : (tabs[0]?.path ?? null)
  return { tabs, active: activePath }
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

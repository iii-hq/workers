/* What a pane remembers per folder: the tabs that were open there and the
   folders that were expanded, so switching away and back finds them
   again. A pure value the page keeps in a ref and persists with the rest
   of its state; insertion order is recency, and the oldest folders fall
   off past `MAX_REMEMBERED_ROOTS`. */

export interface RootUiState {
  /** Open tabs in their persisted form (`persistedTabs`). */
  open: unknown[]
  active: string | null
  expanded: string[]
}

export type RootMemory = ReadonlyMap<string, RootUiState>

export const EMPTY_ROOT_MEMORY: RootMemory = new Map()

export const MAX_REMEMBERED_ROOTS = 16

function isEmpty(state: RootUiState): boolean {
  return state.open.length === 0 && state.expanded.length === 0
}

/** Record the folder's state as the most recent entry; a folder with
    nothing open and nothing expanded is forgotten instead. */
export function rememberRoot(memory: RootMemory, root: string, state: RootUiState): RootMemory {
  const next = new Map(memory)
  next.delete(root)
  if (!isEmpty(state)) next.set(root, state)
  while (next.size > MAX_REMEMBERED_ROOTS) {
    const oldest = next.keys().next().value
    if (oldest === undefined) break
    next.delete(oldest)
  }
  return next
}

export function recallRoot(memory: RootMemory, root: string): RootUiState | null {
  return memory.get(root) ?? null
}

function parseRootUiState(raw: unknown): RootUiState | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const value = raw as Record<string, unknown>
  return {
    open: Array.isArray(value.open) ? value.open : [],
    active: typeof value.active === 'string' ? value.active : null,
    expanded: Array.isArray(value.expanded) ? value.expanded.filter((p): p is string => typeof p === 'string') : [],
  }
}

/** From the persisted `{ [root]: state }` object; junk entries are dropped. */
export function parseRootMemory(raw: unknown): RootMemory {
  const memory = new Map<string, RootUiState>()
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return memory
  for (const [root, value] of Object.entries(raw as Record<string, unknown>)) {
    if (root === '') continue
    const state = parseRootUiState(value)
    if (state !== null && !isEmpty(state)) memory.set(root, state)
  }
  return memory
}

export function serializeRootMemory(memory: RootMemory): Record<string, RootUiState> {
  return Object.fromEntries(memory)
}

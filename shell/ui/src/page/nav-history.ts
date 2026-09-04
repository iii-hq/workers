/* Editor navigation history — back/forward across the files (and lines)
   the user landed on, VS Code's Go Back / Go Forward. A pure value: the
   page keeps it in a ref and mirrors `canBack`/`canForward` into state. */

export interface NavLocation {
  path: string
  line?: number
}

export interface NavHistory {
  entries: readonly NavLocation[]
  /** Index of the current location; -1 when empty. */
  index: number
}

export const EMPTY_HISTORY: NavHistory = { entries: [], index: -1 }

const MAX_ENTRIES = 100

function same(a: NavLocation | undefined, b: NavLocation): boolean {
  return a !== undefined && a.path === b.path && (a.line ?? null) === (b.line ?? null)
}

/** Record a new location: everything after the current index is dropped
    (a fresh branch), an identical consecutive entry is not repeated. */
export function pushLocation(history: NavHistory, location: NavLocation): NavHistory {
  const current = history.entries[history.index]
  if (same(current, location)) return history
  const kept = history.entries.slice(0, history.index + 1)
  const entries = [...kept, location].slice(-MAX_ENTRIES)
  return { entries, index: entries.length - 1 }
}

/** Replace the current entry's line without creating a new one (the
    cursor moved inside the same file). */
export function updateCurrentLine(history: NavHistory, line: number): NavHistory {
  const current = history.entries[history.index]
  if (current === undefined || current.line === line) return history
  const entries = [...history.entries]
  entries[history.index] = { ...current, line }
  return { entries, index: history.index }
}

export function canGoBack(history: NavHistory): boolean {
  return history.index > 0
}

export function canGoForward(history: NavHistory): boolean {
  return history.index >= 0 && history.index < history.entries.length - 1
}

export function goBack(history: NavHistory): { history: NavHistory; location: NavLocation | null } {
  if (!canGoBack(history)) return { history, location: null }
  const index = history.index - 1
  return { history: { entries: history.entries, index }, location: history.entries[index] }
}

export function goForward(history: NavHistory): { history: NavHistory; location: NavLocation | null } {
  if (!canGoForward(history)) return { history, location: null }
  const index = history.index + 1
  return { history: { entries: history.entries, index }, location: history.entries[index] }
}

/** Drop every entry for a path that no longer opens (closed tab, deleted
    file), keeping the index on the same logical position. */
export function forgetPath(history: NavHistory, path: string): NavHistory {
  if (!history.entries.some((entry) => entry.path === path)) return history
  const entries: NavLocation[] = []
  let index = -1
  for (let i = 0; i < history.entries.length; i++) {
    const entry = history.entries[i]
    if (entry.path === path) continue
    if (i <= history.index) index = entries.length
    entries.push(entry)
  }
  return { entries, index: Math.min(index, entries.length - 1) }
}

/** The distinct files visited, most recent first — what an empty pane
    offers to reopen. */
export function recentPaths(history: NavHistory, limit: number): string[] {
  const seen = new Set<string>()
  const out: string[] = []
  for (let index = history.entries.length - 1; index >= 0 && out.length < limit; index -= 1) {
    const path = history.entries[index].path
    if (seen.has(path)) continue
    seen.add(path)
    out.push(path)
  }
  return out
}

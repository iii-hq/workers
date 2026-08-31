/**
 * What the palette ran last, so an empty query opens on the things you
 * reach for: kept per browser in localStorage, newest first, ten deep.
 */

const KEY = 'iii-palette-recents'
const LIMIT = 10

export interface RecentEntry {
  id: string
  at: number
}

function storage(): Storage | null {
  return typeof window === 'undefined' ? null : window.localStorage
}

export function loadRecents(): RecentEntry[] {
  try {
    const raw = storage()?.getItem(KEY)
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      (item): item is RecentEntry =>
        typeof item === 'object' &&
        item !== null &&
        typeof (item as RecentEntry).id === 'string' &&
        typeof (item as RecentEntry).at === 'number',
    )
  } catch {
    return []
  }
}

export function recordRecent(id: string, now = Date.now()): RecentEntry[] {
  const next = [
    { id, at: now },
    ...loadRecents().filter((entry) => entry.id !== id),
  ].slice(0, LIMIT)
  try {
    storage()?.setItem(KEY, JSON.stringify(next))
  } catch {
    // Private mode or a full store: recents are a convenience, not state.
  }
  return next
}

/** Rows the palette already has, in recency order, for an empty query. */
export function recentOf<T extends { id: string }>(
  entries: readonly T[],
  recents: readonly RecentEntry[],
): T[] {
  const byId = new Map(entries.map((entry) => [entry.id, entry]))
  return recents.flatMap((recent) => {
    const entry = byId.get(recent.id)
    return entry ? [entry] : []
  })
}

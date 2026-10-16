import { useMemo, useSyncExternalStore } from 'react'
import { CONTEXT_PANEL_GROUPS, useExtContextSections } from '@/lib/ui-slots'

/** The Context panel's own groups, in their default order. */
export const CONTEXT_GROUPS = CONTEXT_PANEL_GROUPS
export type ContextGroup = (typeof CONTEXT_GROUPS)[number]

const GROUP_LABELS: Record<ContextGroup, string> = {
  project: 'Project',
  tickets: 'Tickets',
  prs: 'Pull requests',
  assets: 'Assets',
  subagents: 'Sub-agents',
  triggers: 'Triggers',
  links: 'Links',
}

/** Where a worker's section lands when it asks for no place. */
const WORKER_DEFAULT_ORDER = 1000

/** A section of the panel: one of its own groups or a worker's. */
export interface ContextSectionInfo {
  id: string
  label: string
  /** The worker that adds it; none for the panel's own groups. */
  source?: string
}

/** A list of ids kept in this browser, for every chat. */
function idListStore(key: string) {
  let value: readonly string[] | null = null
  const listeners = new Set<() => void>()
  const read = (): readonly string[] => {
    if (value) return value
    try {
      const stored: unknown = JSON.parse(localStorage.getItem(key) ?? '[]')
      value = Array.isArray(stored)
        ? stored.filter((id): id is string => typeof id === 'string')
        : []
    } catch {
      value = []
    }
    return value
  }
  return {
    read,
    write(next: readonly string[]) {
      value = next
      try {
        localStorage.setItem(key, JSON.stringify(next))
      } catch {
        // Kept for this page only.
      }
      for (const listener of listeners) listener()
    },
    subscribe(listener: () => void) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    reset() {
      value = null
    },
  }
}

const folded = idListStore('iii-context-collapsed')
const order = idListStore('iii-context-order')
const hidden = idListStore('iii-context-hidden')

function useIds(store: ReturnType<typeof idListStore>): ReadonlySet<string> {
  const list = useSyncExternalStore(store.subscribe, store.read, store.read)
  return useMemo(() => new Set(list), [list])
}

export function useFoldedGroups(): ReadonlySet<string> {
  return useIds(folded)
}

export function useHiddenSections(): ReadonlySet<string> {
  return useIds(hidden)
}

export function toggleGroup(id: string) {
  const next = folded.read().filter((x) => x !== id)
  folded.write(next.length < folded.read().length ? next : [...next, id])
}

/** Fold every section in `ids`, or unfold them all. */
export function foldAllGroups(fold: boolean, ids: readonly string[]) {
  folded.write(fold ? [...ids] : [])
}

export function allGroupsFolded(
  set: ReadonlySet<string>,
  ids: readonly string[],
): boolean {
  return ids.every((id) => set.has(id))
}

export function setSectionHidden(id: string, hide: boolean) {
  const next = hidden.read().filter((x) => x !== id)
  hidden.write(hide ? [...next, id] : next)
}

/**
 * `list`, with each id of `all` it lacks put back right after the id it
 * followed in `all` (or first). A worker's section that left keeps its
 * place this way, and a new one lands beside its default neighbour.
 */
export function mergeOrder(
  list: readonly string[],
  all: readonly string[],
): string[] {
  const out = [...list]
  all.forEach((id, index) => {
    if (out.includes(id)) return
    const before = all
      .slice(0, index)
      .reverse()
      .find((other) => out.includes(other))
    out.splice(before === undefined ? 0 : out.indexOf(before) + 1, 0, id)
  })
  return out
}

/**
 * Move `id` before or after another section of `current` (the order on
 * screen) and keep it, with the places of sections not here today.
 */
export function moveSection(
  current: readonly string[],
  id: string,
  target: { before: string } | { after: string },
) {
  const next = current.filter((other) => other !== id)
  const at =
    'before' in target
      ? next.indexOf(target.before)
      : next.indexOf(target.after) + 1
  if (at < 0) return
  next.splice(at, 0, id)
  order.write(mergeOrder(next, order.read()))
}

export function resetSectionOrder() {
  order.write([])
}

/**
 * Every section the panel can show, the panel's own and the workers', in
 * the order this browser keeps. Hidden ones are listed too: the panel
 * leaves them out, the Sections menu shows them unticked.
 */
export function useContextSections(): ContextSectionInfo[] {
  const workers = useExtContextSections()
  const stored = useSyncExternalStore(order.subscribe, order.read, order.read)
  return useMemo(() => {
    const known: (ContextSectionInfo & { rank: number })[] = [
      ...CONTEXT_GROUPS.map((id, index) => ({
        id,
        label: GROUP_LABELS[id],
        rank: (index + 1) * 100,
      })),
      ...workers.map((section) => ({
        id: section.id,
        label: section.title,
        source: section.scope,
        rank: section.order ?? WORKER_DEFAULT_ORDER,
      })),
    ]
    const byRank = [...known].sort((a, b) => a.rank - b.rank)
    const byId = new Map(known.map((section) => [section.id, section]))
    return mergeOrder(
      stored.filter((id) => byId.has(id)),
      byRank.map((section) => section.id),
    ).flatMap((id) => {
      const section = byId.get(id)
      return section
        ? [{ id: section.id, label: section.label, source: section.source }]
        : []
    })
  }, [workers, stored])
}

/** Tests start from a clean slate. */
export function resetFoldedGroups() {
  folded.reset()
  order.reset()
  hidden.reset()
}

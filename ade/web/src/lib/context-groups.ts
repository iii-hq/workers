import { useSyncExternalStore } from 'react'

/** The Context panel's groups, in panel order. */
export const CONTEXT_GROUPS = [
  'project',
  'tickets',
  'prs',
  'assets',
  'subagents',
  'triggers',
  'links',
] as const
export type ContextGroup = (typeof CONTEXT_GROUPS)[number]

/** The folded groups, for every chat in this browser. */
const KEY = 'iii-context-collapsed'

let folded: ReadonlySet<string> | null = null
const listeners = new Set<() => void>()

function read(): ReadonlySet<string> {
  if (folded) return folded
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(KEY) ?? '[]')
    folded = new Set(
      Array.isArray(stored)
        ? stored.filter((id): id is string => typeof id === 'string')
        : [],
    )
  } catch {
    folded = new Set()
  }
  return folded
}

function write(next: ReadonlySet<string>) {
  folded = next
  try {
    localStorage.setItem(KEY, JSON.stringify([...next]))
  } catch {
    // Kept for this page only.
  }
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useFoldedGroups(): ReadonlySet<string> {
  return useSyncExternalStore(subscribe, read, read)
}

export function toggleGroup(id: ContextGroup) {
  const next = new Set(read())
  if (!next.delete(id)) next.add(id)
  write(next)
}

/** Fold every group, or unfold them all. */
export function foldAllGroups(fold: boolean) {
  write(new Set(fold ? CONTEXT_GROUPS : []))
}

export function allGroupsFolded(set: ReadonlySet<string>): boolean {
  return CONTEXT_GROUPS.every((id) => set.has(id))
}

/** Tests start from a clean slate. */
export function resetFoldedGroups() {
  folded = null
}

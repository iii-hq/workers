/**
 * What the command palette can reach.
 *
 * The console's subject is the engine and the workers attached to it, so the
 * palette searches that: every connected worker, every function they register,
 * every page the console can show (first-party and worker-injected), the open
 * conversations, the open workspaces, and every console action the shortcut
 * registry knows, so nothing needs a memorised key to reach.
 *
 * Worker and function rows come from the engine at open time rather than a
 * cached list — a worker that just disconnected should not stay searchable, and
 * a worker that just arrived should be there without a reload.
 */

import type { LucideIcon } from 'lucide-react'
import { getIiiClient } from '@/lib/iii-client'

export type PaletteKind =
  | 'worker'
  | 'function'
  | 'page'
  | 'chat'
  | 'workspace'
  | 'command'
  | 'action'
  | 'file'
  | 'item'

/** A group header: a kind, or the recents the empty query opens on. */
export type PaletteGroup = PaletteKind | 'recent'

/**
 * A leading character picks a mode, the way editors and chat apps do: `>`
 * and `/` commands, `#` files, `@` chats. The rest of the query is the
 * search text.
 */
export const PREFIX_MODES: Record<string, readonly PaletteKind[]> = {
  '>': ['command', 'action'],
  '/': ['command', 'action'],
  '#': ['file'],
  '@': ['chat'],
}

export interface ParsedQuery {
  prefix: string | null
  kinds: ReadonlySet<PaletteKind> | null
  text: string
}

export function parseQuery(
  raw: string,
  filter: PaletteKind | 'all',
): ParsedQuery {
  const first = raw[0] ?? ''
  const mode = PREFIX_MODES[first]
  if (mode) {
    return { prefix: first, kinds: new Set(mode), text: raw.slice(1).trim() }
  }
  return {
    prefix: null,
    kinds: filter === 'all' ? null : new Set([filter]),
    text: raw.trim(),
  }
}

export interface PaletteEntry {
  id: string
  kind: PaletteKind
  /** Primary line: what the user is looking for. */
  title: string
  /** Secondary line: where it is, or what it does. */
  detail?: string
  /** Right-aligned tag: the owning worker, the page's scope, a status. */
  meta?: string
  /** Extra words that should match without being displayed. */
  keywords?: string[]
  /** The keybinding that runs this row, if one is registered for it. Shown
   *  as key caps so the palette teaches the shortcut it duplicates. */
  shortcut?: string
  /** The row's own icon. Pages carry the icon the console already gives them,
   *  so a page looks the same here as it does in the attach menu; anything
   *  without one falls back to its category icon. */
  icon?: LucideIcon
  run: () => void
}

export interface EngineSnapshot {
  workers: Array<{
    name: string
    status: string
    version: string
    functionCount: number
  }>
  functions: Array<{ id: string; worker: string; description: string }>
  error: string | null
}

const EMPTY: EngineSnapshot = { workers: [], functions: [], error: null }

function str(value: unknown, fallback = ''): string {
  return typeof value === 'string' ? value : fallback
}

function reasonText(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason)
}

function num(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

/**
 * Read the engine's own inventory. Both calls are best-effort: a palette that
 * opens with pages and chats beats one that refuses to open because a worker
 * list timed out.
 */
export async function readEngine(): Promise<EngineSnapshot> {
  try {
    const client = await getIiiClient()
    // Settled, not all: the two reads are independent, and a worker list that
    // times out should not also cost you the function list.
    const [workerResult, functionResult] = await Promise.allSettled([
      client.trigger<{ workers?: unknown }>('engine::workers::list', {}),
      client.trigger<{ functions?: unknown }>('engine::functions::list', {}),
    ])
    const failures = [
      workerResult.status === 'rejected'
        ? reasonText(workerResult.reason)
        : null,
      functionResult.status === 'rejected'
        ? reasonText(functionResult.reason)
        : null,
    ].filter((reason): reason is string => reason !== null)

    const workerResponse =
      workerResult.status === 'fulfilled' ? workerResult.value : undefined
    const functionResponse =
      functionResult.status === 'fulfilled' ? functionResult.value : undefined

    const workerRows = Array.isArray(workerResponse?.workers)
      ? workerResponse.workers
      : []
    const workers = workerRows
      .map((raw) => {
        const row = (raw ?? {}) as Record<string, unknown>
        return {
          name: str(row.name),
          status: str(row.status, 'unknown'),
          version: str(row.version),
          functionCount: num(row.function_count),
        }
      })
      .filter((worker) => worker.name !== '')

    const functionRows = Array.isArray(functionResponse?.functions)
      ? functionResponse.functions
      : []
    const functions = functionRows
      .map((raw) => {
        const row = (raw ?? {}) as Record<string, unknown>
        const id = str(row.function_id) || str(row.id)
        return {
          id,
          worker: str(row.worker_name),
          description: str(row.description),
        }
      })
      .filter((fn) => fn.id !== '')

    return {
      workers,
      functions,
      error: failures.length > 0 ? failures.join('; ') : null,
    }
  } catch (error) {
    // Only a client that never connects lands here; the per-read failures
    // above keep their own partial results.
    return { ...EMPTY, error: reasonText(error) }
  }
}

/**
 * Rank matches the way a keyboard-first surface should: an exact prefix beats a
 * word start, which beats a match anywhere. Subsequence matching (`ewl` finding
 * `engine::workers::list`) comes last, because it is the loosest and would
 * otherwise bury the obvious answers.
 */
export function scoreEntry(entry: PaletteEntry, query: string): number {
  if (!query) return 1
  const haystacks = [
    entry.title,
    entry.detail ?? '',
    ...(entry.keywords ?? []),
  ].map((value) => value.toLowerCase())
  // Several words match in any order ("file open" finds "Open file…"); the
  // row scores as its weakest word, so every word has to land somewhere.
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length > 1) {
    let weakest = Number.POSITIVE_INFINITY
    for (const word of words) {
      const score = scoreWord(haystacks, word)
      if (score === 0) return 0
      weakest = Math.min(weakest, score)
    }
    const whole = scoreWord(haystacks, words.join(' '))
    return Math.max(whole, weakest)
  }
  return scoreWord(haystacks, words[0] ?? '')
}

function scoreWord(haystacks: string[], needle: string): number {
  if (!needle) return 1
  let best = 0
  for (const hay of haystacks) {
    if (!hay) continue
    if (hay === needle) best = Math.max(best, 100)
    else if (hay.startsWith(needle)) best = Math.max(best, 80)
    else {
      const at = hay.indexOf(needle)
      if (at === 0) best = Math.max(best, 80)
      else if (at > 0)
        best = Math.max(
          best,
          hay[at - 1] === ' ' || hay[at - 1] === ':' ? 60 : 40,
        )
    }
  }
  if (best > 0) return best

  // Subsequence: every needle character appears in order.
  const title = haystacks[0]
  let cursor = 0
  for (const char of needle) {
    cursor = title.indexOf(char, cursor)
    if (cursor === -1) return 0
    cursor += 1
  }
  return 20
}

export function filterEntries(
  entries: PaletteEntry[],
  query: string,
  kind: PaletteKind | 'all' | ReadonlySet<PaletteKind> | null,
): PaletteEntry[] {
  const scoped =
    kind === 'all' || kind === null
      ? entries
      : typeof kind === 'string'
        ? entries.filter((entry) => entry.kind === kind)
        : entries.filter((entry) => kind.has(entry.kind))
  if (!query.trim()) return scoped
  return scoped
    .map((entry) => ({ entry, score: scoreEntry(entry, query.trim()) }))
    .filter((row) => row.score > 0)
    .sort((a, b) => b.score - a.score)
    .map((row) => row.entry)
}

export const KIND_LABEL: Record<PaletteGroup, string> = {
  recent: 'Recent',
  worker: 'Workers',
  function: 'Functions',
  page: 'Pages',
  chat: 'Chats',
  workspace: 'Workspaces',
  command: 'Commands',
  action: 'Actions',
  file: 'Files',
  item: 'Results',
}

/** Group in a fixed order so the list does not reshuffle as scores change. */
export const KIND_ORDER: PaletteKind[] = [
  'file',
  'command',
  'action',
  'workspace',
  'page',
  'chat',
  'item',
  'worker',
  'function',
]

/** Group in a fixed order so the list does not reshuffle as scores change;
    `recent` rows, when given, lead and are not repeated below. */
export function groupEntries(
  entries: PaletteEntry[],
  recent: PaletteEntry[] = [],
): Array<[PaletteGroup, PaletteEntry[]]> {
  const recentIds = new Set(recent.map((entry) => entry.id))
  const groups = new Map<PaletteKind, PaletteEntry[]>()
  for (const entry of entries) {
    if (recentIds.has(entry.id)) continue
    const bucket = groups.get(entry.kind)
    if (bucket) bucket.push(entry)
    else groups.set(entry.kind, [entry])
  }
  const ordered: Array<[PaletteGroup, PaletteEntry[]]> = KIND_ORDER.filter(
    (kind) => groups.has(kind),
  ).map((kind) => [kind, groups.get(kind) as PaletteEntry[]])
  return recent.length > 0 ? [['recent', recent], ...ordered] : ordered
}

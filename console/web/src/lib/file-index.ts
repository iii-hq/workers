/**
 * Working-directory file index for `#` file mentions in the composer.
 *
 * Fetches a bounded `coder::tree` snapshot scoped to the conversation's
 * working directory (`fs_scope.root`), flattens it to relative file paths,
 * and caches per working dir with a short TTL so reopening the typeahead
 * doesn't re-walk the repo. The shell worker jail-validates the scope on
 * every call, so this surface carries the same trust as the DirectoryPicker
 * flow.
 */

import { getIiiClient } from '@/lib/iii-client'

export const TREE_FUNCTION_ID = 'coder::tree'

/** Bounds for the snapshot walk; noise dirs are excluded server-side. */
const TREE_MAX_DEPTH = 8
const TREE_PER_FOLDER_LIMIT = 500
const CACHE_TTL_MS = 30_000

/** Subset of `coder::tree`'s `TreeNode` the index consumes. */
export interface TreeNodeWire {
  name: string
  kind: 'file' | 'dir' | 'symlink' | 'other'
  children?: TreeNodeWire[]
}

export interface TreeOutputWire {
  path: string
  root: TreeNodeWire
}

type TriggerFn = (
  functionId: string,
  payload: Record<string, unknown>,
) => Promise<unknown>

interface CacheEntry {
  at: number
  index: string[]
}

const cache = new Map<string, CacheEntry>()
const inFlight = new Map<string, Promise<string[]>>()

/**
 * Flatten a tree snapshot to sorted RELATIVE file paths (the root itself is
 * `''` and never listed). Paths containing `)` are dropped — the
 * `#file(<path>)` token cannot carry them (documented limitation).
 */
export function flattenTree(root: TreeNodeWire): string[] {
  const out: string[] = []
  const walk = (node: TreeNodeWire, prefix: string) => {
    for (const child of node.children ?? []) {
      const path = prefix ? `${prefix}/${child.name}` : child.name
      if (child.kind === 'file') {
        if (!path.includes(')')) out.push(path)
      } else if (child.kind === 'dir') {
        walk(child, path)
      }
    }
  }
  walk(root, '')
  return out.sort((a, b) => a.localeCompare(b))
}

/**
 * Case-insensitive subsequence filter over relative paths. Ranks (1) basename
 * prefix matches, then (2) substring matches anywhere, then (3) subsequence
 * matches; ties break on shorter path then lexicographic.
 */
export function fuzzyFilterFiles(
  index: string[],
  query: string,
  limit = 8,
): string[] {
  const q = query.trim().toLowerCase()
  if (!q) return index.slice(0, limit)

  const ranked: Array<{ path: string; rank: number }> = []
  for (const path of index) {
    const lower = path.toLowerCase()
    const base = lower.slice(lower.lastIndexOf('/') + 1)
    let rank: number
    if (base.startsWith(q)) rank = 0
    else if (lower.includes(q)) rank = 1
    else if (isSubsequence(q, lower)) rank = 2
    else continue
    ranked.push({ path, rank })
  }
  return ranked
    .sort(
      (a, b) =>
        a.rank - b.rank ||
        a.path.length - b.path.length ||
        a.path.localeCompare(b.path),
    )
    .slice(0, limit)
    .map((r) => r.path)
}

function isSubsequence(needle: string, haystack: string): boolean {
  let i = 0
  for (const ch of haystack) {
    if (ch === needle[i]) i++
    if (i === needle.length) return true
  }
  return false
}

/**
 * Fetch (or reuse) the file index for a working directory. Returns the cached
 * index while fresh; on fetch failure the last good index is returned when one
 * exists, otherwise the error propagates. Concurrent callers share one fetch.
 */
export async function fetchFileIndex(
  workingDir: string,
  trigger?: TriggerFn,
): Promise<string[]> {
  const cached = cache.get(workingDir)
  if (cached && Date.now() - cached.at < CACHE_TTL_MS) {
    return cached.index
  }
  const pending = inFlight.get(workingDir)
  if (pending) return pending

  const fetchPromise = doFetch(workingDir, trigger)
    .then((index) => {
      cache.set(workingDir, { at: Date.now(), index })
      return index
    })
    .catch((err) => {
      const lastGood = cache.get(workingDir)
      if (lastGood) return lastGood.index
      throw err
    })
    .finally(() => {
      inFlight.delete(workingDir)
    })
  inFlight.set(workingDir, fetchPromise)
  return fetchPromise
}

async function doFetch(
  workingDir: string,
  trigger?: TriggerFn,
): Promise<string[]> {
  const call =
    trigger ??
    (async (functionId: string, payload: Record<string, unknown>) => {
      const client = await getIiiClient()
      return client.trigger(functionId, payload)
    })
  const res = (await call(TREE_FUNCTION_ID, {
    path: '.',
    fs_scope: { root: workingDir },
    max_depth: TREE_MAX_DEPTH,
    per_folder_limit: TREE_PER_FOLDER_LIMIT,
    use_default_excludes: true,
  })) as TreeOutputWire
  if (!res || typeof res !== 'object' || !res.root) {
    throw new Error('coder::tree returned an unexpected shape')
  }
  return flattenTree(res.root)
}

/** Test seam: drop all cached indexes. */
export function __resetFileIndexCacheForTests(): void {
  cache.clear()
  inFlight.clear()
}

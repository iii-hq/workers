/**
 * Working-directory file search for the composer's `@` / `#` typeaheads.
 *
 * Each query is one `coder::search` call in quick-open mode — path-only,
 * fuzzy-ranked by the worker, `.gitignore`-aware, dot-entries left out —
 * scoped to the conversation's working directory (`fs_scope.root`). The
 * shell worker jail-validates the scope on every call, so this surface
 * carries the same trust as the DirectoryPicker flow. An empty query is a
 * listing: the worker ranks shallow, short paths first.
 *
 * Results are cached per (working dir, query) for a short while so backing
 * over a query doesn't re-walk the repo, and concurrent callers share one
 * in-flight request.
 */

import { type TriggerFn, triggerOr } from '@/lib/attachments/shared'
import { workspaceScope } from '@/lib/fs-scope'

export const SEARCH_FUNCTION_ID = 'coder::search'

/** How many paths one query asks the worker for. */
export const FILE_SEARCH_LIMIT = 60
const CACHE_TTL_MS = 15_000
const CACHE_MAX_ENTRIES = 64

export interface FileHit {
  /** Working-directory-relative; folders carry a trailing `/`. */
  path: string
  kind: 'file' | 'dir'
}

export interface FileSearchOptions {
  limit?: number
}

/** What the composer is handed: a query in, hits out. */
export type FileSearchFn = (
  query: string,
  options?: FileSearchOptions,
) => Promise<FileHit[]>

/** Subset of `coder::search`'s output the typeahead consumes. */
interface SearchOutputWire {
  path_matches?: Array<{ path: string; kind?: string }>
  truncated?: boolean
}

interface CacheEntry {
  at: number
  hits: FileHit[]
}

const cache = new Map<string, CacheEntry>()
const inFlight = new Map<string, Promise<FileHit[]>>()

function cacheKey(workingDir: string, query: string, limit: number): string {
  return `${workingDir}\n${limit}\n${query}`
}

/** Absolute → working-dir-relative; anything outside the dir stays absolute. */
export function relativeToWorkingDir(workingDir: string, abs: string): string {
  const prefix = workingDir.endsWith('/') ? workingDir : `${workingDir}/`
  if (abs === workingDir || abs === prefix) return ''
  return abs.startsWith(prefix) ? abs.slice(prefix.length) : abs
}

/** Any dot-segment (`.github/…`, `.env`) — hidden the way a file browser hides them. */
export function isHiddenPath(path: string): boolean {
  return path.split('/').some((segment) => segment.startsWith('.'))
}

/**
 * Shape the worker's absolute matches into relative hits. Paths containing
 * `)` are dropped — the `#file(<path>)` token cannot carry them (documented
 * limitation) — and hidden entries are filtered here too, so a worker that
 * predates `include_hidden` still yields a clean list.
 */
export function hitsFromSearchOutput(
  workingDir: string,
  out: SearchOutputWire,
): FileHit[] {
  const hits: FileHit[] = []
  for (const match of out.path_matches ?? []) {
    if (!match || typeof match.path !== 'string') continue
    const rel = relativeToWorkingDir(workingDir, match.path)
    if (rel === '' || rel.includes(')') || isHiddenPath(rel)) continue
    if (match.kind === 'dir') hits.push({ path: `${rel}/`, kind: 'dir' })
    else hits.push({ path: rel, kind: 'file' })
  }
  return hits
}

/**
 * Search (or reuse) file hits for a query under a working directory. A
 * failed call resolves to `[]` — the menu simply shows no files — because a
 * worker that is away or a folder that no longer validates must never break
 * typing in the composer.
 */
export async function searchWorkspaceFiles(
  workingDir: string,
  query: string,
  { limit = FILE_SEARCH_LIMIT }: FileSearchOptions = {},
  trigger?: TriggerFn,
): Promise<FileHit[]> {
  const q = query.trim()
  const key = cacheKey(workingDir, q, limit)
  const cached = cache.get(key)
  if (cached && Date.now() - cached.at < CACHE_TTL_MS) return cached.hits
  const pending = inFlight.get(key)
  if (pending) return pending

  const request = doSearch(workingDir, q, limit, trigger)
    .then((hits) => {
      remember(key, hits)
      return hits
    })
    .catch((err: unknown) => {
      console.warn('[console] coder::search failed for the @ menu', err)
      return cached?.hits ?? []
    })
    .finally(() => {
      inFlight.delete(key)
    })
  inFlight.set(key, request)
  return request
}

function remember(key: string, hits: FileHit[]): void {
  cache.set(key, { at: Date.now(), hits })
  if (cache.size > CACHE_MAX_ENTRIES) {
    const oldest = cache.keys().next().value
    if (oldest !== undefined) cache.delete(oldest)
  }
}

async function doSearch(
  workingDir: string,
  query: string,
  limit: number,
  trigger?: TriggerFn,
): Promise<FileHit[]> {
  const call = triggerOr(trigger)
  const out = (await call(SEARCH_FUNCTION_ID, {
    query,
    path: '.',
    fs_scope: workspaceScope(workingDir),
    ignore_case: true,
    search_content: false,
    search_paths: true,
    fuzzy_paths: true,
    respect_gitignore: true,
    include_hidden: false,
    use_default_excludes: true,
    max_matches: limit,
  })) as SearchOutputWire
  if (!out || typeof out !== 'object') {
    throw new Error('coder::search returned an unexpected shape')
  }
  return hitsFromSearchOutput(workingDir, out)
}

/** A search function bound to one working directory, for the composer. */
export function createWorkspaceFileSearch(workingDir: string): FileSearchFn {
  return (query, options) => searchWorkspaceFiles(workingDir, query, options)
}

/** Test seam: drop every cached result. */
export function __resetFileSearchCacheForTests(): void {
  cache.clear()
  inFlight.clear()
}

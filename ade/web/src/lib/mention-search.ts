/**
 * One ranked list for the composer's `@` menu: functions and files
 * together, ordered by how well each matches the query, then paged.
 *
 * Files arrive already fuzzy-filtered by the worker (see file-search) and
 * functions from the catalog; this module only decides the merged order.
 * The scorer is deliberately simple and explainable: what you typed at the
 * start of a name beats it in the middle, a name beats its folder or its
 * description, and everything else is a subsequence match.
 */

import type { FileHit } from '@/lib/file-search'
import type { FunctionEntry } from '@/lib/functions'

export type MentionCandidate =
  | { kind: 'function'; id: string; description: string }
  | { kind: 'file'; path: string; isDir: boolean }

/** Rows per page of the menu; "show more" reveals the next page. */
export const MENTION_PAGE_SIZE = 10

/** Lower is better. */
const Tier = {
  NameStart: 0,
  FullStart: 1,
  NameContains: 2,
  FullContains: 3,
  DescriptionContains: 4,
  Subsequence: 5,
  None: 6,
} as const

/** Last path segment (`src/a.ts` → `a.ts`, `src/lib/` → `lib`). */
export function mentionName(candidate: MentionCandidate): string {
  if (candidate.kind === 'function') return candidate.id
  const trimmed = candidate.path.endsWith('/')
    ? candidate.path.slice(0, -1)
    : candidate.path
  return trimmed.slice(trimmed.lastIndexOf('/') + 1)
}

/** What sits after the name: the folder for a file, the description for a function. */
export function mentionDetail(candidate: MentionCandidate): string {
  if (candidate.kind === 'function') return candidate.description
  const trimmed = candidate.path.endsWith('/')
    ? candidate.path.slice(0, -1)
    : candidate.path
  const slash = trimmed.lastIndexOf('/')
  return slash === -1 ? '' : trimmed.slice(0, slash)
}

function fullText(candidate: MentionCandidate): string {
  return candidate.kind === 'function' ? candidate.id : candidate.path
}

function isSubsequence(needle: string, haystack: string): boolean {
  let i = 0
  for (const ch of haystack) {
    if (ch === needle[i]) i++
    if (i === needle.length) return true
  }
  return needle.length === 0
}

/** Case-insensitive match tier of one candidate against a lowercased query. */
export function mentionTier(
  candidate: MentionCandidate,
  query: string,
): number {
  if (query === '') return Tier.NameStart
  /* A folder's name keeps its slash so `shell/` finds the folder first. */
  const name = (
    candidate.kind === 'file' && candidate.isDir
      ? `${mentionName(candidate)}/`
      : mentionName(candidate)
  ).toLowerCase()
  const full = fullText(candidate).toLowerCase()
  if (name.startsWith(query)) return Tier.NameStart
  if (full.startsWith(query)) return Tier.FullStart
  if (name.includes(query)) return Tier.NameContains
  if (full.includes(query)) return Tier.FullContains
  if (
    candidate.kind === 'function' &&
    candidate.description.toLowerCase().includes(query)
  ) {
    return Tier.DescriptionContains
  }
  if (isSubsequence(query, full)) return Tier.Subsequence
  return Tier.None
}

/**
 * Merge and order. With no query the catalog order is kept — functions
 * first, then the worker's listing — so a bare `@` reads like a table of
 * contents rather than an alphabet soup. With a query, tiers rank first;
 * inside a tier the original order is kept for files (the worker's fuzzy
 * score already sorted them) and shorter ids win for functions.
 */
export function rankMentions(
  query: string,
  functions: readonly FunctionEntry[],
  files: readonly FileHit[],
): MentionCandidate[] {
  const q = query.trim().toLowerCase()
  const candidates: MentionCandidate[] = [
    ...functions.map(
      (entry): MentionCandidate => ({
        kind: 'function',
        id: entry.id,
        description: entry.description,
      }),
    ),
    ...files.map(
      (hit): MentionCandidate => ({
        kind: 'file',
        path: hit.path,
        isDir: hit.kind === 'dir',
      }),
    ),
  ]
  if (q === '') return candidates

  const ranked = candidates
    .map((candidate, index) => ({
      candidate,
      index,
      tier: mentionTier(candidate, q),
    }))
    .filter((row) => row.tier !== Tier.None)
  ranked.sort((a, b) => {
    if (a.tier !== b.tier) return a.tier - b.tier
    if (a.candidate.kind !== b.candidate.kind) {
      /* Same tier across kinds: a `::` in the query reads as a function id,
         a `/` or `.` as a path; otherwise leave files (worker-ranked) first. */
      const wantsFunction = q.includes('::')
      const wantsFile = !wantsFunction && (q.includes('/') || q.includes('.'))
      const aFn = a.candidate.kind === 'function'
      if (wantsFunction) return aFn ? -1 : 1
      if (wantsFile) return aFn ? 1 : -1
      return aFn ? 1 : -1
    }
    if (a.candidate.kind === 'function' && b.candidate.kind === 'function') {
      const byLength = a.candidate.id.length - b.candidate.id.length
      if (byLength !== 0) return byLength
    }
    return a.index - b.index
  })
  return ranked.map((row) => row.candidate)
}

export interface MentionPage<T> {
  visible: T[]
  /** Rows beyond the visible window; 0 when the page shows everything. */
  remaining: number
}

/** The first `page + 1` pages of `items`. */
export function paginateMentions<T>(
  items: readonly T[],
  page: number,
  pageSize = MENTION_PAGE_SIZE,
): MentionPage<T> {
  const count = Math.min(items.length, Math.max(1, page + 1) * pageSize)
  return {
    visible: items.slice(0, count),
    remaining: Math.max(0, items.length - count),
  }
}

/** Stable key for a candidate row. */
export function mentionKey(candidate: MentionCandidate): string {
  return candidate.kind === 'function'
    ? `fn:${candidate.id}`
    : `file:${candidate.path}`
}

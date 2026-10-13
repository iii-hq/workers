/**
 * Provider search for the composer's `@` menus: one call per provider,
 * cached briefly, raced against a timeout so a slow worker never holds the
 * menu, and delivered per provider as each answers.
 */

import { useEffect, useRef, useState } from 'react'
import {
  getMentionRuntime,
  mentionRuntimeGeneration,
  onMentionRuntimeReset,
} from './runtime'
import type {
  MentionItem,
  MentionProvider,
  MentionSearchContext,
} from './types'

export const SEARCH_CACHE_MS = 15_000
export const SEARCH_TIMEOUT_MS = 2_500
const SEARCH_CACHE_MAX = 128

interface CacheEntry {
  at: number
  promise: Promise<MentionItem[]>
}

const cache = new Map<string, CacheEntry>()
onMentionRuntimeReset(() => cache.clear())

function cacheKey(
  provider: MentionProvider,
  query: string,
  limit: number,
  context: MentionSearchContext | undefined,
): string {
  return [
    mentionRuntimeGeneration(),
    provider.name,
    provider.search,
    limit,
    context?.session_id ?? '',
    context?.working_dir ?? '',
    query,
  ].join('\u0000')
}

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error('mention search timed out')),
      ms,
    )
    promise.then(
      (value) => {
        clearTimeout(timer)
        resolve(value)
      },
      (error: unknown) => {
        clearTimeout(timer)
        reject(error)
      },
    )
  })
}

/**
 * One provider's rows for `query` (trimmed). Identical asks within
 * `SEARCH_CACHE_MS` share one call; a failure is not cached.
 */
export function searchMentionProvider(
  provider: MentionProvider,
  query: string,
  { limit, context }: { limit: number; context?: MentionSearchContext },
): Promise<MentionItem[]> {
  const trimmed = query.trim()
  const key = cacheKey(provider, trimmed, limit, context)
  const hit = cache.get(key)
  if (hit && Date.now() - hit.at < SEARCH_CACHE_MS) return hit.promise
  const promise = withTimeout(
    getMentionRuntime().search(provider, { query: trimmed, limit, context }),
    SEARCH_TIMEOUT_MS,
  )
  cache.set(key, { at: Date.now(), promise })
  promise.catch(() => {
    if (cache.get(key)?.promise === promise) cache.delete(key)
  })
  if (cache.size > SEARCH_CACHE_MAX) {
    const oldest = cache.keys().next().value
    if (oldest !== undefined) cache.delete(oldest)
  }
  return promise
}

export interface ProviderResults {
  items: MentionItem[]
  loading: boolean
  failed: boolean
}

export interface UseMentionSearchOptions {
  providers: readonly MentionProvider[]
  /** `null` = the menu is closed: nothing is asked. */
  query: string | null
  limit: number
  context?: MentionSearchContext
  /** Below this many characters nothing is asked (0 = always ask). */
  minQueryLength?: number
  debounceMs?: number
}

/**
 * Rows per provider name for the current query. Each provider's group
 * keeps its previous rows while the next answer is in flight (no flicker
 * between keystrokes) and fills in as soon as that provider answers.
 */
export function useMentionSearch({
  providers,
  query,
  limit,
  context,
  minQueryLength = 0,
  debounceMs = 120,
}: UseMentionSearchOptions): ReadonlyMap<string, ProviderResults> {
  const [results, setResults] = useState<ReadonlyMap<string, ProviderResults>>(
    () => new Map(),
  )
  const seqRef = useRef(0)
  const providerKey = providers.map((p) => `${p.name}:${p.search}`).join('|')
  const sessionId = context?.session_id
  const workingDir = context?.working_dir

  // biome-ignore lint/correctness/useExhaustiveDependencies: providers are tracked through providerKey, context through its two fields.
  useEffect(() => {
    const seq = ++seqRef.current
    const trimmed = query?.trim() ?? null
    if (
      trimmed === null ||
      trimmed.length < minQueryLength ||
      providers.length === 0
    ) {
      setResults((current) => (current.size === 0 ? current : new Map()))
      return
    }
    // Mark every asked provider as loading, keeping what it showed.
    setResults((current) => {
      const next = new Map<string, ProviderResults>()
      for (const provider of providers) {
        const previous = current.get(provider.name)
        next.set(provider.name, {
          items: previous?.items ?? [],
          loading: true,
          failed: false,
        })
      }
      return next
    })
    const searchContext: MentionSearchContext | undefined =
      sessionId || workingDir
        ? {
            ...(sessionId ? { session_id: sessionId } : {}),
            ...(workingDir ? { working_dir: workingDir } : {}),
          }
        : undefined
    const timer = setTimeout(() => {
      for (const provider of providers) {
        searchMentionProvider(provider, trimmed, {
          limit,
          context: searchContext,
        }).then(
          (items) => {
            if (seqRef.current !== seq) return
            setResults((current) =>
              new Map(current).set(provider.name, {
                items,
                loading: false,
                failed: false,
              }),
            )
          },
          () => {
            if (seqRef.current !== seq) return
            setResults((current) =>
              new Map(current).set(provider.name, {
                items: [],
                loading: false,
                failed: true,
              }),
            )
          },
        )
      }
    }, debounceMs)
    return () => clearTimeout(timer)
  }, [
    providerKey,
    query,
    limit,
    sessionId,
    workingDir,
    minQueryLength,
    debounceMs,
  ])

  return results
}

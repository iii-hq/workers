/**
 * The live set of mention providers, shared by every composer, pill and
 * preview in the tab. Loaded lazily (first reader) and refreshed when it
 * is older than the caller allows or a worker joins/leaves.
 */

import { useEffect, useSyncExternalStore } from 'react'
import { useWorkerLifecycle } from '@/hooks/use-worker-lifecycle'
import {
  getMentionRuntime,
  mentionRuntimeGeneration,
  onMentionRuntimeReset,
} from './runtime'
import type { MentionProvider } from './types'

export interface MentionProvidersSnapshot {
  status: 'idle' | 'loading' | 'ready' | 'error'
  providers: readonly MentionProvider[]
  byName: ReadonlyMap<string, MentionProvider>
}

/** How stale the list may be when a reader asks (the `@` menu opening). */
export const PROVIDERS_MAX_AGE_MS = 30_000

const IDLE: MentionProvidersSnapshot = {
  status: 'idle',
  providers: [],
  byName: new Map(),
}

let snapshot: MentionProvidersSnapshot = IDLE
let loadedAt = 0
let inflight: Promise<void> | null = null
const listeners = new Set<() => void>()

function publish(next: MentionProvidersSnapshot): void {
  snapshot = next
  for (const listener of [...listeners]) listener()
}

onMentionRuntimeReset(() => {
  loadedAt = 0
  inflight = null
  publish(IDLE)
})

export function getMentionProvidersSnapshot(): MentionProvidersSnapshot {
  return snapshot
}

export function subscribeMentionProviders(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function findMentionProvider(name: string): MentionProvider | undefined {
  return snapshot.byName.get(name)
}

/**
 * Load (or refresh) the provider list. Resolves once the list is at most
 * `maxAgeMs` old; concurrent callers share one request. A failed load keeps
 * the last good list.
 */
export function loadMentionProviders({
  maxAgeMs = PROVIDERS_MAX_AGE_MS,
  force = false,
}: {
  maxAgeMs?: number
  force?: boolean
} = {}): Promise<void> {
  const fresh = snapshot.status === 'ready' && Date.now() - loadedAt < maxAgeMs
  if (!force && fresh) return Promise.resolve()
  if (inflight) return inflight
  const generation = mentionRuntimeGeneration()
  if (snapshot.status !== 'ready') publish({ ...snapshot, status: 'loading' })
  const request = getMentionRuntime()
    .listProviders()
    .then(
      (providers) => {
        if (generation !== mentionRuntimeGeneration()) return
        loadedAt = Date.now()
        publish({
          status: 'ready',
          providers,
          byName: new Map(providers.map((p) => [p.name, p])),
        })
      },
      () => {
        if (generation !== mentionRuntimeGeneration()) return
        // Retry on the next read instead of caching the failure.
        loadedAt = 0
        publish({
          ...snapshot,
          status: snapshot.providers.length > 0 ? 'ready' : 'error',
        })
      },
    )
    .finally(() => {
      if (inflight === request) inflight = null
    })
  inflight = request
  return request
}

/** Mark the list stale; the next reader reloads it. */
export function invalidateMentionProviders(): void {
  loadedAt = 0
}

/** The provider list, loading it on first use. */
export function useMentionProviders(): MentionProvidersSnapshot {
  const current = useSyncExternalStore(
    subscribeMentionProviders,
    getMentionProvidersSnapshot,
    getMentionProvidersSnapshot,
  )
  useEffect(() => {
    void loadMentionProviders()
  }, [])
  return current
}

/**
 * Reload the provider list the moment a worker finishes joining or
 * leaving, so a freshly added worker's `@name` shows up without a refresh.
 */
export function useMentionProviderRefresh(enabled: boolean): void {
  useWorkerLifecycle({
    enabled,
    fnId: 'ui::mentions::worker-lifecycle',
    operations: ['add', 'remove'],
    stages: ['done'],
    onEvent: () => {
      void loadMentionProviders({ force: true })
    },
  })
}

/**
 * Resolved mentions, shared by every pill and preview in the tab: one get
 * call per item per minute however many places show it, stale views kept
 * on screen while they refresh.
 */

import { useEffect, useSyncExternalStore } from 'react'
import {
  findMentionProvider,
  getMentionProvidersSnapshot,
  loadMentionProviders,
} from './providers'
import {
  getMentionRuntime,
  mentionRuntimeGeneration,
  onMentionRuntimeReset,
} from './runtime'
import { mentionKey } from './token'
import type { MentionItem, MentionView } from './types'

export type MentionViewState =
  | { status: 'loading' }
  | { status: 'ready'; view: MentionView }
  | { status: 'missing' }
  | { status: 'error'; message: string }
  | { status: 'unknown-provider' }

export const VIEW_TTL_MS = 60_000
const VIEW_CACHE_MAX = 500

interface Entry {
  state: MentionViewState
  /** When `state` was fetched; 0 = provisional (refresh on first read). */
  at: number
  inflight: boolean
}

const LOADING: MentionViewState = { status: 'loading' }
const entries = new Map<string, Entry>()
const listeners = new Set<() => void>()

function emit(): void {
  for (const listener of [...listeners]) listener()
}

onMentionRuntimeReset(() => {
  entries.clear()
  emit()
})

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

function setEntry(key: string, entry: Entry): void {
  entries.delete(key)
  entries.set(key, entry)
  if (entries.size > VIEW_CACHE_MAX) {
    const oldest = entries.keys().next().value
    if (oldest !== undefined) entries.delete(oldest)
  }
  emit()
}

export function getMentionViewState(
  name: string,
  id: string,
): MentionViewState {
  return entries.get(mentionKey(name, id))?.state ?? LOADING
}

function isStale(entry: Entry | undefined): boolean {
  if (!entry) return true
  if (entry.inflight) return false
  return Date.now() - entry.at >= VIEW_TTL_MS
}

/** Fetch one item's view unless a fresh one (or a fetch) is already there. */
export async function requestMentionView(
  name: string,
  id: string,
  { force = false }: { force?: boolean } = {},
): Promise<void> {
  const key = mentionKey(name, id)
  const existing = entries.get(key)
  if (!force && !isStale(existing)) return
  if (existing?.inflight) return
  const generation = mentionRuntimeGeneration()
  // A stale view stays on screen while it refreshes.
  const shown = existing?.state.status === 'ready' ? existing.state : LOADING
  setEntry(key, { state: shown, at: existing?.at ?? 0, inflight: true })

  const settle = (state: MentionViewState) => {
    if (generation !== mentionRuntimeGeneration()) return
    // A refresh that fails keeps the view it had.
    const keep =
      state.status === 'error' && shown.status === 'ready' ? shown : state
    setEntry(key, { state: keep, at: Date.now(), inflight: false })
  }

  if (getMentionProvidersSnapshot().status !== 'ready') {
    await loadMentionProviders()
  }
  const provider = findMentionProvider(name)
  if (!provider) {
    settle(
      getMentionProvidersSnapshot().status === 'ready'
        ? { status: 'unknown-provider' }
        : { status: 'error', message: 'mention providers unavailable' },
    )
    return
  }
  try {
    const view = await getMentionRuntime().get(provider, id)
    settle(view ? { status: 'ready', view } : { status: 'missing' })
  } catch (error) {
    settle({
      status: 'error',
      message: error instanceof Error ? error.message : String(error),
    })
  }
}

/**
 * Show something right away for an item just picked from a menu: its
 * search row stands in as the view until the real one lands.
 */
export function primeMentionView(name: string, item: MentionItem): void {
  const key = mentionKey(name, item.id)
  if (entries.get(key)?.state.status === 'ready') return
  setEntry(key, {
    state: {
      status: 'ready',
      view: {
        id: item.id,
        label: item.label,
        hint: item.hint,
        description: item.description,
        icon: item.icon,
        color: item.color,
      },
    },
    at: 0,
    inflight: false,
  })
}

/** Drop cached views (all of a provider's, or one item's). */
export function invalidateMentionViews(name?: string, id?: string): void {
  if (name !== undefined && id !== undefined) {
    entries.delete(mentionKey(name, id))
  } else if (name !== undefined) {
    for (const key of [...entries.keys()]) {
      if (key.startsWith(`${name}\u0000`)) entries.delete(key)
    }
  } else {
    entries.clear()
  }
  emit()
}

/** One mention's view, fetched on first use and refreshed when stale. */
export function useMentionView(name: string, id: string): MentionViewState {
  const state = useSyncExternalStore(
    subscribe,
    () => getMentionViewState(name, id),
    () => getMentionViewState(name, id),
  )
  useEffect(() => {
    void requestMentionView(name, id)
  }, [name, id])
  return state
}

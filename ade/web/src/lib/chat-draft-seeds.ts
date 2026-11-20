/**
 * The unsent chat drafts the shared workspace layout keeps for its columns (`chatDrafts`, see
 * `ScreenPlacement.draft`), as the latest layout read or write saw them. The workspace hook publishes them; a chat
 * column that cannot find its conversation reads its seed here and brings the draft back.
 */
import { useSyncExternalStore } from 'react'
import type { ChatDraftSeed } from '@/lib/workspace-tabs'

let seeds: Readonly<Record<string, ChatDraftSeed>> = {}
const listeners = new Set<() => void>()

let retire: ((sessionId: string) => void) | null = null

/** The workspace hook's writer that drops one seed from the shared layout. */
export function registerChatDraftRetirer(
  next: ((sessionId: string) => void) | null,
): void {
  retire = next
}

/**
 * The draft became a real session (sent), or its session was deleted: its seed must not bring it back. A no-op when
 * the layout keeps no seed for it.
 */
export function retireChatDraftSeed(sessionId: string): void {
  if (!(sessionId in seeds)) return
  retire?.(sessionId)
}

export function publishChatDraftSeeds(
  next: Readonly<Record<string, ChatDraftSeed>>,
): void {
  if (JSON.stringify(next) === JSON.stringify(seeds)) return
  seeds = next
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/** The seed for `sessionId`'s column, if the layout keeps one. */
export function useChatDraftSeed(
  sessionId: string | undefined,
): ChatDraftSeed | undefined {
  return useSyncExternalStore(
    subscribe,
    () => (sessionId ? seeds[sessionId] : undefined),
    () => undefined,
  )
}

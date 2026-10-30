import { useCallback, useEffect, useState } from 'react'

/**
 * Pinned chats: a per-browser list of conversation ids the reader keeps at
 * the top of the sidebar. A UI affordance like the list view, so it lives
 * in localStorage beside it rather than in session metadata.
 */
export const PINNED_CONVERSATIONS_KEY = 'iii-chat-pinned'

export function loadPinnedConversationIds(): string[] {
  if (typeof window === 'undefined') return []
  try {
    const raw = window.localStorage.getItem(PINNED_CONVERSATIONS_KEY)
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    return Array.isArray(parsed)
      ? parsed.filter((id): id is string => typeof id === 'string')
      : []
  } catch {
    return []
  }
}

export function savePinnedConversationIds(ids: readonly string[]): void {
  if (typeof window === 'undefined') return
  try {
    if (ids.length === 0)
      window.localStorage.removeItem(PINNED_CONVERSATIONS_KEY)
    else
      window.localStorage.setItem(
        PINNED_CONVERSATIONS_KEY,
        JSON.stringify([...new Set(ids)]),
      )
  } catch {
    /* Private mode or a full store: pins simply do not persist. */
  }
}

/** Toggle one id; newest pins go first so the Pinned section reads newest-first. */
export function togglePinnedId(ids: readonly string[], id: string): string[] {
  return ids.includes(id) ? ids.filter((x) => x !== id) : [id, ...ids]
}

export function usePinnedConversations(): {
  pinnedIds: readonly string[]
  pinned: ReadonlySet<string>
  togglePin: (id: string) => void
} {
  const [pinnedIds, setPinnedIds] = useState<readonly string[]>(
    loadPinnedConversationIds,
  )
  useEffect(() => {
    savePinnedConversationIds(pinnedIds)
  }, [pinnedIds])
  const togglePin = useCallback((id: string) => {
    setPinnedIds((current) => togglePinnedId(current, id))
  }, [])
  return { pinnedIds, pinned: new Set(pinnedIds), togglePin }
}

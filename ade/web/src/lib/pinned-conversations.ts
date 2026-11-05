import { useCallback, useEffect, useMemo, useState } from 'react'

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

/**
 * Move `id` to land just before or after `targetId` (a drag dropped on a
 * row's upper or lower half). Unknown ids, or a drop on itself, change
 * nothing.
 */
export function movePinnedId(
  ids: readonly string[],
  id: string,
  targetId: string,
  place: 'before' | 'after',
): readonly string[] {
  if (id === targetId || !ids.includes(id) || !ids.includes(targetId))
    return ids
  const rest = ids.filter((x) => x !== id)
  const at = rest.indexOf(targetId) + (place === 'after' ? 1 : 0)
  const next = [...rest.slice(0, at), id, ...rest.slice(at)]
  return next.every((x, i) => x === ids[i]) ? ids : next
}

export function usePinnedConversations(): {
  pinnedIds: readonly string[]
  pinned: ReadonlySet<string>
  togglePin: (id: string) => void
  movePin: (id: string, targetId: string, place: 'before' | 'after') => void
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
  const movePin = useCallback(
    (id: string, targetId: string, place: 'before' | 'after') => {
      setPinnedIds((current) => movePinnedId(current, id, targetId, place))
    },
    [],
  )
  const pinned = useMemo(() => new Set(pinnedIds), [pinnedIds])
  return { pinnedIds, pinned, togglePin, movePin }
}

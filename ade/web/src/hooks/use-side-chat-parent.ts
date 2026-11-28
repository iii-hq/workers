import { useEffect, useState } from 'react'
import { type ResolvedConversation, sideChatParentId } from '@/lib/autocompact'
import { getSession } from '@/lib/sessions/api'
import type { Conversation } from '@/types/chat'

/** `metadata.project_id`, as the conversation list reads it. */
function projectIdOf(metadata: Record<string, unknown>): string | undefined {
  const id = metadata.project_id
  return typeof id === 'string' && id.length > 0 ? id : undefined
}

/**
 * A side chat's conversation when it isn't loaded here (a side chat opened
 * from a link, or its conversation paged out of the list): read with
 * `session::get`, as the harness reads it through `side_chat_of` (D164).
 * Null for any other conversation, while the read runs, and when it fails:
 * the callers then show no value rather than the side chat's frozen copy.
 * Read again when the side chat's status changes (a turn may follow a
 * change to the conversation's setting).
 */
export function useSideChatParent(input: {
  conversation: Pick<Conversation, 'sessionMetadata' | 'status'>
  conversations: readonly Pick<Conversation, 'id'>[] | undefined
  enabled: boolean
}): ResolvedConversation | null {
  const { conversation, conversations, enabled } = input
  const parentId = sideChatParentId(conversation)
  const loaded = Boolean(
    parentId && conversations?.some((c) => c.id === parentId),
  )
  const reads = enabled && parentId !== null && !loaded
  const [parent, setParent] = useState<ResolvedConversation | null>(null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: re-read per status change
  useEffect(() => {
    if (!reads || !parentId) return
    let cancelled = false
    getSession(parentId)
      .then((meta) => {
        if (cancelled) return
        if (!meta) {
          setParent(null)
          return
        }
        const metadata = meta.metadata ?? {}
        setParent({
          id: parentId,
          sessionMetadata: metadata,
          projectId: projectIdOf(metadata),
        })
      })
      .catch(() => {
        // Nothing to read the conversation's value from: no value shows.
        if (!cancelled) setParent(null)
      })
    return () => {
      cancelled = true
    }
  }, [reads, parentId, conversation.status])

  return reads && parent?.id === parentId ? parent : null
}

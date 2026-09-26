import { getIiiClient } from '@/lib/iii-client'
import type { Conversation } from '@/types/chat'
import { getSession } from './api'

export interface RemovalPreview {
  id: string
  title: string
  parentId?: string
  hasChildren: boolean
  hasRunningWork: boolean
  /** Only a verified empty, inactive leaf can skip confirmation. */
  empty: boolean
}

function hasLocalContent(conversation: Conversation): boolean {
  return Boolean(
    conversation.started ||
      conversation.messages.length ||
      conversation.draftText?.trim() ||
      conversation.draftAttachments?.length,
  )
}

/** Read once on removal, never infer a full tree from the bounded sidebar. */
export async function getRemovalPreview(
  conversation: Conversation,
): Promise<RemovalPreview> {
  const preview: RemovalPreview = {
    id: conversation.id,
    title: conversation.title,
    parentId: conversation.parentId,
    hasChildren: false,
    hasRunningWork: conversation.status === 'working',
    empty: false,
  }
  if (conversation.draft) {
    return {
      ...preview,
      empty: !preview.hasRunningWork && !hasLocalContent(conversation),
    }
  }

  const client = await getIiiClient()
  const tree = await client.trigger<{
    root_session_id: string
    sessions: { session_id: string }[]
    complete: boolean
  }>(
    'harness::session-tree',
    { root_session_id: conversation.id },
    { timeoutMs: 10_000 },
  )
  if (
    !tree?.complete ||
    tree.root_session_id !== conversation.id ||
    !Array.isArray(tree.sessions) ||
    !tree.sessions.some((node) => node.session_id === conversation.id) ||
    tree.sessions.some((node) => typeof node.session_id !== 'string')
  ) {
    throw new Error(
      'Could not verify this conversation and its subagents. Please try again.',
    )
  }
  const ids = [...new Set(tree.sessions.map((node) => node.session_id))]
  preview.hasChildren = ids.length > 1
  // The persisted snapshot is authoritative; local data only prevents an
  // unsent/optimistic message from being mistaken for an empty conversation.
  preview.hasRunningWork = false
  let emptyRoot = false
  for (let offset = 0; offset < ids.length; offset += 8) {
    const batch = ids.slice(offset, offset + 8)
    const sessions = await Promise.all(batch.map((id) => getSession(id)))
    for (const [index, meta] of sessions.entries()) {
      if (
        !meta ||
        meta.session_id !== batch[index] ||
        !['idle', 'working', 'done', 'error'].includes(meta.status) ||
        !Number.isSafeInteger(meta.message_count) ||
        meta.message_count < 0
      ) {
        throw new Error(
          'Conversation details changed or could not be verified. Please try again.',
        )
      }
      preview.hasRunningWork ||= meta.status === 'working'
      if (meta.session_id === conversation.id) {
        preview.title = meta.title || conversation.title
        const parent = meta.metadata?.parent_session_id
        preview.parentId = typeof parent === 'string' ? parent : undefined
        emptyRoot =
          meta.message_count === 0 &&
          !meta.draft?.trim() &&
          !meta.draft_attachments?.length &&
          !hasLocalContent(conversation)
      }
    }
  }
  preview.empty = emptyRoot && !preview.hasChildren && !preview.hasRunningWork
  return preview
}

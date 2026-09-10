/**
 * Title-only conversation search for the sidebar (MOT-3887). Case-insensitive
 * substring match; trims the query; empty query returns the input list
 * unchanged; preserves input order. Content search is out of scope (would
 * need a server-side session-manager function).
 */

import type { Conversation } from '@/types/chat'

export function filterConversations(
  conversations: Conversation[],
  query: string,
): Conversation[] {
  const q = query.trim().toLowerCase()
  if (!q) return conversations
  return conversations.filter((c) => c.title.toLowerCase().includes(q))
}

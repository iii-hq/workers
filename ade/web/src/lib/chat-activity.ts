import type { Conversation } from '@/types/chat'

/**
 * Session status covers model wait/reasoning/answering, tools and approvals,
 * including subagents and sessions in other workspace panels. Never infer
 * liveness from old transcript flags, armed triggers or a connected socket.
 */
export function hasWorkingConversation(
  conversations: readonly Pick<Conversation, 'status'>[],
  connected: boolean,
): boolean {
  return (
    connected && conversations.some((session) => session.status === 'working')
  )
}

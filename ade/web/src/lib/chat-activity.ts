import type { Conversation } from '@/types/chat'

/**
 * Session status covers model wait/reasoning/answering, tools and approvals,
 * including subagents and sessions in other workspace panels. Never infer
 * liveness from old transcript flags, armed observers or a connected socket.
 */
export function hasWorkingConversation(
  conversations: readonly Pick<Conversation, 'status'>[],
  connected: boolean,
): boolean {
  return (
    connected && conversations.some((session) => session.status === 'working')
  )
}

/**
 * The `done` reason the harness projects when a turn ended on a
 * `harness::ask` card (`harness/src/session_status.rs`): the agent is
 * waiting on the user's answer, which arrives as their next message.
 */
export const AWAITING_ANSWER_REASON = 'awaiting answer'

/** Whether the session's last turn ended on a question the user has not answered. */
export function isAwaitingAnswer(
  conversation: Pick<Conversation, 'status' | 'statusReason'>,
): boolean {
  return (
    conversation.status === 'done' &&
    conversation.statusReason === AWAITING_ANSWER_REASON
  )
}

import type { AgentMessage, TranscriptItem } from '@/lib/sessions/types'
import type { Conversation } from '@/types/chat'
import { deriveSubagentVisualStatus } from '../active-subagents'

export type SubagentActivityKind =
  | 'active'
  | 'working'
  | 'thinking'
  | 'messaging'
  | 'error'
  | 'completed'
  | 'stopped'
  | 'disconnected'
  | 'queued'
  | 'waiting'

export interface SubagentActivitySignal {
  kind: Extract<SubagentActivityKind, 'working' | 'thinking' | 'messaging'>
  timestamp: number
}

/** The latest visible operation encoded in a child transcript message. */
export function activityFromAgentMessage(
  message: AgentMessage | undefined,
  eventTimestamp?: number,
): SubagentActivitySignal | null {
  if (!message) return null
  const timestamp = eventTimestamp ?? message.timestamp

  if (message.role === 'function_result') {
    return { kind: 'working', timestamp }
  }
  if (message.role === 'user') {
    return { kind: 'working', timestamp }
  }
  if (message.role !== 'assistant') return null

  for (let index = message.content.length - 1; index >= 0; index -= 1) {
    const block = message.content[index]
    if (block.type === 'thinking') return { kind: 'thinking', timestamp }
    if (block.type === 'text' && block.text.length > 0) {
      return { kind: 'messaging', timestamp }
    }
    if (block.type === 'function_call' || block.type === 'function_result') {
      return { kind: 'working', timestamp }
    }
  }
  return { kind: 'working', timestamp }
}

/** Seed a widget mounted midway through a turn from durable transcript data. */
export function latestSubagentActivity(
  items: readonly TranscriptItem[],
): SubagentActivitySignal | null {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const signal = activityFromAgentMessage(items[index].message)
    if (signal) return signal
  }
  return null
}

/** Session status is authoritative for terminal/idle states; transcript
 * activity refines only a session that is still working. */
export function displayedSubagentActivity(
  conversation: Conversation | undefined,
  signal: SubagentActivitySignal | null,
  connectionState: Parameters<
    typeof deriveSubagentVisualStatus
  >[1] = 'connected',
): SubagentActivityKind {
  if (!conversation) {
    return connectionState === 'connected'
      ? (signal?.kind ?? 'working')
      : 'disconnected'
  }
  const visual = deriveSubagentVisualStatus(conversation, connectionState)
  if (visual === 'failed') return 'error'
  if (visual === 'working') return signal?.kind ?? 'working'
  return visual
}

interface ResolveChildSessionInput {
  responseSessionId?: string | null
  requestSessionId?: string | null
  parentSessionId?: string
  functionTriggerId?: string
  conversations: readonly Conversation[]
}

/** Resolve modern direct responses first, then named sessions, then the
 * durable parent-call link stamped on child session metadata. */
export function resolveChildSessionId({
  responseSessionId,
  requestSessionId,
  parentSessionId,
  functionTriggerId,
  conversations,
}: ResolveChildSessionInput): string | null {
  if (responseSessionId) return responseSessionId
  if (requestSessionId) return requestSessionId
  if (!functionTriggerId) return null
  return (
    conversations.find(
      (conversation) =>
        conversation.parentFunctionCallId === functionTriggerId &&
        (!parentSessionId || conversation.parentId === parentSessionId),
    )?.id ?? null
  )
}

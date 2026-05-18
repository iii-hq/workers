import type {
  AgentEvent,
  AgentMessage,
  AssistantMessageEvent,
  ContentBlock,
} from '@/types/iii-agent-event'
import type { StreamEvent } from './types'

export function translateAgentEvent(
  event: AgentEvent,
  sessionId?: string,
): StreamEvent[] {
  switch (event.type) {
    case 'agent_start':
    case 'turn_start':
    case 'turn_end':
    case 'function_execution_update':
      return []

    case 'message_end':
      // Emit assistant-end at each turn boundary, otherwise multi-turn
      // agents (text → fcall → text → ...) would merge every turn's
      // text into the first assistant message.
      if (event.message.role === 'assistant') {
        return [{ kind: 'assistant-end' }]
      }
      return []

    case 'message_update':
      return translateMessageUpdate(event.llm_event)

    case 'message_start':
      return translateMessageStart(event.message)

    case 'function_execution_start':
      return [
        {
          kind: 'fcall-start',
          functionId: event.function_id,
          input: event.args,
          functionCallId: event.function_call_id,
          sessionId,
        },
      ]

    case 'function_execution_end':
      return [
        {
          kind: 'fcall-end',
          output: event.result,
          durationMs: 0,
        },
      ]

    case 'approval_requested':
      return [
        {
          kind: 'fcall-start',
          functionId: event.function_id,
          input: event.args,
          pendingApproval: true,
          functionCallId: event.function_call_id,
          sessionId,
        },
      ]

    case 'approval_resolved':
      if (event.decision === 'allow') {
        return []
      }
      return [
        {
          kind: 'fcall-end',
          output: {
            error: {
              kind: 'denied',
              message: event.reason ?? 'denied by user',
            },
          },
          durationMs: 0,
        },
      ]

    case 'agent_end':
      return [{ kind: 'assistant-end' }]
  }
}

// Done/Error events terminate via MessageEnd → agent_end → assistant-end,
// so we only surface the non-terminal text and thinking deltas here.
function translateMessageUpdate(llm: AssistantMessageEvent): StreamEvent[] {
  switch (llm.type) {
    case 'text_delta':
      if (llm.delta.length === 0) return []
      return [{ kind: 'assistant-token', token: llm.delta }]
    case 'thinking_start':
      return [{ kind: 'thought-start' }]
    case 'thinking_delta':
      if (llm.delta.length === 0) return []
      return [{ kind: 'thought-token', token: llm.delta }]
    case 'thinking_end':
      return [{ kind: 'thought-end', durationMs: 0 }]
    default:
      return []
  }
}

function translateMessageStart(message: AgentMessage): StreamEvent[] {
  if (message.role !== 'assistant') {
    return []
  }
  // If the provider streamed deltas, message_update already populated the
  // renderer — re-emitting the body here would duplicate everything. We
  // only fall through for non-streaming providers that ship the full body
  // in a single MessageStart.
  const hasStreamableContent = message.content.some(
    (b) => b.type === 'text' || b.type === 'thinking',
  )
  if (hasStreamableContent) {
    return []
  }
  const out: StreamEvent[] = []
  for (const block of message.content) {
    appendBlock(block, out)
  }
  return out
}

function appendBlock(block: ContentBlock, out: StreamEvent[]): void {
  switch (block.type) {
    case 'thinking':
      out.push({ kind: 'thought-start' })
      if (block.text.length > 0) {
        out.push({ kind: 'thought-token', token: block.text })
      }
      out.push({ kind: 'thought-end', durationMs: 0 })
      return
    case 'text':
      if (block.text.length > 0) {
        out.push({ kind: 'assistant-token', token: block.text })
      }
      return
    case 'functionCall':
    case 'functionResult':
    case 'image':
      // FunctionCall/Result blocks ride on function_execution_start/end;
      // images aren't part of the StreamEvent contract.
      return
  }
}

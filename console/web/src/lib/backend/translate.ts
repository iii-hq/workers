/**
 * Pure translator from iii `AgentEvent` (the wire shape on
 * `agent::events`) to console/web's `StreamEvent` contract documented in
 * `PLAYGROUND.md`.
 *
 * Use `createAgentEventTranslator()` for a stateful translator that handles
 * the full event surface, including `turn_state_changed` pending-approval
 * mirroring.
 *
 * Wire mapping:
 *   - `text_delta`     → `assistant-token { token: delta }`
 *   - `thinking_start` → `thought-start`
 *   - `thinking_delta` → `thought-token { token: delta }`
 *   - `thinking_end`   → `thought-end { durationMs: 0 }`
 *   - other LLM events (text_start/text_end, functioncall_*, usage,
 *     stop, done, error) → no UI signal.
 *
 * Other events:
 *   - `function_execution_start` → `fcall-start` (with args).
 *   - `function_execution_end`   → `fcall-end` (with result).
 *   - `agent_end` → `assistant-end`.
 *   - `turn_state_changed` → pending-approval `fcall-start` (stateful).
 *   - `turn_end` → not translated (async compaction listens on raw wire).
 */

import type {
  AgentEvent,
  AgentMessage,
  AssistantMessageEvent,
  ContentBlock,
  FunctionResult,
} from '@/types/iii-agent-event'
import { diffPending, type PendingApproval } from './pending-approvals-store'
import { pendingApprovalsFromTurnState } from './turn-state-mirror'
import type { StreamEvent } from './types'

export function createAgentEventTranslator(): {
  translate(event: AgentEvent, sessionId?: string): StreamEvent[]
} {
  const mirrors = new Map<string, PendingApproval[]>()

  function translateTurnStateChanged(
    event: Extract<AgentEvent, { type: 'turn_state_changed' }>,
    sessionId: string,
  ): StreamEvent[] {
    const prev = mirrors.get(sessionId) ?? []
    const next = pendingApprovalsFromTurnState(event.new_value)
    mirrors.set(sessionId, next)
    const { added, removed } = diffPending(prev, next)
    const out: StreamEvent[] = added.map((entry) => ({
      kind: 'fcall-start' as const,
      functionId: entry.function_id,
      input: entry.args,
      pendingApproval: true,
      functionCallId: entry.function_call_id,
      sessionId,
    }))
    for (const entry of removed) {
      out.push({
        kind: 'fcall-approval-cleared',
        functionCallId: entry.function_call_id,
      })
    }
    return out
  }

  function translate(event: AgentEvent, sessionId?: string): StreamEvent[] {
    switch (event.type) {
      case 'turn_state_changed':
        return sessionId ? translateTurnStateChanged(event, sessionId) : []

      case 'message_complete':
        return translateMessageComplete(
          event.message,
          event.body_streamed === true,
        )

      case 'message_update':
        return translateMessageUpdate(event.llm_event)

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
            output: event.is_error
              ? wrapErrorOutput(event.result)
              : event.result,
            durationMs: event.duration_ms,
            functionCallId: event.function_call_id,
          },
        ]

      case 'agent_end':
        return [{ kind: 'assistant-end' }]

      case 'turn_end':
        return []

      case 'compaction_done':
        return [
          {
            kind: 'compaction',
            mode: event.mode,
            summaryText: event.summary_text,
            tokensBefore: event.tokens_before,
            compactionEntryId: event.compaction_entry_id,
            tailStartId: event.tail_start_id,
          },
        ]
    }
  }

  return { translate }
}

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

function translateMessageComplete(
  message: AgentMessage,
  bodyStreamed: boolean,
): StreamEvent[] {
  if (message.role !== 'assistant') {
    return []
  }
  const out: StreamEvent[] = []
  if (!bodyStreamed) {
    for (const block of message.content) {
      appendBlock(block, out)
    }
  }
  out.push({ kind: 'assistant-end' })
  const stop = message.stop_reason as
    | 'end'
    | 'length'
    | 'error'
    | 'aborted'
    | 'function_call'
    | undefined
  if (stop === 'length' || stop === 'error' || stop === 'aborted') {
    out.push({
      kind: 'stop-reason',
      reason: stop,
      message:
        typeof message.error_message === 'string' &&
        message.error_message.length > 0
          ? message.error_message
          : undefined,
    })
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
      return
  }
}

function wrapErrorOutput(result: FunctionResult): {
  error: {
    kind: string
    message: string
    details: unknown
    content: ContentBlock[]
  }
} {
  return {
    error: {
      kind: 'function_error',
      message: deriveErrorMessage(result.content),
      details: result.details,
      content: result.content,
    },
  }
}

function deriveErrorMessage(content: ContentBlock[]): string {
  for (const block of content) {
    if (block.type === 'text' && block.text.length > 0) {
      return block.text.replace(/\s+/g, ' ').trim()
    }
  }
  return 'function returned an error'
}

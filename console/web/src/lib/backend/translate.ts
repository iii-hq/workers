/**
 * Pure translator from iii `AgentEvent` (the wire shape on `agent::events`)
 * to console/web's `StreamEvent` contract.
 *
 * Since the session-manager integration, transcript CONTENT (thought/text
 * tokens, full message snapshots) renders from `session::message_added` /
 * `session::message_updated` events reconciled by the conversations layer —
 * NOT from this translator. `agent::events` remains the channel for
 * ephemeral turn state only:
 *
 *   - `turn_state_changed` → pending-approval `fcall-start` (stateful).
 *   - `function_execution_start` → `fcall-start` (running, with args).
 *   - `function_execution_end`   → `fcall-end` (result + duration).
 *   - `message_complete` with a non-`end` stop_reason → `stop-reason`.
 *   - `agent_end` → `assistant-end` (turn-over signal).
 *   - `message_update` / `compaction_done` / `turn_end` → no UI signal
 *     (tokens and compaction markers arrive via session events).
 */

import type {
  AgentEvent,
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
        return translateMessageComplete(event.message)

      case 'message_update':
        // Transcript content arrives via session::message_updated snapshots.
        return []

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
        // The compaction marker renders from the session's custom entry
        // (session::message_added); no separate UI signal needed here.
        return []
    }
  }

  return { translate }
}

function translateMessageComplete(
  message: Extract<AgentEvent, { type: 'message_complete' }>['message'],
): StreamEvent[] {
  if (message.role !== 'assistant') {
    return []
  }
  const out: StreamEvent[] = []
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

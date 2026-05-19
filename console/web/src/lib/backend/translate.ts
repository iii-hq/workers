/**
 * Pure translator from iii `AgentEvent` (the wire shape on
 * `agent::events`) to console/web's `StreamEvent` contract documented in
 * `PLAYGROUND.md`.
 *
 * Phase 2.A: `turn-orchestrator` now emits `MessageUpdate` events with a
 * provider `AssistantMessageEvent` payload for every non-terminal frame,
 * so token-by-token streaming flows through the `message_update` branch
 * below. The terminal `MessageStart`/`MessageEnd` for the assistant
 * message are still emitted, but `translateMessageStart` no longer
 * re-emits the body (the deltas already populated the renderer); it
 * just emits any function-call blocks that ride on the same message.
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
 *   - `approval_resolved` deny → synthetic `fcall-end`.
 *   - `approval_resolved` allow → noop (the matching exec_start follows).
 *   - `agent_end` → `assistant-end`.
 *   - `agent_start` / `turn_start` / `turn_end` / `message_end` /
 *     `function_execution_update` → noop.
 *   - `turn_state_changed` → noop (routed through `createTurnStateTranslator`).
 */

import type {
  AgentEvent,
  AgentMessage,
  AssistantMessageEvent,
  ContentBlock,
  TurnStateChangedEvent,
} from '@/types/iii-agent-event'
import { diffPending, type PendingApproval } from './pending-approvals-store'
import { pendingApprovalsFromTurnState } from './turn-state-mirror'
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

    // `turn_state_changed` events are always routed to `createTurnStateTranslator`
    // in `real.ts` before reaching this function. The case here exists only to
    // satisfy the exhaustiveness check; calling `translateAgentEvent` with one
    // directly is a bug.
    case 'turn_state_changed':
      return []

    case 'message_end':
      // Per-turn boundary: signal `assistant-end` so consumers can finalize
      // the streaming assistant message and reset their per-turn pointer.
      // Without this, multi-turn agents (text → fcall → text → ...) merge
      // every turn's text into the first assistant message.
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

/**
 * Phase 2.A: translate a provider `AssistantMessageEvent` (carried inside
 * `AgentEvent.MessageUpdate.llm_event`) into the StreamEvent contract.
 * Non-terminal text and thinking deltas drive the renderer; everything
 * else is silently dropped — the terminal `Done`/`Error` event is
 * mirrored by a `MessageEnd` (and ultimately by `agent_end` →
 * `assistant-end`), so we don't need to surface them here.
 */
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
  // User prompts and tool/function results aren't surfaced as backend-driven
  // StreamEvents — the consumer renders the user message itself and the
  // tool result rides on `fcall-end`'s output.
  if (message.role !== 'assistant') {
    return []
  }
  // Phase 2.A: assistant text and thinking already streamed via
  // `message_update`; don't double-emit them here. We only forward
  // function-call / image blocks, which don't ride on the streamed
  // delta path. (Function calls actually arrive on
  // `function_execution_start`, but the assistant message technically
  // owns them too — keeping the early return below preserves the
  // historical contract for non-streaming providers that emit a single
  // `MessageStart` with the full body.)
  const hasStreamableContent = message.content.some(
    (b) => b.type === 'text' || b.type === 'thinking',
  )
  if (hasStreamableContent) {
    // The provider streamed; nothing to re-emit.
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
      // FunctionCall/Result blocks ride on dedicated AgentEvents
      // (function_execution_start/end); images aren't part of the
      // StreamEvent contract today.
      return
  }
}

/**
 * Stateful translator for `turn_state_changed` events. Holds a per-session
 * mirror of the previous `awaiting_approval` list so it can emit a
 * `fcall-start { pendingApproval: true }` exactly once per new pending
 * call, and suppress duplicates when the same record is re-broadcast
 * (the backend emits on every turn_state write, not just transitions
 * into the parking state).
 *
 * No `fcall-end` is emitted when the list shrinks — the orchestrator
 * already fires `function_execution_end` for resolved/denied calls
 * through its existing path. Removing the entry from our mirror is
 * bookkeeping only.
 */
export function createTurnStateTranslator(): (
  event: TurnStateChangedEvent,
  sessionId: string,
) => StreamEvent[] {
  const mirrors = new Map<string, PendingApproval[]>()
  return (event, sessionId) => {
    const prev = mirrors.get(sessionId) ?? []
    const next = pendingApprovalsFromTurnState(event.new_value)
    mirrors.set(sessionId, next)
    const { added } = diffPending(prev, next)
    return added.map((entry) => ({
      kind: 'fcall-start' as const,
      functionId: entry.function_id,
      input: entry.args,
      pendingApproval: true,
      functionCallId: entry.function_call_id,
      sessionId,
    }))
  }
}

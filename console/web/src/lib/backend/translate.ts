/**
 * Pure translator from iii `AgentEvent` (the wire shape on
 * `agent::events`) to console/web's `StreamEvent` contract documented in
 * `PLAYGROUND.md`.
 *
 * Phase 2.A: `turn-orchestrator` emits `message_update` events with a
 * provider `AssistantMessageEvent` payload for every non-terminal frame,
 * so token-by-token streaming flows through the `message_update` branch
 * below. Terminal assistant turns emit a single `message_complete` event;
 * when `body_streamed` is false the full body is translated here, otherwise
 * only `assistant-end` (+ abnormal `stop-reason`) is emitted.
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
 *   - `agent_start` / `turn_start` / `turn_end` /
 *     `function_execution_update` → noop.
 *   - `turn_state_changed` → noop (routed through `createTurnStateTranslator`).
 */

import type {
  AgentEvent,
  AgentMessage,
  AssistantMessageEvent,
  ContentBlock,
  FunctionResult,
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

    case 'turn_state_changed':
      return []

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
          /* Soft failures are surfaced in the UI via the canonical
             `{ error: { kind, message, ... } }` shape (see PLAYGROUND.md
             "Error semantics"). The harness sends a raw FunctionResult and a
             sibling `is_error: true` flag; the canonical shape is a UI
             concern, so the wrap lives here rather than on the orchestrator
             side. Keeps the wire format provider-agnostic. */
          output: event.is_error ? wrapErrorOutput(event.result) : event.result,
          durationMs: event.duration_ms,
        },
      ]

    case 'agent_end':
      return [{ kind: 'assistant-end' }]

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

/**
 * Phase 2.A: translate a provider `AssistantMessageEvent` (carried inside
 * `AgentEvent.MessageUpdate.llm_event`) into the StreamEvent contract.
 * Non-terminal text and thinking deltas drive the renderer; everything
 * else is silently dropped — the terminal `Done`/`Error` event is
 * mirrored by `message_complete` (and ultimately by `agent_end` →
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

/*
 * Wrap a soft function-execution failure into the canonical
 * `{ error: { kind, message, details, content } }` shape consumed by the
 * group accordion's failed counter and the embedded fcall's error view.
 * `details` / `content` carry the raw payload so the expanded response
 * pane still has everything to render. `kind: 'function_error'` matches
 * the rest of the translator's deny path (`approval_resolved` uses
 * `kind: 'denied'`).
 */
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

/**
 * Pull a one-line message out of a FunctionResult's content blocks for the
 * canonical `error.message`. First non-empty text block wins; otherwise we
 * fall back to a generic string so the UI always has something to render.
 */
function deriveErrorMessage(content: ContentBlock[]): string {
  for (const block of content) {
    if (block.type === 'text' && block.text.length > 0) {
      // Collapse to a single line — the message field is the header; the
      // full multi-line payload remains under error.content / details.
      return block.text.replace(/\s+/g, ' ').trim()
    }
  }
  return 'function returned an error'
}

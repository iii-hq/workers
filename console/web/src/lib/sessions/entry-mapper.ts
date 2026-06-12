/**
 * Maps session-manager transcript entries to the console's UI message model
 * and reconciles live event snapshots into an ordered `Message[]`.
 *
 * Identity scheme (the reconciliation contract):
 * - A single-segment entry (user message, custom entry) renders as one UI
 *   message whose id IS the `entry_id`. The optimistic user message the
 *   console appends on send uses the predicted entry id
 *   (`<message_id>-user-0`, the harness's deterministic scheme), so the
 *   `message_added` snapshot replaces it in place.
 * - An assistant entry splits per content block into segments with ids
 *   `<entry_id>:<block_index>` (thinking → thought, text → assistant,
 *   function_call → function-call row). Each `message_updated` snapshot
 *   re-derives the segment list wholesale and replaces the entry's range.
 * - `function_result` entries render no row of their own; they fill the
 *   `output` of the function-call row with the matching `functionCallId`.
 * - `custom_type: "compaction"` custom entries render the compaction marker.
 *
 * Events are at-least-once and unordered: callers keep the highest
 * `revision` per entry (see use-conversations) and treat `session::messages`
 * read-backs as the source of truth.
 */

import type {
  FunctionCallMessage,
  Message,
  SystemMessage,
  UserMessage,
} from '@/types/chat'
import type { AgentMessage, ContentBlock, TranscriptItem } from './types'

export const COMPACTION_CUSTOM_TYPE = 'compaction'

/** The harness wraps every tool call in agent_trigger; unwrap for display. */
function unwrapFunctionCall(
  block: Extract<ContentBlock, { type: 'function_call' }>,
): {
  functionId: string
  input: unknown
} {
  if (
    block.function_id === 'agent_trigger' &&
    block.arguments &&
    typeof block.arguments === 'object'
  ) {
    const args = block.arguments as { function?: unknown; payload?: unknown }
    if (typeof args.function === 'string' && args.function.length > 0) {
      return { functionId: args.function, input: args.payload ?? {} }
    }
  }
  return { functionId: block.function_id, input: block.arguments }
}

function textOf(blocks: ContentBlock[]): string {
  let out = ''
  for (const block of blocks) {
    if (block.type === 'text') out += block.text
  }
  return out
}

function compactionMarker(
  entryId: string,
  data: unknown,
  timestamp: number,
): SystemMessage {
  const d = (data ?? {}) as {
    summary?: unknown
    tokens_before?: unknown
    timestamp?: unknown
  }
  const tokensBefore = typeof d.tokens_before === 'number' ? d.tokens_before : 0
  return {
    id: entryId,
    role: 'system',
    kind: 'compaction',
    content:
      tokensBefore > 0
        ? `compacted ${tokensBefore.toLocaleString()} tokens`
        : 'conversation compacted',
    tone: 'info',
    summaryText: typeof d.summary === 'string' ? d.summary : undefined,
    tokensBefore,
    createdAt: typeof d.timestamp === 'number' ? d.timestamp : timestamp,
  }
}

/**
 * Derive the UI segments for one transcript item. `function_result` entries
 * return [] — they pair into an existing function-call row instead (see
 * applyEntryUpsert).
 */
export function entrySegments(
  item: TranscriptItem,
  sessionId?: string,
): Message[] {
  if (item.custom) {
    if (item.custom.custom_type === COMPACTION_CUSTOM_TYPE) {
      return [compactionMarker(item.entry_id, item.custom.data, Date.now())]
    }
    return []
  }
  const message = item.message
  if (!message) return []

  switch (message.role) {
    case 'user': {
      const msg: UserMessage = {
        id: item.entry_id,
        role: 'user',
        content: textOf(message.content),
        createdAt: message.timestamp,
      }
      return [msg]
    }
    case 'assistant':
      return assistantSegments(item.entry_id, message, sessionId)
    case 'function_result':
      return []
    case 'custom': {
      const content = message.display ?? textOf(message.content)
      if (!content) return []
      const msg: SystemMessage = {
        id: item.entry_id,
        role: 'system',
        kind: 'notice',
        content,
        tone: 'info',
        createdAt: message.timestamp,
      }
      return [msg]
    }
  }
}

function assistantSegments(
  entryId: string,
  message: Extract<AgentMessage, { role: 'assistant' }>,
  sessionId?: string,
): Message[] {
  const out: Message[] = []
  for (const [i, block] of message.content.entries()) {
    const id = `${entryId}:${i}`
    switch (block.type) {
      case 'thinking':
        out.push({
          id,
          role: 'thought',
          content: block.text,
          durationMs: 0,
          createdAt: message.timestamp,
        })
        break
      case 'text':
        if (block.text.length === 0) break
        out.push({
          id,
          role: 'assistant',
          content: block.text,
          model: message.model,
          createdAt: message.timestamp,
        })
        break
      case 'function_call': {
        const { functionId, input } = unwrapFunctionCall(block)
        const msg: FunctionCallMessage = {
          id,
          role: 'function-call',
          functionId,
          input,
          functionCallId: block.id,
          sessionId,
          createdAt: message.timestamp,
        }
        out.push(msg)
        break
      }
      case 'image':
      case 'function_result':
        break
    }
  }
  return out
}

/** Non-error output mirrors `function_execution_end.result`. */
export function functionResultOutput(
  message: Extract<AgentMessage, { role: 'function_result' }>,
): unknown {
  if (message.is_error) {
    return {
      error: {
        kind: 'function_error',
        message:
          textOf(message.content).replace(/\s+/g, ' ').trim() ||
          'function returned an error',
        details: message.details,
        content: message.content,
      },
    }
  }
  return { content: message.content, details: message.details }
}

function belongsToEntry(messageId: string, entryId: string): boolean {
  return messageId === entryId || messageId.startsWith(`${entryId}:`)
}

/** Patchable transient state for a function-call row. */
export type FcallPatch = Partial<
  Pick<
    FunctionCallMessage,
    | 'running'
    | 'pendingApproval'
    | 'output'
    | 'durationMs'
    | 'sessionId'
    | 'functionCallId'
  >
>

/**
 * Patch the function-call row matching `functionCallId`. Returns the same
 * array when no row matched (caller may then append a fallback row).
 */
export function applyFcallPatch(
  messages: Message[],
  functionCallId: string,
  patch: FcallPatch,
): { messages: Message[]; found: boolean } {
  let found = false
  const next = messages.map((m) => {
    if (m.role !== 'function-call' || m.functionCallId !== functionCallId)
      return m
    found = true
    return { ...m, ...patch } as Message
  })
  return { messages: found ? next : messages, found }
}

/**
 * Upsert one transcript item into the ordered message list:
 * - replaces the entry's existing segment range in place (or appends at the
 *   end — appends only ever happen at the active leaf);
 * - absorbs transient state (and locally-created duplicate rows) by
 *   `functionCallId`;
 * - pairs `function_result` entries into their function-call row.
 */
export function applyEntryUpsert(
  messages: Message[],
  item: TranscriptItem,
  opts?: { sessionId?: string; streaming?: boolean },
): Message[] {
  // function_result: fill the matching call row instead of inserting.
  if (item.message?.role === 'function_result') {
    const output = functionResultOutput(item.message)
    const { messages: patched, found } = applyFcallPatch(
      messages,
      item.message.function_call_id,
      { output, running: false, pendingApproval: false },
    )
    if (found) return patched
    // Fallback (assistant snapshot lost): standalone row carrying the result.
    const row: FunctionCallMessage = {
      id: item.entry_id,
      role: 'function-call',
      functionId: item.message.function_id,
      input: undefined,
      output,
      functionCallId: item.message.function_call_id,
      sessionId: opts?.sessionId,
      createdAt: item.message.timestamp,
    }
    return [...messages, row]
  }

  let segments = entrySegments(item, opts?.sessionId)

  // Carry over transient/local state for function-call segments and drop the
  // locally-created rows they replace (agent::events fallback rows, which
  // have non-entry ids).
  const absorbedLocalIds = new Set<string>()
  segments = segments.map((segment) => {
    if (segment.role !== 'function-call' || !segment.functionCallId)
      return segment
    const existing = messages.find(
      (m): m is FunctionCallMessage =>
        m.role === 'function-call' &&
        m.functionCallId === segment.functionCallId,
    )
    if (!existing) return segment
    if (!belongsToEntry(existing.id, item.entry_id))
      absorbedLocalIds.add(existing.id)
    return {
      ...segment,
      output: existing.output,
      durationMs: existing.durationMs,
      running: existing.running,
      pendingApproval: existing.pendingApproval,
      sessionId: existing.sessionId ?? segment.sessionId,
    }
  })

  // Preserve optimistic-only fields when replacing a user message in place.
  segments = segments.map((segment) => {
    if (segment.role !== 'user') return segment
    const existing = messages.find(
      (m): m is UserMessage => m.role === 'user' && m.id === segment.id,
    )
    if (!existing?.attachments) return segment
    return { ...segment, attachments: existing.attachments }
  })

  if (opts?.streaming) {
    const last = segments[segments.length - 1]
    if (last && (last.role === 'assistant' || last.role === 'thought')) {
      segments = [
        ...segments.slice(0, -1),
        { ...last, streaming: true } as Message,
      ]
    }
  }

  const withoutAbsorbed = absorbedLocalIds.size
    ? messages.filter((m) => !absorbedLocalIds.has(m.id))
    : messages

  const firstIdx = withoutAbsorbed.findIndex((m) =>
    belongsToEntry(m.id, item.entry_id),
  )
  if (firstIdx === -1) {
    return segments.length > 0
      ? [...withoutAbsorbed, ...segments]
      : withoutAbsorbed
  }
  const before = withoutAbsorbed.slice(0, firstIdx)
  const after = withoutAbsorbed
    .slice(firstIdx)
    .filter((m) => !belongsToEntry(m.id, item.entry_id))
  return [...before, ...segments, ...after]
}

/** Full hydration: fold the active path into an ordered message list. */
export function transcriptToMessages(
  items: TranscriptItem[],
  sessionId?: string,
): Message[] {
  let messages: Message[] = []
  for (const item of items) {
    messages = applyEntryUpsert(messages, item, { sessionId })
  }
  return messages
}

/** Clear transient streaming/running flags (turn over, abort, error). */
export function clearTransientFlags(messages: Message[]): Message[] {
  let changed = false
  const next = messages.map((m) => {
    if ((m.role === 'assistant' || m.role === 'thought') && m.streaming) {
      changed = true
      return { ...m, streaming: false }
    }
    if (m.role === 'function-call' && m.running) {
      changed = true
      return { ...m, running: false }
    }
    return m
  })
  return changed ? next : messages
}

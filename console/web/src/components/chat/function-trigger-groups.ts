import type {
  AssistantMessage,
  FunctionTriggerMessage,
  Message,
} from '@/types/chat'

export type MessageListRow =
  | { kind: 'message'; message: Message }
  | {
      kind: 'function-trigger-group'
      id: string
      calls: FunctionTriggerMessage[]
      /**
       * The intermediate assistant update emitted after this call batch. It
       * reports the batch's result and announces the next phase, so it closes
       * this group while remaining normal user-visible prose.
       */
      summary?: AssistantMessage
    }

/** Calls that must remain in a collapsed group. */
export function collapsedFunctionTriggerCalls(
  calls: readonly FunctionTriggerMessage[],
  hasPersistentDisplay: (call: FunctionTriggerMessage) => boolean,
): FunctionTriggerMessage[] {
  const lastIndex = calls.length - 1
  return calls.filter(
    (call, index) =>
      index === lastIndex ||
      call.running === true ||
      call.pendingApproval === true ||
      hasPersistentDisplay(call),
  )
}

/**
 * Whether an assistant message is an intermediate phase update rather than
 * the turn's final answer. Durable transcript entries carry `stopReason`;
 * the look-ahead keeps older transcripts and hand-authored fixtures working.
 */
function isProgressUpdate(
  messages: readonly Message[],
  index: number,
  message: AssistantMessage,
): boolean {
  if (message.stopReason !== undefined)
    return message.stopReason === 'function_call'

  for (let i = index + 1; i < messages.length; i++) {
    const next = messages[i]
    if (next.role === 'thought' && !next.streaming) continue
    return next.role === 'function-trigger'
  }
  return false
}

/**
 * Collapse transcript-level function calls into phase-sized runs.
 *
 * Completed thoughts are transparent because their component deliberately
 * leaves the DOM after streaming; a streaming thought remains a boundary so
 * its live position is stable. An intermediate assistant message closes the
 * preceding run and becomes its user-visible summary. Terminal assistant
 * prose is never consumed, preserving the final answer as a normal message.
 */
export function functionTriggerGroups(
  messages: readonly Message[],
): MessageListRow[] {
  const rows: MessageListRow[] = []
  let calls: FunctionTriggerMessage[] = []

  const flushCalls = (summary?: AssistantMessage) => {
    if (calls.length === 0) return
    rows.push({
      kind: 'function-trigger-group',
      id: `function-trigger-group:${calls[0].id}`,
      calls,
      summary,
    })
    calls = []
  }

  for (const [index, message] of messages.entries()) {
    if (message.role === 'function-trigger') {
      calls.push(message)
      continue
    }

    // ThoughtMessage renders nothing once streaming finishes. Treat those
    // records as transparent so they cannot add visual gaps or split a batch.
    if (message.role === 'thought' && !message.streaming) continue

    if (
      message.role === 'assistant' &&
      message.content.trim().length > 0 &&
      calls.length > 0 &&
      isProgressUpdate(messages, index, message)
    ) {
      flushCalls(message)
      continue
    }

    flushCalls()
    rows.push({ kind: 'message', message })
  }

  flushCalls()
  return rows
}

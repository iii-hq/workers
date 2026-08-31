import type {
  AssistantMessage,
  FunctionTriggerMessage,
  Message,
  ThoughtMessage,
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
 * Thoughts are presentation interstitials rather than structural boundaries:
 * calls on either side remain in the same stable group. A visible thought stays
 * on the side of that group where it first appeared, preserving its sibling key
 * and screen position as streaming hands off to the next call. An intermediate
 * assistant message closes the preceding run and becomes its user-visible
 * summary. Terminal assistant prose is never consumed.
 */
export function functionTriggerGroups(
  messages: readonly Message[],
  visibleCompletedThoughtIds: ReadonlySet<string> = new Set(),
): MessageListRow[] {
  const rows: MessageListRow[] = []
  let calls: FunctionTriggerMessage[] = []
  let leadingThoughts: ThoughtMessage[] = []
  let trailingThoughts: ThoughtMessage[] = []

  const flushCalls = (summary?: AssistantMessage) => {
    rows.push(
      ...leadingThoughts.map(
        (message): MessageListRow => ({ kind: 'message', message }),
      ),
    )
    if (calls.length > 0) {
      rows.push({
        kind: 'function-trigger-group',
        id: `function-trigger-group:${calls[0].id}`,
        calls,
        summary,
      })
    }
    rows.push(
      ...trailingThoughts.map(
        (message): MessageListRow => ({ kind: 'message', message }),
      ),
    )
    calls = []
    leadingThoughts = []
    trailingThoughts = []
  }

  for (const [index, message] of messages.entries()) {
    if (message.role === 'function-trigger') {
      calls.push(message)
      continue
    }

    // Keep the thought as a sibling on the side where it was first observed
    // while streaming or exiting. Calls can join one stable group without
    // reparenting worker renderers, and the thought instance can animate out.
    if (message.role === 'thought') {
      if (message.streaming || visibleCompletedThoughtIds.has(message.id)) {
        const thoughts = calls.length === 0 ? leadingThoughts : trailingThoughts
        thoughts.push(message)
      }
      continue
    }

    if (
      message.role === 'assistant' &&
      message.content.trim().length > 0 &&
      calls.length > 0 &&
      isProgressUpdate(messages, index, message)
    ) {
      if (trailingThoughts.length === 0) {
        flushCalls(message)
      } else {
        // A visible thought occurred before this prose. Keep its chronological
        // position instead of attaching the prose as a group summary, which
        // MessageList intentionally renders directly after the activity group.
        flushCalls()
        rows.push({ kind: 'message', message })
      }
      continue
    }

    flushCalls()
    rows.push({ kind: 'message', message })
  }

  flushCalls()
  return rows
}

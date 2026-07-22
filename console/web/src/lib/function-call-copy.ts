/**
 * Message-level copy payloads for assistant turns. Function calls are their
 * own messages in the transcript (never embedded in the assistant record),
 * so the copy button on an assistant turn has to reach the call messages that
 * follow it — this module makes that association and serializes it.
 */
import type { FunctionCallMessage, Message } from '@/types/chat'

/** The "no meaningful value" rule the function-call panes use for `· empty`. */
function isEmptyInput(v: unknown): boolean {
  if (v === null || v === undefined) return true
  if (typeof v === 'string') return v.length === 0
  if (Array.isArray(v)) return v.length === 0
  if (typeof v === 'object') {
    return Object.keys(v as Record<string, unknown>).length === 0
  }
  return false
}

function formatJson(v: unknown): string {
  try {
    return JSON.stringify(v, null, 2)
  } catch {
    return String(v)
  }
}

/**
 * One function call as copyable plain text: `ƒ <id>` and its arguments. The
 * call is what the model emitted; the tool result (output) is copyable from
 * the call card itself and is deliberately left out of the message-level copy.
 */
export function functionCallToText(m: FunctionCallMessage): string {
  if (isEmptyInput(m.input)) return `ƒ ${m.functionId}`
  return `ƒ ${m.functionId}\n${formatJson(m.input)}`
}

/**
 * Copy payload for an assistant turn: its prose followed by every function
 * call it made, blank-line separated. With no calls the prose is returned
 * unchanged, so callers can build this unconditionally.
 */
export function assistantCopyText(
  content: string,
  calls: readonly FunctionCallMessage[],
): string {
  if (calls.length === 0) return content
  const callText = calls.map(functionCallToText).join('\n\n')
  return content ? `${content}\n\n${callText}` : callText
}

/**
 * Map each assistant message id to the function calls of its turn. Trailing
 * calls attach to the assistant message they follow; a run with no assistant
 * before it attaches FORWARD to the turn's next assistant message — the
 * canonical agent flow is thought → calls → summarizing prose, so the calls
 * usually precede the message that talks about them. Thought messages are
 * transparent; user/system messages are turn boundaries and reset both
 * directions. Between two assistant messages, trailing attribution wins.
 */
export function functionCallsByAssistant(
  messages: readonly Message[],
): Map<string, FunctionCallMessage[]> {
  const byAssistant = new Map<string, FunctionCallMessage[]>()
  let currentId: string | null = null
  let leading: FunctionCallMessage[] = []
  for (const m of messages) {
    if (m.role === 'assistant') {
      currentId = m.id
      if (leading.length > 0) {
        byAssistant.set(m.id, leading)
        leading = []
      }
    } else if (m.role === 'function-call') {
      if (currentId !== null) {
        const run = byAssistant.get(currentId)
        if (run) run.push(m)
        else byAssistant.set(currentId, [m])
      } else {
        leading.push(m)
      }
    } else if (m.role !== 'thought') {
      currentId = null
      leading = []
    }
  }
  return byAssistant
}

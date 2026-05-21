/**
 * Round-trips user + assistant text only. Function-call traces are dropped
 * because their `function_call_id` pairing doesn't survive across runs and
 * Anthropic rejects orphan tool_result blocks.
 */

import type { Message } from '@/types/chat'
import type { AgentMessage, AssistantMessage, UserMessage } from '@/types/iii-agent-event'

export function translateUiHistoryForBackend(messages: readonly Message[]): AgentMessage[] {
  // Stacked compactions: only the most recent summary needs to ship since
  // the anchored prompt in summarize.ts already folds the prior one in.
  let lastCompactIdx = -1
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i]
    if (m.role === 'system' && m.kind === 'compaction') {
      lastCompactIdx = i
      break
    }
  }

  const out: AgentMessage[] = []
  const start = lastCompactIdx >= 0 ? lastCompactIdx : 0

  if (lastCompactIdx >= 0) {
    const marker = messages[lastCompactIdx] as Extract<Message, { role: 'system' }>
    const summary = marker.summaryText ?? ''
    if (summary.length > 0) {
      // Wire-identical to harness's buildSummaryMessage.
      out.push({
        role: 'assistant',
        content: [
          {
            type: 'text',
            text: `<conversation-summary>\n${summary}\n</conversation-summary>`,
          },
        ],
        stop_reason: 'end',
        model: '',
        provider: '',
        timestamp: marker.createdAt,
      })
    }
  }

  for (let i = start + (lastCompactIdx >= 0 ? 1 : 0); i < messages.length; i++) {
    const m = messages[i]
    if (m.role === 'user') out.push(toUserMessage(m))
    else if (m.role === 'assistant') {
      const asst = toAssistantMessage(m)
      if (asst) out.push(asst)
    }
  }
  return out
}

function toUserMessage(m: Extract<Message, { role: 'user' }>): UserMessage {
  return {
    role: 'user',
    content: [{ type: 'text', text: m.content }],
    timestamp: m.createdAt,
  }
}

function toAssistantMessage(m: Extract<Message, { role: 'assistant' }>): AssistantMessage | null {
  // Empty-content assistant messages confuse providers; the wire shape
  // requires at least one text block.
  if (!m.content || m.content.length === 0) return null
  return {
    role: 'assistant',
    content: [{ type: 'text', text: m.content }],
    stop_reason: 'end',
    model: m.model ?? '',
    provider: providerForModel(m.model),
    timestamp: m.createdAt,
  }
}

// Mirrors resolveRunParams in real.ts.
function providerForModel(model: string | undefined): string {
  if (!model) return ''
  const i = model.indexOf('::')
  if (i > 0) return model.slice(0, i)
  if (model.startsWith('claude')) return 'anthropic'
  if (model.startsWith('gemini')) return 'google'
  return 'openai'
}

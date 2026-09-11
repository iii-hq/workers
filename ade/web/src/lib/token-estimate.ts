import type { Message } from '@/types/chat'

const CHARS_PER_TOKEN = 4

function charsOf(value: unknown): number {
  if (value == null) return 0
  if (typeof value === 'string') return value.length
  try {
    return JSON.stringify(value).length
  } catch {
    return 0
  }
}

export function estimateMessageTokens(message: Message): number {
  switch (message.role) {
    case 'user':
      return Math.ceil(message.content.length / CHARS_PER_TOKEN)
    case 'assistant':
      return Math.ceil(message.content.length / CHARS_PER_TOKEN)
    case 'thought':
      return Math.ceil(message.content.length / CHARS_PER_TOKEN)
    case 'function-trigger': {
      const inChars = charsOf(message.input)
      const outChars = charsOf(message.output)
      return Math.ceil((inChars + outChars) / CHARS_PER_TOKEN)
    }
    case 'system': {
      if (message.kind === 'compaction' && message.summaryText) {
        return Math.ceil(message.summaryText.length / CHARS_PER_TOKEN)
      }
      return 0
    }
  }
}

export function estimateConversationTokens(
  messages: readonly Message[],
): number {
  // Messages older than the most-recent compaction marker are no longer
  // in the server's flat-state — they were replaced by the summary the
  // marker carries. Counting them would keep the CTX bar pinned at
  // pre-compaction levels even after the server rewrote the session.
  //
  // We honour the *last* marker (compactions can stack across long
  // sessions) and count it + everything after it. The marker itself
  // contributes its summaryText tokens via estimateMessageTokens.
  let lastCompactionIdx = -1
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i]
    if (m && m.role === 'system' && m.kind === 'compaction') {
      lastCompactionIdx = i
      break
    }
  }
  const start = lastCompactionIdx >= 0 ? lastCompactionIdx : 0
  let total = 0
  for (let i = start; i < messages.length; i++) {
    total += estimateMessageTokens(messages[i])
  }
  return total
}

export function formatTokenCount(n: number): string {
  if (n < 1_000) return String(n)
  if (n < 1_000_000) {
    const k = n / 1_000
    return `${k >= 10 ? k.toFixed(0) : k.toFixed(1)}k`
  }
  const m = n / 1_000_000
  return `${m >= 10 ? m.toFixed(0) : m.toFixed(1)}M`
}

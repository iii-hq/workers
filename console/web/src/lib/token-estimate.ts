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
    case 'function-call': {
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
  let total = 0
  for (const m of messages) total += estimateMessageTokens(m)
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

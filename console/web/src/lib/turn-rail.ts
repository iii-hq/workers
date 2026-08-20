import type { Message } from '@/types/chat'

export type TurnTone = 'ink' | 'accent' | 'alert'

export interface TurnSummary {
  /** Id of the user message that opens the turn. */
  id: string
  prompt: string
  reply: string
  calls: number
  tone: TurnTone
}

/** The rail hides below this many turns; fewer is faster to scroll. */
export const TURN_RAIL_MIN_TURNS = 5
/** And below this container width, where the gutter would crowd the text. */
export const TURN_RAIL_MIN_WIDTH_PX = 640

const REPLY_PREVIEW_CHARS = 240
const PROMPT_PREVIEW_CHARS = 140

function firstLines(text: string, max: number): string {
  const flat = text.replace(/\s+/g, ' ').trim()
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat
}

function opensTurn(message: Message): boolean {
  return (
    message.role === 'user' &&
    !message.notification &&
    !message.reaction &&
    !message.validation
  )
}

/** One entry per user turn: prompt, first reply, call count, and a tone
    (alert for a failed turn, accent while the reply streams). */
export function turnsFromMessages(messages: readonly Message[]): TurnSummary[] {
  const turns: TurnSummary[] = []
  let current: TurnSummary | null = null
  let replyFound = false
  for (const message of messages) {
    if (opensTurn(message) && message.role === 'user') {
      current = {
        id: message.id,
        prompt: firstLines(message.content, PROMPT_PREVIEW_CHARS),
        reply: '',
        calls: 0,
        tone: 'ink',
      }
      replyFound = false
      turns.push(current)
      continue
    }
    if (current === null) continue
    if (message.role === 'function-trigger') {
      current.calls += 1
    } else if (message.role === 'assistant') {
      if (!replyFound && message.content.trim() !== '') {
        current.reply = firstLines(message.content, REPLY_PREVIEW_CHARS)
        replyFound = true
      }
      if (message.stopReason === 'error' || message.stopReason === 'aborted') {
        current.tone = 'alert'
      } else if (message.streaming && current.tone !== 'alert') {
        current.tone = 'accent'
      }
    } else if (message.role === 'system' && message.tone === 'error') {
      current.tone = 'alert'
    }
  }
  return turns
}

export interface ViewportSegment {
  top: number
  height: number
}

/** Where the visible window sits in the scroll range, both as fractions. */
export function viewportSegment(
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
): ViewportSegment {
  if (scrollHeight <= 0) return { top: 0, height: 1 }
  const height = Math.min(1, clientHeight / scrollHeight)
  const top = Math.min(1 - height, Math.max(0, scrollTop / scrollHeight))
  return { top, height }
}

/** The tick closest to a fraction of the rail, or -1 with no ticks. */
export function nearestTickIndex(
  fraction: number,
  ticks: readonly number[],
): number {
  let best = -1
  let bestDistance = Number.POSITIVE_INFINITY
  ticks.forEach((tick, index) => {
    const distance = Math.abs(tick - fraction)
    if (distance < bestDistance) {
      bestDistance = distance
      best = index
    }
  })
  return best
}

/** The turn the top of the viewport is in: the last tick at or above it. */
export function currentTickIndex(
  scrollTop: number,
  offsets: readonly number[],
): number {
  let current = -1
  offsets.forEach((offset, index) => {
    if (offset <= scrollTop + 1) current = index
  })
  return current
}

/** Scroll target for a step from `current`, clamped to the tick range. */
export function steppedTickIndex(
  current: number,
  delta: number,
  count: number,
): number {
  if (count === 0) return -1
  return Math.min(count - 1, Math.max(0, current + delta))
}

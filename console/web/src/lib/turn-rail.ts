import type { Message } from '@/types/chat'

export type TurnTone = 'ink' | 'accent' | 'alert'
export type TurnKind = 'user' | 'agent'

export interface TurnSummary {
  /** Id of the message the tick anchors to. */
  id: string
  kind: TurnKind
  /** User prompt for a user tick; empty for an agent tick. */
  prompt: string
  /** First reply after a user tick; the agent text for an agent tick. */
  reply: string
  /** Function calls between this tick and the next. */
  calls: number
  tone: TurnTone
}

/** The rail hides below this many ticks; fewer is faster to scroll. */
export const TURN_RAIL_MIN_TURNS = 5
/** And below this container width, where the gutter would crowd the text. */
export const TURN_RAIL_MIN_WIDTH_PX = 480

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

/** A tick per user prompt and per agent prose message, in order. Agent
    loops are mostly one prompt followed by many steps, so the agent's own
    messages are the landmarks. Calls land on the tick before them; a failed
    reply turns its tick alert, a streaming one accent. */
export function turnsFromMessages(messages: readonly Message[]): TurnSummary[] {
  const turns: TurnSummary[] = []
  let current: TurnSummary | null = null
  let lastUser: TurnSummary | null = null
  for (const message of messages) {
    if (opensTurn(message) && message.role === 'user') {
      current = {
        id: message.id,
        kind: 'user',
        prompt: firstLines(message.content, PROMPT_PREVIEW_CHARS),
        reply: '',
        calls: 0,
        tone: 'ink',
      }
      lastUser = current
      turns.push(current)
      continue
    }
    if (current === null) continue
    if (message.role === 'function-trigger') {
      current.calls += 1
      continue
    }
    if (message.role === 'system' && message.tone === 'error') {
      current.tone = 'alert'
      continue
    }
    if (message.role !== 'assistant') continue
    const text = firstLines(message.content, REPLY_PREVIEW_CHARS)
    const failed =
      message.stopReason === 'error' || message.stopReason === 'aborted'
    if (lastUser !== null && lastUser.reply === '' && text !== '') {
      lastUser.reply = text
    }
    if (text === '') {
      if (failed) current.tone = 'alert'
      else if (message.streaming && current.tone !== 'alert')
        current.tone = 'accent'
      continue
    }
    current = {
      id: message.id,
      kind: 'agent',
      prompt: '',
      reply: text,
      calls: 0,
      tone: failed ? 'alert' : message.streaming ? 'accent' : 'ink',
    }
    turns.push(current)
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

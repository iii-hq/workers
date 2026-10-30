/**
 * The context window over the conversation's model steps, read from the
 * session's assistant messages (`session::messages` with `content: "usage"`
 * and `roles: ["assistant"]`): each one carries what the provider billed for
 * that step, and the prompt it read — fresh input plus cache read and write —
 * is the window's size at that step. A drop between steps is a compaction.
 */

import { unwrapEnvelope } from '@iii-dev/console-ui/format'

export interface StepPoint {
  /** 1-based model step. */
  step: number
  /** The whole prompt the provider read: the context window at this step. */
  prompt: number
  output: number
  cost?: number
  /** Milliseconds since epoch. */
  at: number
}

interface UsageLike {
  input?: number
  output?: number
  cache_read?: number
  cache_write?: number
  cost_usd?: number
}

/** The assistant messages of one `session::messages` page, as step points
    (unnumbered — `stepPoints` numbers the whole series once sorted). */
function pagePoints(page: unknown): Omit<StepPoint, 'step'>[] {
  const raw = unwrapEnvelope(page)
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return []
  const messages = (raw as { messages?: unknown }).messages
  if (!Array.isArray(messages)) return []
  const out: Omit<StepPoint, 'step'>[] = []
  for (const item of messages) {
    const message = (item as { message?: unknown })?.message as
      | { role?: string; usage?: UsageLike; timestamp?: number }
      | undefined
    if (!message || message.role !== 'assistant') continue
    const usage = message.usage
    if (!usage) continue
    const prompt = (usage.input ?? 0) + (usage.cache_read ?? 0) + (usage.cache_write ?? 0)
    // A step without a prompt count (a stream cut before usage came back)
    // has nothing to plot.
    if (!(prompt > 0)) continue
    out.push({
      prompt,
      output: usage.output ?? 0,
      cost: usage.cost_usd ?? undefined,
      at: typeof message.timestamp === 'number' ? message.timestamp : 0,
    })
  }
  return out
}

/** Every page's assistant steps, oldest first, numbered 1..n. */
export function stepPoints(pages: unknown[]): StepPoint[] {
  const all = pages.flatMap(pagePoints)
  all.sort((a, b) => a.at - b.at)
  return all.map((point, index) => ({ ...point, step: index + 1 }))
}

/** Indexes (into the series) of steps that opened on a smaller window than
    the step before — compactions, or a model switch to a tighter budget. */
export function compactions(points: readonly StepPoint[], dropShare = 0.2): number[] {
  const out: number[] = []
  for (let i = 1; i < points.length; i++) {
    const prev = points[i - 1].prompt
    if (prev > 0 && points[i].prompt < prev * (1 - dropShare)) out.push(i)
  }
  return out
}

export interface Growth {
  /** Tokens the window grew per step, over the recent steps. */
  perStep: number
  /** Steps until the window is full at that pace; `null` when it is not growing. */
  stepsLeft: number | null
}

/**
 * How fast the window is filling, from the last few step-to-step changes
 * (drops — compactions — are left out, since they say nothing about pace),
 * and how many steps are left at that pace.
 */
export function growth(
  points: readonly StepPoint[],
  usable: number,
  current: number,
  window = 8,
): Growth | null {
  if (points.length < 2) return null
  const deltas: number[] = []
  for (let i = Math.max(1, points.length - window); i < points.length; i++) {
    const delta = points[i].prompt - points[i - 1].prompt
    if (delta > 0) deltas.push(delta)
  }
  if (deltas.length === 0) return { perStep: 0, stepsLeft: null }
  const perStep = Math.round(deltas.reduce((sum, d) => sum + d, 0) / deltas.length)
  const room = usable - current
  const stepsLeft = perStep > 0 && room > 0 ? Math.floor(room / perStep) : perStep > 0 ? 0 : null
  return { perStep, stepsLeft }
}

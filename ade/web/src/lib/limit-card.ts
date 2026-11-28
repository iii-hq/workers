/**
 * When the limit card shows above a Claude Code conversation's composer
 * (chat-projects §4.3.4, D135), pure:
 * - the last turn failed with `llm.usage_limit` (the failure's kind, never
 *   its text), or with `harness.unknown_account` (a pin to a removed
 *   account);
 * - or the conversation is idle and its account's reading is at 100 %
 *   (tmux's `limitHit`: the 5-hour window first, then the week).
 *
 * The X puts it away until the reading changes, as the tmux card's key does.
 * A failed card whose window has reset since stays, `reset`: "The limit has
 * reset" and "Try again"; a reading's card goes once its reset passed.
 */
import { createContext } from 'react'
import type { ClaudeAccount } from '@/lib/backend/claude-accounts'
import {
  type LimitWindow,
  type LockedAccount,
  limitHit,
} from '@/lib/claude-accounts-words'
import type { Message, SystemMessage } from '@/types/chat'

/**
 * Set around a transcript whose Claude Code account is locked, and why (a
 * side chat, D2, or a sub-agent): its limit card offers no other account,
 * and the account notices in the transcript say so too.
 */
export const LockedAccountContext = createContext<LockedAccount>(null)

/** The last turn's failure: a `turn-failure` row with nothing but notices after it. */
export function lastTurnFailure(
  messages: readonly Message[],
): SystemMessage | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i]
    if (m.role !== 'system') return null
    if (m.kind === 'turn-failure') return m
  }
  return null
}

export interface LimitCardModel {
  /** `failed`: the last turn hit the limit; `reading`: idle at 100 %; `unknown`: the pinned account is gone. */
  kind: 'failed' | 'reading' | 'unknown'
  /** The account at its limit (or the removed one). */
  account: string
  window: LimitWindow | null
  resetsAtMs: number | null
  /** A failed card whose `resetsAtMs` has passed: the same account can run again. */
  reset: boolean
  /** What the X puts away: the card for this reading. */
  key: string
}

export function limitCardModel(input: {
  sessionId: string
  /** The last turn's failure row, if it failed. */
  failure: SystemMessage | null
  /** No turn runs and nothing streams. */
  idle: boolean
  /** The account the next turn runs on. */
  resolved: string | null
  account: (id: string | null | undefined) => ClaudeAccount | null
  /** The clock the reset is read against. */
  now: number
}): LimitCardModel | null {
  const { sessionId, failure } = input
  // A turn on its way (the retry "Continue in" started) puts it away.
  if (!input.idle) return null
  const usage = failure?.failure?.usageLimit
  if (usage) {
    const account = usage.account ?? input.resolved
    if (!account) return null
    const hit = limitHit(input.account(account))
    const window = usage.window ?? hit?.window ?? null
    const resetsAtMs = usage.resetsAtMs ?? hit?.limit.resetsAtMs ?? null
    return {
      kind: 'failed',
      account,
      window,
      resetsAtMs,
      reset: resetsAtMs !== null && resetsAtMs <= input.now,
      key: [
        sessionId,
        failure?.id,
        hit?.window ?? '',
        hit?.limit.usedPercent ?? '',
        hit?.limit.resetsAtMs ?? '',
      ].join('|'),
    }
  }
  const gone = failure?.failure?.unknownAccount
  if (gone) {
    return {
      kind: 'unknown',
      account: gone,
      window: null,
      resetsAtMs: null,
      reset: false,
      key: [sessionId, failure?.id, gone].join('|'),
    }
  }
  if (!input.resolved) return null
  const hit = limitHit(input.account(input.resolved))
  if (!hit) return null
  // A reading taken before its window reset: nothing is full any more.
  if (hit.limit.resetsAtMs !== null && hit.limit.resetsAtMs <= input.now) {
    return null
  }
  return {
    kind: 'reading',
    account: input.resolved,
    window: hit.window,
    resetsAtMs: hit.limit.resetsAtMs,
    reset: false,
    key: [
      sessionId,
      input.resolved,
      hit.window,
      hit.limit.usedPercent,
      hit.limit.resetsAtMs ?? '',
    ].join('|'),
  }
}

/** Session → the key its card was put away for (this tab's memory). */
const dismissed = new Map<string, string>()

export function limitDismissed(sessionId: string, key: string): boolean {
  return dismissed.get(sessionId) === key
}

export function dismissLimit(sessionId: string, key: string): void {
  dismissed.set(sessionId, key)
}

/** Tests: forget every put-away card. */
export function resetLimitDismissalsForTests(): void {
  dismissed.clear()
}

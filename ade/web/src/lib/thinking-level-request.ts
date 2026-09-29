/**
 * A worker page asking the mounted conversation to change its reasoning
 * effort, the same shape as `working-directory-request`: the page states what
 * it wants, the ChatView that owns the session decides.
 *
 * The page cannot write this itself. The level lives on the console's
 * in-memory conversation record and is sent from there on every turn, so a
 * write straight into session metadata would not reach the next turn — the
 * console would keep sending the value it already holds.
 */

import { THINKING_LEVELS, type ThinkingLevel } from '@/types/chat'

export interface ThinkingLevelChangeRequest {
  sessionId: string
  level: ThinkingLevel
}

type ThinkingLevelChangeListener = (
  request: ThinkingLevelChangeRequest,
) => boolean

const listeners = new Set<ThinkingLevelChangeListener>()

export function requestThinkingLevelChange(
  request: ThinkingLevelChangeRequest,
): boolean {
  const normalized = {
    sessionId: request.sessionId.trim(),
    level: request.level.trim(),
  }
  if (!normalized.sessionId || !normalized.level) return false
  // Only a level the console offers. An unknown one would be stored on the
  // conversation and then sent to the provider as a reasoning effort it has
  // never heard of.
  if (!THINKING_LEVELS.includes(normalized.level)) return false

  for (const listener of [...listeners]) {
    if (listener(normalized)) return true
  }
  return false
}

export function onThinkingLevelChangeRequest(
  listener: ThinkingLevelChangeListener,
): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

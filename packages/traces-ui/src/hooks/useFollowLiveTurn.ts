/**
 * Auto-follow the chat's turns (the masthead's "follow" toggle): while
 * enabled, watch the all-spans feed for a NEW top-level turn trace of the
 * ACTIVE chat conversation and open it, so the traces canvas lands on the
 * work moments after the user sends a message.
 *
 * Detection lives in `lib/followTurn.ts` (see its docstring for the
 * delivery reality — tagged worker spans arrive on close, so novelty of
 * the trace, not span pending-ness, is the live signal). Guards:
 * - sub-agent steps (`iii.tag.kind: harness.subagent`) and other sessions'
 *   turns never qualify;
 * - traces present when following starts are baselined, not opened —
 *   unless one is still running (pending span), so a mid-turn toggle-on
 *   jumps to the live work;
 * - each turn trace opens at most once: closing or navigating away
 *   mid-turn is respected until the next turn starts.
 */

import { useEffect, useRef } from 'react'
import type { StoredSpan } from '../api/traces'
import { evaluateFollow, type FollowState } from '../lib/followTurn'

export interface FollowLiveTurnOptions {
  /** The toggle, minus pause: a paused surface must not jump around. */
  enabled: boolean
  /** Active chat conversation = session id (`null` when no chat is open). */
  activeSessionId: string | null
  /** The masthead's all-spans feed (seed + live stream). */
  spans: readonly StoredSpan[]
  selectedTraceId: string | null
  onOpenTrace: (traceId: string) => void
}

export function useFollowLiveTurn({
  enabled,
  activeSessionId,
  spans,
  selectedTraceId,
  onOpenTrace,
}: FollowLiveTurnOptions): void {
  const stateRef = useRef<FollowState | null>(null)

  useEffect(() => {
    if (!enabled || !activeSessionId) {
      // Re-enabling (or reopening a chat) starts a fresh baseline.
      stateRef.current = null
      return
    }
    const { state, openTraceId } = evaluateFollow(
      stateRef.current,
      spans,
      activeSessionId,
    )
    stateRef.current = state
    if (openTraceId && openTraceId !== selectedTraceId) {
      onOpenTrace(openTraceId)
    }
  }, [enabled, activeSessionId, spans, selectedTraceId, onOpenTrace])
}

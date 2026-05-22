import { useEffect, useRef } from 'react'
import {
  nextApprovalsToAutoResolve,
  selectAutoAcceptCandidates,
} from '@/lib/backend/auto-accept'
import { DEFAULT_POLICY, type AutoAcceptPolicy } from '@/lib/backend/auto-accept-policy'
import type { Message } from '@/types/chat'

export type ResolveApproval = (
  sessionId: string,
  functionCallId: string,
  decision: 'allow' | 'deny',
) => Promise<void>

export interface UseAutoAcceptApprovalsArgs {
  conversationId: string
  /** Whether auto-accept is currently enabled on this conversation. */
  enabled: boolean
  messages: ReadonlyArray<Message>
  /** Backend approval resolver. When undefined, the hook is a no-op. */
  resolveApproval: ResolveApproval | undefined
  /** Optional safety policy. Defaults to {@link DEFAULT_POLICY}. */
  policy?: AutoAcceptPolicy
  /** Optional callback fired after each successful auto-accept (e.g. for live-region announcement). */
  onAccepted?: (functionId: string, functionCallId: string) => void
  /** Optional callback fired when the policy refuses a pending approval. */
  onDenied?: (functionId: string, functionCallId: string) => void
}

/**
 * Hook that watches the conversation for pending function-call
 * approvals and auto-resolves them with `decision: "allow"` IF
 * `enabled` is true AND the call's function id passes the safety
 * policy. Calls that the policy refuses are left for the user to
 * resolve manually.
 *
 * Behaviour notes:
 * - Dedupe-by-ref: every auto-resolved id is recorded so a re-render
 *   of the same pending entry doesn't double-fire. The dedup set
 *   resets on `conversationId` change.
 * - Optimistic dedup: the id is added to the set BEFORE the await,
 *   then removed on rejection so a retry can fire.
 * - Cheap path: the effect bails immediately when `enabled` is false,
 *   when there's no resolver, or when the last message isn't a
 *   function-call (the only role that can introduce a pending
 *   approval). This is the hot path during token streaming.
 *
 * The selection / policy logic lives in
 * `lib/backend/auto-accept.ts` as a pure function; the hook is the
 * React-shaped wrapper over it.
 */
export function useAutoAcceptApprovals({
  conversationId,
  enabled,
  messages,
  resolveApproval,
  policy = DEFAULT_POLICY,
  onAccepted,
  onDenied,
}: UseAutoAcceptApprovalsArgs): void {
  const autoResolvedRef = useRef<Set<string>>(new Set())

  useEffect(() => {
    autoResolvedRef.current = new Set()
  }, [conversationId])

  useEffect(() => {
    if (!enabled) return
    if (!resolveApproval) return
    const selection = selectAutoAcceptCandidates(messages, policy)
    if (selection === null) return // hot-path early exit
    if (onDenied) {
      for (const d of selection.deniedByPolicy) {
        onDenied(d.functionId, d.functionCallId)
      }
    }
    const todo = nextApprovalsToAutoResolve(selection.candidates, autoResolvedRef.current)
    if (todo.length === 0) return

    for (const p of todo) {
      autoResolvedRef.current.add(p.functionCallId)
      void resolveApproval(p.sessionId, p.functionCallId, 'allow')
        .then(() => onAccepted?.(p.functionId, p.functionCallId))
        .catch((err) => {
          /* Warn-and-continue posture matches the per-card resolve
           * handler — a transient failure shouldn't break the loop
           * for the next pending entry. Remove from dedup set so a
           * retry on the next render can fire. */
          console.warn('[chat] auto-accept: approval::resolve failed', {
            functionCallId: p.functionCallId,
            sessionId: p.sessionId,
            err,
          })
          autoResolvedRef.current.delete(p.functionCallId)
        })
    }
  }, [enabled, messages, resolveApproval, policy, onAccepted, onDenied])
}

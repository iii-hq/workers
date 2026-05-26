/**
 * Pure helpers for the per-conversation "auto-accept all approvals"
 * feature. Live next to `pending-approvals-store.ts` because both are
 * framework-free helpers consumed by `useAutoAcceptApprovals`.
 *
 * Why these are pure: they encode the policy decisions (which
 * approvals are safe, which have already fired, what to skip on the
 * streaming hot path). Keeping them React-free means they can be
 * tested directly with vitest, no DOM, no testing-library.
 */

import type { Message } from '@/types/chat'
import {
  type AutoAcceptPolicy,
  DEFAULT_POLICY,
  isAutoAcceptable,
} from './auto-accept-policy'

export interface PendingApprovalCandidate {
  /** iii `function_call_id` — the dedup key. */
  functionCallId: string
  /** iii `session_id` — needed for the `approval::resolve` payload. */
  sessionId: string
  /** iii function id — exposed so callers can announce / log which call resolved. */
  functionId: string
}

/**
 * Identify which messages are auto-acceptable pending approvals.
 *
 * Walks `messages` once and applies the safety policy. Returns
 * EITHER:
 *  - `{ candidates: [...], deniedByPolicy: [...] }` when the tail is
 *    a function-call message (the only role that can introduce a
 *    pending approval); or
 *  - `null` when the tail isn't a function-call (the hot-path early
 *    exit during token streaming).
 *
 * The dedup-against-already-resolved step is left to the caller so
 * the policy filter and the dedup set can live in separate concerns.
 */
export function selectAutoAcceptCandidates(
  messages: ReadonlyArray<Message>,
  policy: AutoAcceptPolicy = DEFAULT_POLICY,
): {
  candidates: PendingApprovalCandidate[]
  deniedByPolicy: PendingApprovalCandidate[]
} | null {
  const last = messages.length > 0 ? messages[messages.length - 1] : null
  if (!last || last.role !== 'function-call') return null

  const candidates: PendingApprovalCandidate[] = []
  const deniedByPolicy: PendingApprovalCandidate[] = []
  for (const m of messages) {
    if (
      m.role !== 'function-call' ||
      m.pendingApproval !== true ||
      typeof m.functionCallId !== 'string' ||
      typeof m.sessionId !== 'string'
    ) {
      continue
    }
    const entry: PendingApprovalCandidate = {
      functionCallId: m.functionCallId,
      sessionId: m.sessionId,
      functionId: m.functionId,
    }
    if (!isAutoAcceptable(m.functionId, policy)) {
      deniedByPolicy.push(entry)
      continue
    }
    candidates.push(entry)
  }
  return { candidates, deniedByPolicy }
}

/**
 * Return the subset of `pending` whose `functionCallId` is NOT in
 * `alreadyResolved`. The caller is expected to add each returned id
 * to its dedup set BEFORE awaiting the resolve, so a re-render of
 * the same pending entry doesn't fire a second resolve.
 */
export function nextApprovalsToAutoResolve(
  pending: ReadonlyArray<PendingApprovalCandidate>,
  alreadyResolved: ReadonlySet<string>,
): PendingApprovalCandidate[] {
  return pending.filter((p) => !alreadyResolved.has(p.functionCallId))
}

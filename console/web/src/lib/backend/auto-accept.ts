/**
 * Pure helper for the per-conversation "auto-accept all approvals"
 * feature. Given the conversation's currently-pending approvals plus a
 * set of approval ids we've already auto-resolved, returns the subset
 * we still need to fire `approval::resolve` for.
 *
 * Lives next to `pending-approvals-store.ts` because both are
 * framework-free helpers consumed by `ChatView`'s React effects.
 *
 * The dedup is required because the effect re-runs on every
 * `conversation.messages` change — without it, the same approval
 * would be auto-fired repeatedly until the orchestrator finally
 * removes the pending entry from state.
 */

export interface PendingApprovalCandidate {
  /** iii `function_call_id` — the dedup key. */
  functionCallId: string
  /** iii `session_id` — needed for the `approval::resolve` payload. */
  sessionId: string
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

import {
  isWorkerPresent,
  useWorkerPresence,
  type WorkerPresence,
} from './use-worker-presence'

/**
 * Presence probe for the standalone `approval-gate` worker. The gate owns the
 * `approval::*` functions; it is OPTIONAL — the console must run cleanly with or
 * without it. This hook answers a single question: "is `approval-gate`
 * connected right now?" so callers can gate the approval UI + RPC on it. Thin
 * wrapper over the generic worker-presence probe.
 *
 * Unlike `useHarnessStatus`, there is no install CTA: we never push the gate on
 * the operator.
 */

/** Engine worker name for the standalone approval-gate worker. */
const APPROVAL_GATE_WORKER_NAME = 'approval-gate'
/** Base id for the browser-local handler bound to the `worker` trigger. */
const APPROVAL_GATE_WATCH_FN = 'console::approval-gate-watch'

export type ApprovalGateStatus = WorkerPresence

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats the gate as present so the approval UI shows
 *   in isolation, matching the mock fixtures).
 */
export function useApprovalGateStatus(enabled: boolean): ApprovalGateStatus {
  return useWorkerPresence({
    workerName: APPROVAL_GATE_WORKER_NAME,
    watchFnId: APPROVAL_GATE_WATCH_FN,
    enabled,
  })
}

/**
 * Whether the approval-gate's `approval::*` functions are registered and safe to
 * trigger. False during the initial presence probe and while the gate is absent.
 */
export function isApprovalGateAvailable(status: ApprovalGateStatus): boolean {
  return isWorkerPresent(status)
}

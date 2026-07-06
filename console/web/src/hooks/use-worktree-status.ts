import {
  isWorkerPresent,
  useWorkerPresence,
  type WorkerPresence,
} from './use-worker-presence'

/**
 * Presence probe for the `worktree` worker. Worktree owns the per-repo
 * isolated-checkout registry: the `worktree::*` picker section, the working
 * directory badge, claim/release, and the landed / land-blocked live events.
 * It is OPTIONAL, so the console gates that whole surface on its presence
 * rather than rendering controls that would call functions that don't exist.
 * Thin wrapper over the generic worker-presence probe.
 */

/** Engine worker name for the worktree worker. */
const WORKTREE_WORKER_NAME = 'worktree'
/** Base id for the browser-local handler bound to the `worker` trigger. */
const WORKTREE_WATCH_FN = 'console::worktree-watch'

export type WorktreeStatus = WorkerPresence

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats worktree as present so its UI shows in
 *   isolation).
 */
export function useWorktreeStatus(enabled: boolean): WorktreeStatus {
  return useWorkerPresence({
    workerName: WORKTREE_WORKER_NAME,
    watchFnId: WORKTREE_WATCH_FN,
    enabled,
  })
}

/**
 * Whether the worktree worker's registry functions are registered and safe
 * to trigger. False during the initial presence probe and while the worker
 * is absent.
 */
export function isWorktreeAvailable(status: WorktreeStatus): boolean {
  return isWorkerPresent(status)
}

export { WORKTREE_WATCH_FN, WORKTREE_WORKER_NAME }

import {
  isWorkerPresent,
  useWorkerPresence,
  type WorkerPresence,
} from './use-worker-presence'

/**
 * Presence probe for the `iii-directory` worker. It serves the
 * filesystem-backed prompt library (`directory::prompts::*`) behind the
 * chat's system-prompt picker. It is OPTIONAL, so the console gates the
 * picker on its presence rather than rendering controls that would call
 * functions that don't exist. Thin wrapper over the generic
 * worker-presence probe.
 */

/** Engine worker name for the directory worker. */
const DIRECTORY_WORKER_NAME = 'iii-directory'
/** Base id for the browser-local handler bound to the `worker` trigger. */
const DIRECTORY_WATCH_FN = 'console::directory-watch'

export type DirectoryStatus = WorkerPresence

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats the worker as present so its UI shows in
 *   isolation).
 */
export function useDirectoryStatus(enabled: boolean): DirectoryStatus {
  return useWorkerPresence({
    workerName: DIRECTORY_WORKER_NAME,
    watchFnId: DIRECTORY_WATCH_FN,
    enabled,
  })
}

/**
 * Whether the directory worker's functions are registered and safe to
 * trigger. False during the initial presence probe and while the worker is
 * absent.
 */
export function isDirectoryAvailable(status: DirectoryStatus): boolean {
  return isWorkerPresent(status)
}

export { DIRECTORY_WATCH_FN, DIRECTORY_WORKER_NAME }

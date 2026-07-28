import {
  isWorkerPresent,
  useWorkerPresence,
  type WorkerPresence,
} from './use-worker-presence'

/**
 * Presence probe for the directory worker, which serves the prompt store
 * (`directory::prompts::*`): worker-shipped slash templates plus the user
 * prompt library. OPTIONAL, so the console gates the Prompts page on its
 * presence rather than rendering controls that would call functions that
 * don't exist. Thin wrapper over the generic worker-presence probe.
 */

/** Engine worker name for the directory worker. */
const PROMPTS_WORKER_NAME = 'iii-directory'
/** Base id for the browser-local handler bound to the `worker` trigger. */
const PROMPTS_WATCH_FN = 'console::prompts-watch'

export type PromptsStatus = WorkerPresence

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats the worker as present so its UI shows
 *   in isolation).
 */
export function usePromptsStatus(enabled: boolean): PromptsStatus {
  return useWorkerPresence({
    workerName: PROMPTS_WORKER_NAME,
    watchFnId: PROMPTS_WATCH_FN,
    enabled,
  })
}

/**
 * Whether the directory worker's prompt functions are registered and safe
 * to trigger. False during the initial presence probe and while the worker
 * is absent.
 */
export function isPromptsAvailable(status: PromptsStatus): boolean {
  return isWorkerPresent(status)
}

export { PROMPTS_WATCH_FN, PROMPTS_WORKER_NAME }

import {
  isWorkerPresent,
  useWorkerPresence,
  type WorkerPresence,
} from './use-worker-presence'

/**
 * Presence probe for the `memory` worker. Memory owns cross-session agent
 * memory: named banks of always-injected markdown rules plus auto-extracted
 * memories (`memory::*`). It is OPTIONAL, so the console gates the Memory page
 * on its presence rather than rendering controls that would call functions
 * that don't exist. Thin wrapper over the generic worker-presence probe.
 */

/** Engine worker name for the memory worker. */
const MEMORY_WORKER_NAME = 'memory'
/** Base id for the browser-local handler bound to the `worker` trigger. */
const MEMORY_WATCH_FN = 'console::memory-watch'

export type MemoryStatus = WorkerPresence

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats memory as present so its UI shows in
 *   isolation).
 */
export function useMemoryStatus(enabled: boolean): MemoryStatus {
  return useWorkerPresence({
    workerName: MEMORY_WORKER_NAME,
    watchFnId: MEMORY_WATCH_FN,
    enabled,
  })
}

/**
 * Whether the memory worker's functions are registered and safe to trigger.
 * False during the initial presence probe and while the worker is absent.
 */
export function isMemoryAvailable(status: MemoryStatus): boolean {
  return isWorkerPresent(status)
}

export { MEMORY_WATCH_FN, MEMORY_WORKER_NAME }

import {
  isWorkerPresent,
  useWorkerPresence,
  type WorkerPresence,
} from './use-worker-presence'

/**
 * Presence probe for the `browser` worker. Browser owns interactive
 * Chromium sessions: the Browser page (viewport, console feed, pick-to-chat)
 * and the chat's browser function-call views. It is OPTIONAL, so the console
 * gates that whole surface on its presence rather than rendering controls
 * that would call functions that don't exist. Thin wrapper over the generic
 * worker-presence probe.
 */

/** Engine worker name for the browser worker. */
const BROWSER_WORKER_NAME = 'browser'
/** Base id for the browser-local handler bound to the `worker` trigger. */
const BROWSER_WATCH_FN = 'console::browser-watch'

export type BrowserStatus = WorkerPresence

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats browser as present so its UI shows in
 *   isolation).
 */
export function useBrowserStatus(enabled: boolean): BrowserStatus {
  return useWorkerPresence({
    workerName: BROWSER_WORKER_NAME,
    watchFnId: BROWSER_WATCH_FN,
    enabled,
  })
}

/**
 * Whether the browser worker's session functions are registered and safe
 * to trigger. False during the initial presence probe and while the worker
 * is absent.
 */
export function isBrowserAvailable(status: BrowserStatus): boolean {
  return isWorkerPresent(status)
}

export { BROWSER_WATCH_FN, BROWSER_WORKER_NAME }

import {
  isWorkerPresent,
  useWorkerPresence,
  type WorkerPresence,
} from './use-worker-presence'

/**
 * Presence probe for the `shell` worker. Shell owns the chat's working-directory
 * surface: the `shell::workspace::*` picker control plane and the `shell::*` /
 * `coder::*` calls the chosen dir scopes. It is
 * OPTIONAL, so the console gates the working-directory picker + banner on its
 * presence rather than rendering controls that would call functions that don't
 * exist. Thin wrapper over the generic worker-presence probe.
 */

/** Engine worker name for the shell worker. */
const SHELL_WORKER_NAME = 'shell'
/** Base id for the browser-local handler bound to the `worker` trigger. */
const SHELL_WATCH_FN = 'console::shell-watch'

export type ShellStatus = WorkerPresence

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats shell as present so the picker shows in
 *   isolation).
 */
export function useShellStatus(enabled: boolean): ShellStatus {
  return useWorkerPresence({
    workerName: SHELL_WORKER_NAME,
    watchFnId: SHELL_WATCH_FN,
    enabled,
  })
}

/**
 * Whether the shell worker's working-directory functions are registered and
 * safe to trigger. False during the initial presence probe and while shell is
 * absent.
 */
export function isShellAvailable(status: ShellStatus): boolean {
  return isWorkerPresent(status)
}

import {
  isWorkerPresent,
  useWorkerPresence,
  type WorkerPresence,
} from './use-worker-presence'

/**
 * Presence probe for the `github` worker. It wraps the GitHub CLI (`gh`) as
 * typed `github::*` functions (pull requests, issues, Actions runs, releases,
 * search). It is OPTIONAL, so the console gates the Github page on its
 * presence rather than rendering controls that would call functions that
 * don't exist. Thin wrapper over the generic worker-presence probe.
 */

/** Engine worker name for the github worker. */
const GITHUB_WORKER_NAME = 'github'
/** Base id for the browser-local handler bound to the `worker` trigger. */
const GITHUB_WATCH_FN = 'console::github-watch'

export type GithubStatus = WorkerPresence

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats github as present so its UI shows in
 *   isolation).
 */
export function useGithubStatus(enabled: boolean): GithubStatus {
  return useWorkerPresence({
    workerName: GITHUB_WORKER_NAME,
    watchFnId: GITHUB_WATCH_FN,
    enabled,
  })
}

/**
 * Whether the github worker's functions are registered and safe to trigger.
 * False during the initial presence probe and while the worker is absent.
 */
export function isGithubAvailable(status: GithubStatus): boolean {
  return isWorkerPresent(status)
}

export { GITHUB_WATCH_FN, GITHUB_WORKER_NAME }

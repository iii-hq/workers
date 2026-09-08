import { useWorkerPresence, type WorkerPresence } from './use-worker-presence'

/**
 * Presence probe for the `session-manager` worker — the durable transcript
 * store behind `session::list` / `session::get` / `session::messages` and the
 * `session::*` live triggers. It is OPTIONAL: a project that runs the engine
 * with `http`, `state` and a console but no agent stack has no
 * session-manager, and every conversation read the console made there
 * answered `function_not_found` — then retried on a fixed timer, forever. The
 * conversation layer gates its server wiring on this probe instead.
 */

const SESSION_MANAGER_WORKER_NAME = 'session-manager'
const SESSION_MANAGER_WATCH_FN = 'console::session-manager-watch'

export type SessionManagerStatus = WorkerPresence

/**
 * @param enabled - only run against the real backend; pass `false` for the
 *   mock/Storybook backend (treats the worker as present).
 */
export function useSessionManagerStatus(
  enabled: boolean,
): SessionManagerStatus {
  return useWorkerPresence({
    workerName: SESSION_MANAGER_WORKER_NAME,
    watchFnId: SESSION_MANAGER_WATCH_FN,
    enabled,
  })
}

/**
 * Whether the conversation store may talk to session-manager. Optimistic
 * while the initial probe is in flight (a real stack boots with it present,
 * and flipping the store off and on again would restart its wiring for
 * nothing); false only once the probe has said the worker is absent.
 */
export function isSessionManagerAvailable(
  status: SessionManagerStatus,
): boolean {
  return status.loading || status.present
}

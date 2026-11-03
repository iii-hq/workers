/**
 * gantry::on-verdict: a private function whose handler returns the verdict
 * event it is called with. Every completed gantry::verify calls it once,
 * fire-and-forget, while anything observes it, and iii delivers the event to
 * each observer. Best-effort: a failing observer never fails gantry::verify.
 */
import { CallAction } from 'iii-sdk';

export const ON_VERDICT_FN = 'gantry::on-verdict';

/** @typedef {{ status: string, error_code?: string | null, repo_root?: string | null, msn_id?: string | null, mission_rel_path?: string | null }} VerdictEvent */

/** The event observers receive: every field present, `null` when unknown. */
export function toVerdictEvent(event) {
  return {
    status: event.status,
    error_code: event.error_code ?? null,
    repo_root: event.repo_root ?? null,
    msn_id: event.msn_id ?? null,
    mission_rel_path: event.mission_rel_path ?? null,
  };
}

/**
 * `call` is the worker's `call`, `isObserved` answers whether
 * gantry::on-verdict has an active observer (one atomic load in the SDK).
 */
export function createVerdictEmitter({ call, isObserved }) {
  /** @param {VerdictEvent} event */
  async function emit(event) {
    if (!isObserved()) return;
    try {
      await call({
        function_id: ON_VERDICT_FN,
        payload: toVerdictEvent(event),
        action: CallAction.Void(),
      });
    } catch {
      /* best-effort: the verify result is authoritative */
    }
  }

  return {
    /** The handler of gantry::on-verdict: observers observe what it returns. */
    handler: (event) => event,
    emit,
  };
}

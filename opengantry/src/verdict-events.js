/**
 * gantry::verdict trigger fan-out. The trigger-type owner tracks bindings
 * and delivers events via worker.trigger() — best-effort; a failing subscriber
 * never fails gantry::verify.
 */

/** @typedef {{ status: string, error_code?: string | null, repo_root?: string | null, msn_id?: string | null, mission_rel_path?: string | null }} VerdictEvent */

export function createVerdictEmitter({ trigger }) {
  /** @type {Map<string, { id: string, function_id: string }>} */
  const bindings = new Map();

  async function registerTrigger(config) {
    bindings.set(config.id, {
      id: config.id,
      function_id: config.function_id,
    });
  }

  async function unregisterTrigger(config) {
    bindings.delete(config.id);
  }

  /** @param {VerdictEvent} event */
  async function emit(event) {
    const payload = {
      status: event.status,
      error_code: event.error_code ?? null,
      repo_root: event.repo_root ?? null,
      msn_id: event.msn_id ?? null,
      mission_rel_path: event.mission_rel_path ?? null,
    };
    for (const binding of bindings.values()) {
      try {
        await trigger({ function_id: binding.function_id, payload });
      } catch {
        /* best-effort fan-out */
      }
    }
  }

  return {
    handler: { registerTrigger, unregisterTrigger },
    emit,
  };
}

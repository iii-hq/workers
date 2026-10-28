/**
 * gantry::verdict trigger fan-out. The trigger-type owner tracks bindings
 * and delivers events via worker.trigger() — best-effort; a failing subscriber
 * never fails gantry::verify.
 *
 * On an iii 0.25 engine the same events are also delivered to observers of the
 * owner-delivered observable function `gantry::on-verdict`: observers are filed
 * in the same bindings map (keyed apart from binding ids), so one emit reaches
 * bindings and observers alike, through `obs.call`. The trigger type stays for
 * 0.24 engines and existing consumers; invoking `gantry::on-verdict` directly
 * is refused with FORBIDDEN.
 */
import { InvocationError } from 'iii-sdk';

export const VERDICT_TRIGGER = 'gantry::verdict';
export const VERDICT_OBSERVABLE = 'gantry::on-verdict';

/**
 * The observers' args: the gantry::verdict trigger config, which takes no
 * fields (every binding receives every verdict), so any object is accepted.
 */
export const VERDICT_ARGS_FORMAT = {
  type: 'object',
  properties: {},
  description:
    'Same as the gantry::verdict trigger config: no fields; every observer receives every verdict.',
};

const FORBIDDEN_MESSAGE = `${VERDICT_OBSERVABLE} is an event source: observe it (iii.observe({ function_id: '${VERDICT_OBSERVABLE}', args: {} }, target)); only the opengantry worker emits its events`;

/** Observers share the bindings map under a key no trigger binding id uses. */
const observerKey = (observerId) => `observer:${observerId}`;

/** @typedef {{ status: string, error_code?: string | null, repo_root?: string | null, msn_id?: string | null, mission_rel_path?: string | null }} VerdictEvent */

export function createVerdictEmitter({ trigger }) {
  /** @type {Map<string, { id: string, function_id: string, observer?: { call: (payload: unknown) => Promise<unknown> } }>} */
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

  /** The gantry::on-verdict function handler: events are routed in-process, never through it. */
  async function onVerdict() {
    throw new InvocationError({
      code: 'FORBIDDEN',
      message: FORBIDDEN_MESSAGE,
      function_id: VERDICT_OBSERVABLE,
    });
  }

  /** The `observe` option of gantry::on-verdict (iii 0.25). */
  const observe = {
    delivery: 'owner',
    args_format: VERDICT_ARGS_FORMAT,
    async onObserverAdded(obs) {
      const key = observerKey(obs.id);
      bindings.set(key, { id: key, function_id: obs.target, observer: obs });
    },
    async onObserverRemoved(obs) {
      bindings.delete(observerKey(obs.id));
    },
  };

  /** @param {VerdictEvent} event */
  async function emit(event) {
    const payload = {
      status: event.status,
      error_code: event.error_code ?? null,
      repo_root: event.repo_root ?? null,
      msn_id: event.msn_id ?? null,
      mission_rel_path: event.mission_rel_path ?? null,
    };
    for (const binding of Array.from(bindings.values())) {
      try {
        if (binding.observer) {
          await binding.observer.call(payload);
        } else {
          await trigger({ function_id: binding.function_id, payload });
        }
      } catch {
        /* best-effort fan-out (FILTERS_NOT_MATCHED / OBSERVER_NOT_ACTIVE included) */
      }
    }
  }

  return {
    handler: { registerTrigger, unregisterTrigger },
    onVerdict,
    observe,
    emit,
  };
}

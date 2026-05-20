/**
 * Reactive abort wake. A `state` trigger on `scope: 'agent'` filtered by
 * the abort_signal key shape (`session/<id>/abort_signal`) and a
 * `new_value === true` write fires this adapter, which publishes
 * `turn::step_requested` so the orchestrator's FSM advances to
 * `steering_check` and observes the abort flag promptly.
 *
 * Without this wake, a session mid-streaming would only check
 * `abort_signal` after the current step completes naturally. The reactive
 * trigger doesn't preempt the running step (durable subscriber publishes
 * queue), but it guarantees the orchestrator runs another FSM step as
 * soon as the current one finishes — which is the earliest moment we
 * can safely react.
 *
 * Mirror of the canonical pattern in
 * `harness-node/src/harness/fanout/sessions-poll.ts`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';

const ABORT_SIGNAL_KEY_RE = /^session\/([^/]+)\/abort_signal$/;

export function isAbortSignalWrite(event: unknown): boolean {
  if (!event || typeof event !== 'object') return false;
  const obj = event as Record<string, unknown>;
  if (obj.event_type !== 'state:created' && obj.event_type !== 'state:updated') return false;
  if (obj.new_value !== true) return false;
  const key = obj.key;
  if (typeof key !== 'string') return false;
  return ABORT_SIGNAL_KEY_RE.test(key);
}

function extractSessionId(key: string): string | null {
  const m = ABORT_SIGNAL_KEY_RE.exec(key);
  return m ? (m[1] ?? null) : null;
}

export async function handleAbortSignalWrite(iii: ISdk, event: unknown): Promise<void> {
  if (!event || typeof event !== 'object') return;
  const obj = event as Record<string, unknown>;
  const key = obj.key;
  if (typeof key !== 'string') return;
  const session_id = extractSessionId(key);
  if (!session_id) return;

  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'iii::durable::publish',
      payload: { topic: 'turn::step_requested', data: { session_id } },
    });
  } catch (err) {
    logger.warn('turn::on_abort_signal: publish failed', {
      session_id,
      err: String(err),
    });
  }
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::is_abort_signal_set',
    async (event: unknown) => isAbortSignalWrite(event),
    {
      description:
        'Condition: state event sets session/<id>/abort_signal = true (state:created or state:updated).',
    },
  );

  iii.registerFunction(
    'turn::on_abort_signal',
    async (event: unknown) => handleAbortSignalWrite(iii, event),
    {
      description:
        'State trigger adapter on scope=agent for abort_signal writes; publishes turn::step_requested so the orchestrator picks up the abort promptly.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: 'turn::on_abort_signal',
    config: {
      scope: 'agent',
      condition_function_id: 'turn::is_abort_signal_set',
    },
  });
}

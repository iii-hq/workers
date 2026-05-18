/**
 * `state` trigger adapter. Registered against `scope: 'approvals'` so it fires
 * on every decision write. Extracts the session id from the `<sid>/<cid>` key
 * and wakes the orchestrator through the normal turn step topic.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';

export const STEP_TOPIC = 'turn::step_requested';
export const TRIGGER_FN_ID = 'approval::on_decision_written';

export async function handleDecisionWritten(iii: ISdk, event: unknown): Promise<void> {
  if (!event || typeof event !== 'object') return;
  const obj = event as Record<string, unknown>;
  if (obj.event_type === 'deleted') return;

  const key = obj.key;
  if (typeof key !== 'string' || key.length === 0) return;
  const slash = key.indexOf('/');
  if (slash <= 0) return;
  const session_id = key.slice(0, slash);

  try {
    await iii.trigger<unknown, unknown>({
      function_id: 'iii::durable::publish',
      payload: { topic: STEP_TOPIC, data: { session_id } },
    });
  } catch (err) {
    logger.warn('approval::on_decision_written: publish failed', {
      session_id,
      err: String(err),
    });
  }
}

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';

export const STEP_TOPIC = 'turn::step_requested';
export const TRIGGER_FN_ID = 'approval::on_decision_written';
export const CONDITION_FN_ID = 'approval::is_decision_write';

export function isDecisionWrite(event: unknown): boolean {
  if (!event || typeof event !== 'object') return false;
  const obj = event as Record<string, unknown>;
  if (obj.event_type !== 'state:created' && obj.event_type !== 'state:updated') return false;
  const nv = obj.new_value;
  if (!nv || typeof nv !== 'object') return false;
  return typeof (nv as Record<string, unknown>).decision === 'string';
}

export async function handleDecisionWritten(iii: ISdk, event: unknown): Promise<void> {
  if (!event || typeof event !== 'object') return;
  const obj = event as Record<string, unknown>;

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

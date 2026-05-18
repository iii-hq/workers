/**
 * `state` trigger adapter. Registered against `scope: 'approvals'` so it fires
 * on every decision write. Extracts the session id from the `<sid>/<cid>` key
 * and wakes the orchestrator by invoking turn::step directly.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';

export const STEP_FN_ID = 'turn::step';
export const TRIGGER_FN_ID = 'approval::on_decision_written';
export const CONDITION_FN_ID = 'approval::is_decision_write';

/**
 * Pure condition. Returns true only when a state write to scope=approvals
 * looks like a real decision: event_type in {state:created, state:updated},
 * and new_value carries a `decision` string.
 *
 * Bound to the approvals state trigger via `condition_function_id`. The
 * engine runs this server-side before invoking handleDecisionWritten, so
 * unrelated writes to the scope (none today, but defensive against future
 * additions) and `state:deleted` events are filtered out at the engine.
 */
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
      function_id: STEP_FN_ID,
      payload: { session_id },
    });
  } catch (err) {
    logger.warn('approval::on_decision_written: turn::step invoke failed', {
      session_id,
      err: String(err),
    });
  }
}

/**
 * `state` trigger adapter. Registered against `scope: 'approvals'` so it
 * fires on every decision write. Extracts the session id from the
 * `<sid>/<cid>` key and wakes the orchestrator by invoking `turn::step`.
 *
 * Failure-mode contract: event handlers MUST NOT throw — the engine
 * delivers events and a thrown handler would crash the trigger dispatch
 * for everyone. Every failure path here is `logger.warn` + return.
 */

import type { ISdk } from 'iii-sdk';
import { logger } from '../runtime/otel.js';
import { STEP_FN_ID } from '../turn-orchestrator/subscriber.js';
import {
  DecisionWriteEventSchema,
  StateEventSchema,
  type TurnStepPayload,
  parsePendingKey,
} from './schemas.js';

export { STEP_FN_ID };
export const TRIGGER_FN_ID = 'approval::on_decision_written';
export const CONDITION_FN_ID = 'approval::is_decision_write';

/**
 * Pure condition. Returns true only when a state write to scope=approvals
 * looks like a real decision: event_type in {state:created, state:updated},
 * and new_value carries a `decision` string.
 *
 * Bound to the approvals state trigger via `condition_function_id`. The
 * engine runs this server-side before invoking `handleDecisionWritten`, so
 * unrelated writes and `state:deleted` events are filtered upstream.
 */
export function isDecisionWrite(event: unknown): boolean {
  return DecisionWriteEventSchema.safeParse(event).success;
}

export async function handleDecisionWritten(iii: ISdk, event: unknown): Promise<void> {
  const parsed = StateEventSchema.safeParse(event);
  if (!parsed.success) {
    logger.warn('approval::on_decision_written: malformed event', {
      err: String(parsed.error.issues[0]?.message ?? 'unknown'),
    });
    return;
  }
  const key = parsePendingKey(parsed.data.key);
  if (!key) {
    logger.warn('approval::on_decision_written: unparseable key', { key: parsed.data.key });
    return;
  }

  try {
    await iii.trigger<TurnStepPayload, void>({
      function_id: STEP_FN_ID,
      payload: { session_id: key.session_id },
    });
  } catch (err) {
    logger.warn('approval::on_decision_written: turn::step invoke failed', {
      session_id: key.session_id,
      err: String(err),
    });
  }
}

/**
 * Self-loop wake: a state trigger on `scope: 'agent'` filtered by the
 * turn_state key shape and a non-terminal, non-awaiting `new_value.state`
 * invokes `turn::step`. Saving the record is the wake — replaces the durable
 * `turn::step_requested` self-publish that used to live in `subscriber.ts`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { TurnState } from './state.js';

export const STEP_FN_ID = 'turn::step';
export const HANDLER_FN_ID = 'turn::on_record_written';
export const CONDITION_FN_ID = 'turn::is_stepable_record_write';

const TURN_STATE_KEY_RE = /^session\/(?<session_id>[^/]+)\/turn_state$/;

const NON_STEPABLE_STATES: ReadonlySet<TurnState> = new Set([
  'stopped',
  'function_awaiting_approval',
]);

type StepableWrite = {
  session_id: string;
  state: TurnState;
};

/**
 * One source of truth for "is this a stepable turn_state write?". Returns the
 * extracted session_id + state on match, null otherwise. Both the condition
 * (boolean check) and the handler (needs the session_id) route through this,
 * so they can't drift — and the handler can't fire on a parking-state write
 * even if the condition was bypassed.
 */
function parseStepableWrite(event: unknown): StepableWrite | null {
  if (!event || typeof event !== 'object') return null;
  const obj = event as Record<string, unknown>;

  if (obj.event_type !== 'state:created' && obj.event_type !== 'state:updated') return null;

  const key = obj.key;
  if (typeof key !== 'string') return null;
  const session_id = TURN_STATE_KEY_RE.exec(key)?.groups?.session_id;
  if (!session_id) return null;

  const nv = obj.new_value;
  if (!nv || typeof nv !== 'object') return null;
  const state = (nv as Record<string, unknown>).state;
  if (typeof state !== 'string') return null;
  if (NON_STEPABLE_STATES.has(state as TurnState)) return null;

  return { session_id, state: state as TurnState };
}

export function isStepableRecordWrite(event: unknown): boolean {
  return parseStepableWrite(event) !== null;
}

export async function handleStepableRecordWrite(iii: ISdk, event: unknown): Promise<void> {
  const parsed = parseStepableWrite(event);
  if (!parsed) return;

  try {
    await iii.trigger<unknown, unknown>({
      function_id: STEP_FN_ID,
      payload: { session_id: parsed.session_id },
    });
  } catch (err) {
    logger.warn('turn::on_record_written: turn::step invoke failed', {
      session_id: parsed.session_id,
      err: String(err),
    });
  }
}

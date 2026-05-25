/**
 * Durable FSM wake via iii-queue FIFO `turn-step`. Enqueues `turn::{state}` per
 * persisted turn_state, not a generic dispatcher.
 */

import { TriggerAction, type ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import * as persistence from './persistence.js';
import { type TurnState } from './state.js';

export const TURN_STEP_QUEUE = 'turn-step';

const NON_STEPABLE_STATES = new Set<TurnState>(['stopped', 'failed', 'function_awaiting_approval']);

/** True when a persisted turn_state transition should enqueue `turn::{newState}`. */
export function shouldWakeStep(previousState: TurnState | null, newState: TurnState): boolean {
  if (NON_STEPABLE_STATES.has(newState)) return false;
  if (previousState !== null && previousState === newState) return false;
  return true;
}


export async function wakeState(iii: ISdk, session_id: string, state: TurnState): Promise<void> {
  try {
    await iii.trigger({
      function_id: `turn::${state}`,
      payload: { session_id },
      action: TriggerAction.Enqueue({ queue: TURN_STEP_QUEUE }),
    });
  } catch (err) {
    logger.warn('wakeState failed', { session_id, state, err: String(err) });
  }
}

/** Enqueue the handler for the session's current persisted state (approval/abort). */
export async function wakeFromRecord(iii: ISdk, session_id: string): Promise<void> {
  const rec = await persistence.loadRecord(iii, session_id);
  if (!rec || rec.state === 'stopped' || rec.state === 'failed') return;
  await wakeState(iii, session_id, rec.state);
}

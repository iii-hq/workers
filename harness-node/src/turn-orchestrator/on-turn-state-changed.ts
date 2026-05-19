/**
 * State-trigger adapter that mirrors `on-record-written` but emits a
 * `turn_state_changed` agent event instead of triggering `turn::step`.
 * Gives the frontend a live signal carrying the new turn_state record
 * so it can derive pending approvals from state, not from an
 * approval_requested event.
 */

import type { ISdk } from '../runtime/iii.js';
import { emit } from './events.js';

export const HANDLER_FN_ID = 'turn::on_turn_state_changed';
export const CONDITION_FN_ID = 'turn::is_turn_state_write';

const TURN_STATE_KEY_RE = /^session\/(?<session_id>[^/]+)\/turn_state$/;

type ParsedWrite = {
  session_id: string;
  event_type: 'state:created' | 'state:updated';
  new_value: Record<string, unknown>;
  old_value?: Record<string, unknown>;
};

function parseWrite(event: unknown): ParsedWrite | null {
  if (!event || typeof event !== 'object') return null;
  const obj = event as Record<string, unknown>;
  if (obj.event_type !== 'state:created' && obj.event_type !== 'state:updated') return null;
  const key = obj.key;
  if (typeof key !== 'string') return null;
  const session_id = TURN_STATE_KEY_RE.exec(key)?.groups?.session_id;
  if (!session_id) return null;
  const nv = obj.new_value;
  if (!nv || typeof nv !== 'object') return null;
  const ov = obj.old_value;
  return {
    session_id,
    event_type: obj.event_type,
    new_value: nv as Record<string, unknown>,
    old_value: ov && typeof ov === 'object' ? (ov as Record<string, unknown>) : undefined,
  };
}

export function isTurnStateWrite(event: unknown): boolean {
  return parseWrite(event) !== null;
}

export async function handleTurnStateWrite(iii: ISdk, event: unknown): Promise<void> {
  const parsed = parseWrite(event);
  if (!parsed) return;
  await emit(iii, parsed.session_id, {
    type: 'turn_state_changed',
    event_type: parsed.event_type,
    new_value: parsed.new_value,
    old_value: parsed.old_value,
  });
}

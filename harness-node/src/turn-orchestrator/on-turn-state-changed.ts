/**
 * State-trigger adapter that mirrors `on-record-written` but emits a
 * `turn_state_changed` agent event instead of triggering `turn::step`.
 * Gives the frontend a live signal carrying the new turn_state record
 * so it can derive pending approvals from state directly.
 */

import { z } from 'zod';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { emit } from './events.js';
import { turnStateKey } from './state.js';

export const HANDLER_FN_ID = 'turn::on_turn_state_changed';
export const CONDITION_FN_ID = 'turn::is_turn_state_write';

const TurnStateRecordValueSchema = z.object({ state: z.string() }).passthrough();

export const TurnStateWriteEventSchema = z.object({
  event_type: z.enum(['state:created', 'state:updated']),
  key: z.string(),
  new_value: TurnStateRecordValueSchema,
  old_value: z.unknown().optional(),
});

export type ParsedTurnStateWrite = {
  session_id: string;
  event_type: 'state:created' | 'state:updated';
  new_value: Record<string, unknown>;
  old_value?: Record<string, unknown>;
};

function sessionIdFromTurnStateKey(key: string): string | null {
  const match = /^session\/([^/]+)\/turn_state$/.exec(key);
  const session_id = match?.[1];
  if (!session_id || turnStateKey(session_id) !== key) return null;
  return session_id;
}

/** Shared parse for agent-scope turn_state create/update events. */
export function parseTurnStateWrite(event: unknown): ParsedTurnStateWrite | null {
  const parsed = TurnStateWriteEventSchema.safeParse(event);
  if (!parsed.success) return null;

  const session_id = sessionIdFromTurnStateKey(parsed.data.key);
  if (!session_id) return null;

  const old_value =
    parsed.data.old_value &&
    typeof parsed.data.old_value === 'object' &&
    parsed.data.old_value !== null
      ? (parsed.data.old_value as Record<string, unknown>)
      : undefined;

  return {
    session_id,
    event_type: parsed.data.event_type,
    new_value: parsed.data.new_value as Record<string, unknown>,
    ...(old_value !== undefined && { old_value }),
  };
}

export function isTurnStateWrite(event: unknown): boolean {
  return parseTurnStateWrite(event) !== null;
}

export async function handleTurnStateWrite(iii: ISdk, event: unknown): Promise<void> {
  const parsed = parseTurnStateWrite(event);
  if (!parsed) return;

  try {
    await emit(iii, parsed.session_id, {
      type: 'turn_state_changed',
      event_type: parsed.event_type,
      new_value: parsed.new_value,
      ...(parsed.old_value !== undefined && { old_value: parsed.old_value }),
    });
  } catch (err) {
    logger.warn('turn::on_turn_state_changed: emit failed', {
      session_id: parsed.session_id,
      err: String(err),
    });
  }
}

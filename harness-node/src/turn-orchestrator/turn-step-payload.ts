/**
 * Shared payload and result types for `turn::{state}` iii functions.
 */

import { z } from 'zod';
import { logger } from '../runtime/otel.js';
import type { TurnState, TurnStateRecord } from './state.js';

export const TurnStepPayloadSchema = z.object({
  session_id: z.string().min(1),
});

export type TurnStepPayload = z.infer<typeof TurnStepPayloadSchema>;

export type TurnStepResult =
  | { ok: true; from_state: TurnState; to_state: TurnState }
  | { ok: true; skipped: true; reason: 'stale' };

/** Returns a stale skip result when the queue message no longer matches persisted state. */
export function staleSkipResult(
  expectedState: TurnState,
  rec: TurnStateRecord,
): TurnStepResult | null {
  if (rec.state === expectedState) return null;
  logger.warn(`turn::${expectedState} skipped: stale queue message`, {
    session_id: rec.session_id,
    expected: expectedState,
    actual: rec.state,
  });
  return { ok: true, skipped: true, reason: 'stale' };
}

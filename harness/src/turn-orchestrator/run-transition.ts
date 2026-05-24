/**
 * Shared FSM transition runner. Every `turn::{state}` function performs the
 * same load → null-check → stale-skip → handle → save sequence; this owns it so
 * each per-state file only contributes its handler.
 *
 * The record loaded here is snapshotted before the handler mutates it and
 * threaded into `saveRecord`, so the save path needs no extra `state::get` to
 * compute the wake decision or the UI event's `old_value` — one read per
 * transition instead of three.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import * as persistence from './persistence.js';
import { type TurnStepPayload, type TurnStepResult } from './schemas.js';
import { type TurnState, type TurnStateRecord, cloneRecord } from './state.js';

export type TransitionHandler = (iii: ISdk, rec: TurnStateRecord) => Promise<void>;

/** Returns a stale skip result when the queue message no longer matches persisted state. */
function staleSkipResult(expectedState: TurnState, rec: TurnStateRecord): TurnStepResult | null {
  if (rec.state === expectedState) return null;
  logger.warn(`turn::${expectedState} skipped: stale queue message`, {
    session_id: rec.session_id,
    expected: expectedState,
    actual: rec.state,
  });
  return { ok: true, skipped: true, reason: 'stale' };
}

export async function runTransition(
  iii: ISdk,
  state: TurnState,
  handle: TransitionHandler,
  payload: TurnStepPayload,
): Promise<TurnStepResult> {
  const rec = await persistence.loadRecord(iii, payload.session_id);
  if (!rec) {
    throw new Error(`turn::${state} invariant: missing session ${payload.session_id}`);
  }
  const skipped = staleSkipResult(state, rec);
  if (skipped) return skipped;

  const previous = cloneRecord(rec);
  const from_state = rec.state;
  try {
    await handle(iii, rec);
  } catch (err) {
    throw new Error(`transition from ${from_state} failed: ${String(err)}`);
  }
  await persistence.saveRecord(iii, rec, previous);
  return { ok: true, from_state, to_state: rec.state };
}

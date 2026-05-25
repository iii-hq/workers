/**
 * Shared FSM transition runner. Every `turn::{state}` function performs the
 * same load → null-check → stale-skip → handle → save sequence; this owns it so
 * each per-state file only contributes its handler.
 *
 * On an unexpected handler throw the session is routed to the `failed`
 * terminal (acked, so the durable queue stops retrying) and the failure is
 * surfaced to the UI. A handler may throw `TransientError` to opt into the
 * queue's retry/backoff/DLQ instead.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { TransientError } from './errors.js';
import { emit } from './events.js';
import { type TurnStepPayload, type TurnStepResult } from './schemas.js';
import { createTurnStore } from './state-runtime/store.js';
import { type TurnState, type TurnStateRecord, transitionTo } from './state.js';
import { syntheticAssistant } from './synthetic-assistant.js';

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

async function failTransition(
  iii: ISdk,
  rec: TurnStateRecord,
  previous: TurnStateRecord,
  from_state: TurnState,
  err: unknown,
): Promise<TurnStepResult> {
  const store = createTurnStore(iii);
  const message = err instanceof Error ? err.message : String(err);
  rec.error = { kind: 'transition_error', message: `from ${from_state}: ${message}` };
  transitionTo(rec, 'failed');
  await store.saveRecord(rec, previous);

  // Surface the failure to the live UI (mirrors the graceful error path):
  // message_complete{stop_reason:'error'} → the translator emits a `stop-reason`
  // event so the user sees WHY; a bare agent_end renders as a silent end.
  // (The UI translator reads stop_reason, not error_kind.)
  const failed = syntheticAssistant({ stop_reason: 'error', text: rec.error.message });
  await emit(iii, rec.session_id, { type: 'message_complete', message: failed, body_streamed: false });

  const messages = await store.loadMessages(rec.session_id);
  await emit(iii, rec.session_id, { type: 'agent_end', messages });
  logger.error('transition failed; session marked failed', {
    session_id: rec.session_id,
    from_state,
    err: message,
  });
  return { ok: true, from_state, to_state: 'failed' };
}

export async function runTransition(
  iii: ISdk,
  state: TurnState,
  handle: TransitionHandler,
  payload: TurnStepPayload,
): Promise<TurnStepResult> {
  const store = createTurnStore(iii);
  const rec = await store.loadRecord(payload.session_id);
  if (!rec) {
    throw new Error(`turn::${state} invariant: missing session ${payload.session_id}`);
  }
  const skipped = staleSkipResult(state, rec);
  if (skipped) return skipped;

  // JSON round-trip matches a persisted reload — snapshot before handler mutates.
  const previous = JSON.parse(JSON.stringify(rec)) as TurnStateRecord;
  const from_state = rec.state;
  try {
    await handle(iii, rec);
  } catch (err) {
    if (err instanceof TransientError) throw err;
    return failTransition(iii, rec, previous, from_state, err);
  }
  await store.saveRecord(rec, previous);
  return { ok: true, from_state, to_state: rec.state };
}

/**
 * `run::abort` — user-initiated interrupt for a session's in-flight turn.
 *
 * The run::start busy guard makes an in-flight session reject new messages, so
 * users need a deliberate way to end a turn: a runaway agent loop, an
 * approval prompt they want to walk away from, or a turn they simply changed
 * their mind about. Abort surfaces a synthetic `aborted` assistant, emits
 * turn_end, clears any parked approvals from the record, and routes to the
 * `finishing` step (agent_end + stopped).
 *
 * Best-effort against an actively-executing step: record writes are
 * last-write-wins, so a step holding the record in memory can overwrite the
 * abort when it saves — its NEXT wake then runs and the loop continues; the
 * abort lands reliably in the gaps between steps (every round-trip has one),
 * so a retry interrupts even a busy loop. Parked states
 * (function_awaiting_approval) have no in-flight step and abort cleanly.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { emit } from './events.js';
import { deleteResolution } from './function-resolve.js';
import { RunAbortPayloadSchema, type RunAbortPayload, type RunAbortResult } from './schemas.js';
import { createTurnStatePorts } from './state-runtime/ports.js';
import { createTurnStore } from './state-runtime/store.js';
import { emitTurnEndOnce, transitionToFinishing } from './state-runtime/turn-end.js';
import { isTurnInFlight, type TurnStateRecord } from './state.js';
import { syntheticAssistant } from './synthetic-assistant.js';

export async function execute(iii: ISdk, payload: RunAbortPayload): Promise<RunAbortResult> {
  const { session_id } = payload;
  const store = createTurnStore(iii);
  const rec = await store.loadRecordStrict(session_id);

  if (!rec || !isTurnInFlight(rec)) {
    return { session_id, aborted: false, state: rec?.state ?? null };
  }
  if (rec.state === 'finishing') {
    return { session_id, aborted: false, state: 'finishing' };
  }

  logger.info('run::abort: ending in-flight turn', { session_id, state: rec.state });

  // Best-effort upstream cancellation: router::abort closes the provider-side
  // reader so the upstream actually stops generating (and billing). The FSM
  // record below stays the source of truth — the router's own terminal frame
  // is discarded by the streaming step's re-entry guard.
  void iii
    .trigger({
      function_id: 'router::abort',
      payload: { request_id: `${session_id}:${rec.started_at_ms}` },
      timeoutMs: 2_000,
    })
    .catch((err) => {
      logger.debug('run::abort: router::abort failed (best-effort)', {
        session_id,
        err: String(err),
      });
    });

  const ports = createTurnStatePorts(iii, store);
  const msg = syntheticAssistant({ stop_reason: 'aborted', text: 'run aborted by user' });
  rec.last_assistant = msg;
  // Best-effort: drop any unconsumed resolution rows for the calls being
  // unparked, so a decision that raced the abort doesn't linger.
  for (const entry of rec.awaiting_approval ?? []) {
    await deleteResolution(iii, session_id, entry.function_call_id).catch(() => {});
  }
  rec.awaiting_approval = [];
  (rec as TurnStateRecord).work = undefined;

  await emit(iii, session_id, { type: 'message_complete', message: msg, body_streamed: false });
  await emitTurnEndOnce(ports, rec, msg);
  transitionToFinishing(rec);
  await store.saveRecord(rec);

  return { session_id, aborted: true, state: 'finishing' };
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'run::abort',
    async (payload: RunAbortPayload) => execute(iii, RunAbortPayloadSchema.parse(payload)),
    {
      description:
        "Abort the session's in-flight turn: emit an aborted assistant + turn_end, clear parked approvals, and route to finishing. Best-effort while a step is mid-execution (retry lands between steps); no-op when no turn is running.",
    },
  );
}

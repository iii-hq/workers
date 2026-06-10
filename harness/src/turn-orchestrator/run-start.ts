/**
 * `run::start`. Persist run config + messages and seed the FSM at `provisioning`.
 *
 * **Incoming**: flat run request from `harness::trigger` (`body.payload` after
 * `HarnessTriggerInputSchema` parse); console/web sends
 * `{ session_id, message_id?, provider, model, mode?, messages }` and omits
 * `system_prompt`, `max_turns` (schema defaults).
 * **Outgoing**: `{ session_id, started }` — persists run config, messages, and
 * seeds `turn_state` to provisioning via `saveRecord`. `started` is false when
 * a turn was already in flight and this call was ignored.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { RunStartPayloadSchema, type RunStartPayload, type RunStartResult } from './schemas.js';
import { createTurnStore } from './state-runtime/store.js';
import { isTurnInFlight, newRecord } from './state.js';

/**
 * Idle window after which an in-flight (non-parked) record is treated as
 * abandoned and a new run::start may take it over. `updated_at_ms` bumps on
 * every state transition, so a live turn refreshes it each round-trip; a
 * single very long provider stream is the one legitimate gap, hence the
 * generous window. `function_awaiting_approval` is exempt — it parks on the
 * user indefinitely by design; `run::abort` is its escape, not staleness.
 */
export const STALE_TURN_TAKEOVER_MS = 30 * 60 * 1000;

export async function execute(iii: ISdk, payload: RunStartPayload): Promise<RunStartResult> {
  const store = createTurnStore(iii);
  const { session_id, messages, max_turns, message_id: _message_id, ...run } = payload;

  const existing = await store.loadRecordStrict(session_id);
  if (existing && isTurnInFlight(existing)) {
    const idleMs = Date.now() - existing.updated_at_ms;
    const takeOver =
      existing.state !== 'function_awaiting_approval' && idleMs > STALE_TURN_TAKEOVER_MS;
    if (!takeOver) {
      logger.warn('run::start ignored: session already has a turn in flight', {
        session_id,
        state: existing.state,
      });
      return { session_id, started: false, reason: 'session_busy' };
    }
    logger.error('run::start taking over a stale in-flight turn', {
      session_id,
      state: existing.state,
      idle_ms: idleMs,
    });
  }

  await store.ensureSession(session_id);

  await store.saveRunRequest(session_id, {
    ...run,
    mode: run.mode ?? null,
    function_schemas: [],
  });
  await store.appendMessages(session_id, messages);

  const record = newRecord(session_id, max_turns);
  await store.saveRecord(record);
  return { session_id, started: true };
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'run::start',
    async (payload: RunStartPayload) => execute(iii, RunStartPayloadSchema.parse(payload)),
    {
      description: 'Start a durable agent session and return immediately.',
    },
  );
}

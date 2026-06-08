/**
 * `run::start`. Persist run config + messages and seed the FSM at `provisioning`.
 *
 * **Incoming**: flat run request from `harness::trigger` (`body.payload` after
 * `HarnessTriggerInputSchema` parse); console/web sends
 * `{ session_id, message_id?, provider, model, mode?, messages }` and omits
 * `system_prompt`, `max_turns` (schema defaults).
 * **Outgoing**: `{ session_id }` — persists run config, messages, and seeds
 * `turn_state` to provisioning via `saveRecord`.
 */

import type { ISdk } from '../runtime/iii.js';
import { RunStartPayloadSchema, type RunStartPayload, type RunStartResult } from './schemas.js';
import { createTurnStore } from './state-runtime/store.js';
import { newRecord } from './state.js';

export async function execute(iii: ISdk, payload: RunStartPayload): Promise<RunStartResult> {
  const store = createTurnStore(iii);
  const { session_id, messages, max_turns, message_id: _message_id, ...run } = payload;

  // Single ensure for the whole run: this is the gateway that begins the turn
  // loop, so every later loadMessages/appendMessages is guaranteed a live record
  // without re-ensuring per call. Must precede the first tree write below.
  await store.ensureSession(session_id);

  await store.saveRunRequest(session_id, {
    ...run,
    mode: run.mode ?? null,
    function_schemas: [],
  });
  await store.appendMessages(session_id, messages);

  const record = newRecord(session_id, max_turns);
  await store.saveRecord(record);
  return { session_id };
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

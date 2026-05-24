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
import * as persistence from './persistence.js';
import { RunStartPayloadSchema, type RunStartPayload, type RunStartResult } from './schemas.js';
import { newRecord } from './state.js';

export async function execute(iii: ISdk, payload: RunStartPayload): Promise<RunStartResult> {
  const { session_id, messages, max_turns, message_id: _message_id, ...run } = payload;

  await persistence.saveRunRequest(iii, session_id, {
    ...run,
    mode: run.mode ?? null,
    function_schemas: [],
  });
  await persistence.saveMessages(iii, session_id, messages);

  const record = newRecord(session_id, max_turns);
  await persistence.saveRecord(iii, record);
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

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

export async function execute(iii: ISdk, payload: RunStartPayload): Promise<RunStartResult> {
  const store = createTurnStore(iii);
  const { session_id, messages, max_turns, message_id: _message_id, ...run } = payload;

  // Refuse to clobber a turn that is still running for this session. A second
  // run::start (second tab, TUI/ACP client, or a double-submit) would otherwise
  // reset the live turn_state record to a fresh `provisioning` and race the
  // in-flight step's last-write-wins saveRecord, corrupting both turns. Read
  // the committed record first; terminal turns (stopped/failed) and fresh
  // sessions start normally. No mutation happens on the busy path.
  const existing = await store.loadRecord(session_id);
  if (existing && isTurnInFlight(existing)) {
    logger.warn('run::start ignored: session already has a turn in flight', {
      session_id,
      state: existing.state,
    });
    return { session_id, started: false, reason: 'session_busy' };
  }

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

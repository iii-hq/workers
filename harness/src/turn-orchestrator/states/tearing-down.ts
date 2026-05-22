/**
 * `turn::tearing_down`. Stop sandbox, emit `agent_end`, transition to `stopped`.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { AgentMessage } from '../../types/agent-message.js';
import { emit } from '../events.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import {
  TurnStepPayloadSchema,
  type TurnStepPayload,
  type TurnStepResult,
  staleSkipResult,
} from '../turn-step-payload.js';

type SandboxStopPayload = { sandbox_id: string; wait: true };

export async function handleTearingDown(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const sandbox_id = await persistence.loadSandboxId(iii, rec.session_id);
  if (sandbox_id) {
    try {
      await iii.trigger<SandboxStopPayload, unknown>({
        function_id: 'sandbox::stop',
        payload: { sandbox_id, wait: true },
        timeoutMs: 60_000,
      });
    } catch (err) {
      logger.warn('sandbox::stop failed during teardown', {
        sandbox_id,
        err: String(err),
      });
    }
  }
  const messages: AgentMessage[] = await persistence.loadMessages(iii, rec.session_id);
  await emit(iii, rec.session_id, { type: 'agent_end', messages });
  transitionTo(rec, 'stopped');
}

export async function execute(iii: ISdk, payload: TurnStepPayload): Promise<TurnStepResult> {
  const rec = await persistence.loadRecord(iii, payload.session_id);
  if (!rec) {
    throw new Error(`turn::tearing_down invariant: missing session ${payload.session_id}`);
  }
  const skipped = staleSkipResult('tearing_down', rec);
  if (skipped) return skipped;

  const from_state = rec.state;
  try {
    await handleTearingDown(iii, rec);
  } catch (err) {
    throw new Error(`transition from ${from_state} failed: ${String(err)}`);
  }
  await persistence.saveRecord(iii, rec);
  return { ok: true, from_state, to_state: rec.state };
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::tearing_down',
    async (payload: unknown) => execute(iii, TurnStepPayloadSchema.parse(payload)),
    {
      description:
        'Run one durable FSM transition for session in state tearing_down: stop sandbox and mark stopped.',
    },
  );
}

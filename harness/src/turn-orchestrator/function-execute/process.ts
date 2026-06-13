/**
 * Run prepared function calls, finalize results, route onward, and register the FSM step.
 */

import type { ISdk } from '../../runtime/iii.js';
import { runTransition } from '../run-transition.js';
import {
  TurnStepPayloadSchema,
  parseFunctionBatchRecord,
  type TurnStepPayload,
} from '../schemas.js';
import { transitionTo, type TurnStateRecord } from '../state.js';
import { finalizeBatch, runBatch } from './run.js';
import { createPorts } from './ports.js';

export async function handleExecute(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const batch = parseFunctionBatchRecord(rec);
  const ports = createPorts(iii);

  const outcome = await runBatch(ports, batch);
  batch.work = outcome.work;

  if (outcome.kind === 'incomplete') {
    const ids = new Set(batch.awaiting_approval.map((entry) => entry.function_call_id));
    const merged = [...batch.awaiting_approval];
    for (const pending of outcome.newPending) {
      if (ids.has(pending.function_call_id)) continue;
      ids.add(pending.function_call_id);
      merged.push(pending);
    }
    batch.awaiting_approval = merged;
    // The transition save enqueues one post-persist scan wake (see
    // shouldWakeStep): it drains any resolution that landed while this
    // batch was still executing — that resolve's own wake stale-skipped
    // against the not-yet-parked record.
    transitionTo(batch, 'function_awaiting_approval');
    return;
  }

  await finalizeBatch(ports, batch);
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::function_execute',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'function_execute', handleExecute, parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state function_execute: dispatch prepared calls and finalize results.',
    },
  );
}

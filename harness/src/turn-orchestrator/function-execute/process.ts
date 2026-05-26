/**
 * Run prepared function calls, finalize results, route onward, and register the FSM step.
 */

import type { ISdk } from '../../runtime/iii.js';
import { runTransition } from '../run-transition.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';
import { transitionTo, type AwaitingApprovalEntry, type TurnStateRecord } from '../state.js';
import type { PendingApproval } from './types.js';
import { isBatchComplete } from './types.js';
import { finalizeBatch, loadOrPlanWork, runBatch } from './run.js';
import { createPorts } from './ports.js';

function mergeAwaitingApproval(
  existing: AwaitingApprovalEntry[] | undefined,
  newPending: PendingApproval[],
): AwaitingApprovalEntry[] {
  const ids = new Set(existing?.map((entry) => entry.function_call_id) ?? []);
  const merged = [...(existing ?? [])];
  for (const pending of newPending) {
    if (ids.has(pending.function_call_id)) continue;
    ids.add(pending.function_call_id);
    merged.push(pending);
  }
  return merged;
}

export async function handleExecute(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const ports = createPorts(iii);
  const work = loadOrPlanWork(rec);

  const outcome = await runBatch(ports, rec, work);
  rec.work = outcome.work;

  if (outcome.kind === 'incomplete') {
    rec.awaiting_approval = mergeAwaitingApproval(rec.awaiting_approval, outcome.newPending);
    transitionTo(rec, 'function_awaiting_approval');
    return;
  }

  if (isBatchComplete(outcome.work)) {
    await finalizeBatch(ports, rec, outcome.work);
  } else {
    transitionTo(rec, 'function_execute');
  }
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

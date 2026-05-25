/**
 * Run prepared function calls, finalize results, route onward, and register the FSM step.
 */

import type { ISdk } from '../../runtime/iii.js';
import { runTransition } from '../run-transition.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';
import { transitionTo, type TurnStateRecord } from '../state.js';
import { finalizeBatch, loadOrPlanWork, runBatch } from './run.js';
import { createPorts } from './ports.js';

export async function handleExecute(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const ports = createPorts(iii);
  const work = loadOrPlanWork(rec);

  const outcome = await runBatch(ports, rec, work);

  if (outcome.kind === 'parked') {
    rec.work = outcome.work;
    rec.awaiting_approval = [...(rec.awaiting_approval ?? []), outcome.pending];
    transitionTo(rec, 'function_awaiting_approval');
    return;
  }

  await finalizeBatch(ports, rec, outcome.work);
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

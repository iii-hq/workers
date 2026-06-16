/**
 * Apply pending `function_resolutions` rows to parked calls and register
 * the FSM step. Wakes arrive directly from `harness::function::resolve`
 * (no state trigger): the resolve handler enqueues
 * `turn::function_awaiting_approval` after persisting the decision.
 */

import type { ISdk } from '../../runtime/iii.js';
import { createPorts } from '../function-execute/ports.js';
import { enqueueAwaitingApprovalWake } from '../function-resolve.js';
import { runTransition } from '../run-transition.js';
import {
  TurnStepPayloadSchema,
  parseFunctionBatchRecord,
  type TurnStepPayload,
} from '../schemas.js';
import type { TurnStateRecord } from '../state.js';
import { createAwaitingApprovalPorts } from './ports.js';
import { processResolvedApprovals, routeAfterApprovalProcessing } from './run.js';

export async function handleAwaitingApproval(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const batch = parseFunctionBatchRecord(rec);
  const executePorts = createPorts(iii);
  const readPorts = createAwaitingApprovalPorts(iii);
  const resolved = await processResolvedApprovals(readPorts, executePorts, batch);
  await routeAfterApprovalProcessing(executePorts, batch);

  // A wake that resolved at least one call but left siblings parked kicks one
  // fresh, uncontended wake. This wake's own re-scan already drains siblings
  // whose decision landed mid-wake; the follow-up covers the remaining gap —
  // a sibling still pending here whose contender wake was dropped (lost the
  // lease race, exhausted retries) would otherwise orphan the batch. A wake
  // that resolves nothing enqueues nothing, so this cannot storm.
  if (resolved > 0 && batch.awaiting_approval.length > 0) {
    await enqueueAwaitingApprovalWake(iii, batch.session_id);
  }
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::function_awaiting_approval',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'function_awaiting_approval', handleAwaitingApproval, parsed, {
        serialize: true,
      });
    },
    {
      description:
        'Run one durable FSM transition for session in state function_awaiting_approval: execute each call as its resolution arrives via harness::function::resolve.',
    },
  );
}

/**
 * Read approval decisions, execute resolved calls individually, and register the FSM step.
 */

import { TriggerAction, type ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { createPorts } from '../function-execute/ports.js';
import { runTransition } from '../run-transition.js';
import {
  ApprovalDecisionEventSchema,
  TurnStepPayloadSchema,
  parseFunctionBatchRecord,
  type TurnStepPayload,
} from '../schemas.js';
import { TURN_STEP_QUEUE } from '../state-runtime/store.js';
import type { TurnStateRecord } from '../state.js';
import { createAwaitingApprovalPorts } from './ports.js';
import { processResolvedApprovals, routeAfterApprovalProcessing } from './run.js';

export async function handleApprovalStateWrite(iii: ISdk, event: unknown): Promise<void> {
  const parsed = ApprovalDecisionEventSchema.safeParse(event);
  if (!parsed.success) return;
  try {
    await iii.trigger({
      function_id: 'turn::function_awaiting_approval',
      payload: { session_id: parsed.data.session_id },
      action: TriggerAction.Enqueue({ queue: TURN_STEP_QUEUE }),
    });
  } catch (err) {
    logger.warn('turn::on_approval: wake failed', {
      session_id: parsed.data.session_id,
      err: String(err),
    });
  }
}

export async function handleAwaitingApproval(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const batch = parseFunctionBatchRecord(rec);
  const executePorts = createPorts(iii);
  const readPorts = createAwaitingApprovalPorts(iii);
  await processResolvedApprovals(readPorts, executePorts, batch);
  await routeAfterApprovalProcessing(executePorts, batch);
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
        'Run one durable FSM transition for session in state function_awaiting_approval: execute each call as its approval decision arrives.',
    },
  );

  iii.registerFunction(
    'turn::on_approval',
    async (event: unknown) => handleApprovalStateWrite(iii, event),
    {
      description:
        'State trigger on scope=approvals; enqueues turn::function_awaiting_approval when a decision is written.',
    },
  );

  iii.registerTrigger({
    type: 'state',
    function_id: 'turn::on_approval',
    config: { scope: 'approvals' },
  });
}

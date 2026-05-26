/**
 * Read approval decisions, execute resolved calls individually, and register the FSM step.
 */

import type { ISdk } from '../../runtime/iii.js';
import { text } from '../../types/content.js';
import type { FunctionResult } from '../../types/function.js';
import { createPorts } from '../function-execute/ports.js';
import { finalizeBatch, FunctionExecuteInvariantError, runOneCall } from '../function-execute/run.js';
import type { PreparedCall } from '../function-execute/types.js';
import { isBatchComplete } from '../function-execute/types.js';
import { runTransition } from '../run-transition.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';
import { transitionTo, type AwaitingApprovalEntry, type TurnStateRecord } from '../state.js';
import {
  createAwaitingApprovalPorts,
  type ApprovalDecision,
  type AwaitingApprovalPorts,
} from './ports.js';

export function denialResultFromDecision(decision: ApprovalDecision): FunctionResult {
  const reason =
    decision.reason ?? (decision.decision === 'aborted' ? 'session_aborted' : 'denied');
  const message =
    decision.decision === 'aborted'
      ? `Function call aborted: ${reason}`
      : `Permission denied by user: ${reason}`;
  return {
    content: [text(message)],
    details: {
      approval_denied: true,
      decision: decision.decision,
      reason,
    },
    terminate: false,
  };
}

export function applyDecisionToPrepared(
  current: PreparedCall,
  decision: ApprovalDecision,
): PreparedCall {
  if (decision.decision === 'allow') {
    return { route: 'pre_approved', call: current.call };
  }
  return {
    route: 'synthetic',
    call: current.call,
    result: denialResultFromDecision(decision),
  };
}

function findPreparedCall(
  prepared: readonly PreparedCall[],
  function_call_id: string,
): PreparedCall | undefined {
  return prepared.find((entry) => entry.call.id === function_call_id);
}

function withoutAwaitingEntry(
  awaiting: AwaitingApprovalEntry[],
  function_call_id: string,
): AwaitingApprovalEntry[] {
  return awaiting.filter((entry) => entry.function_call_id !== function_call_id);
}

export async function processResolvedApprovals(
  readPorts: AwaitingApprovalPorts,
  executePorts: ReturnType<typeof createPorts>,
  rec: TurnStateRecord,
): Promise<void> {
  if (!rec.work) return;

  let awaiting = [...(rec.awaiting_approval ?? [])];
  const executed = { ...rec.work.executed };

  for (const entry of [...awaiting]) {
    const callId = entry.function_call_id;

    if (executed[callId]) {
      awaiting = withoutAwaitingEntry(awaiting, callId);
      continue;
    }

    const decision = await readPorts.readDecision(rec.session_id, callId);
    if (!decision) continue;

    const current = findPreparedCall(rec.work.prepared, callId);
    if (!current) {
      awaiting = withoutAwaitingEntry(awaiting, callId);
      continue;
    }

    const resolved = applyDecisionToPrepared(current, decision);
    await runOneCall(executePorts, rec.session_id, resolved, executed, { skipStart: true });

    awaiting = withoutAwaitingEntry(awaiting, callId);
    rec.work = { prepared: rec.work.prepared, executed };
    await executePorts.checkpoint(rec);
  }

  rec.awaiting_approval = awaiting;
}

export async function routeAfterApprovalProcessing(
  executePorts: ReturnType<typeof createPorts>,
  rec: TurnStateRecord,
): Promise<void> {
  if ((rec.awaiting_approval?.length ?? 0) > 0) {
    return;
  }

  const work = rec.work;
  if (!work) {
    throw new FunctionExecuteInvariantError(
      'function_awaiting_approval with empty awaiting_approval requires work',
    );
  }

  if (isBatchComplete(work)) {
    await finalizeBatch(executePorts, rec, work);
  } else {
    transitionTo(rec, 'function_execute');
  }
}

export async function handleAwaitingApproval(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const executePorts = createPorts(iii);
  const readPorts = createAwaitingApprovalPorts(iii);
  await processResolvedApprovals(readPorts, executePorts, rec);
  await routeAfterApprovalProcessing(executePorts, rec);
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::function_awaiting_approval',
    async (payload: TurnStepPayload) => {
      const parsed = TurnStepPayloadSchema.parse(payload);
      return runTransition(iii, 'function_awaiting_approval', handleAwaitingApproval, parsed);
    },
    {
      description:
        'Run one durable FSM transition for session in state function_awaiting_approval: execute each call as its approval decision arrives.',
    },
  );
}

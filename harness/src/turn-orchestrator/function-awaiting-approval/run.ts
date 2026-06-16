/**
 * Resolve approval decisions and route the batch after each decision.
 */

import { text } from '../../types/content.js';
import type { FunctionResult } from '../../types/function.js';
import { finalizeBatch, runOneCall } from '../function-execute/run.js';
import type { FunctionExecutePorts } from '../function-execute/ports.js';
import type { PreparedCall } from '../function-execute/types.js';
import { isBatchComplete } from '../function-execute/types.js';
import { transitionTo, type FunctionBatchTurnRecord } from '../state.js';
import type { ApprovalDecision, AwaitingApprovalPorts } from './ports.js';

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
    terminate: decision.decision === 'aborted',
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

/**
 * Apply every available approval decision to the parked batch, returning how
 * many calls this wake executed.
 *
 * Re-scans until a full pass resolves nothing new: executing an approved call
 * can write a sibling's decision as a side effect (a parallel approve-all), and
 * that sibling — already read-and-skipped earlier in the same pass — would
 * otherwise stay parked until its own wake fires. When that wake was dropped
 * (it lost the lease race and exhausted the queue's retries), only a re-scan
 * within this wake can drain it.
 */
export async function processResolvedApprovals(
  readPorts: AwaitingApprovalPorts,
  executePorts: FunctionExecutePorts,
  rec: FunctionBatchTurnRecord,
): Promise<number> {
  const work = rec.work;
  let awaiting = [...rec.awaiting_approval];
  const executed = { ...work.executed };
  let resolvedCount = 0;

  let resolvedThisPass = true;
  while (resolvedThisPass) {
    resolvedThisPass = false;

    for (const entry of [...awaiting]) {
      const callId = entry.function_call_id;

      if (executed[callId]) {
        awaiting = awaiting.filter((e) => e.function_call_id !== callId);
        continue;
      }

      const decision = await readPorts.readDecision(rec.session_id, callId);
      if (!decision) continue;

      const current = work.prepared.find((p) => p.call.id === callId);
      if (!current) continue;

      const resolved = applyDecisionToPrepared(current, decision);
      await runOneCall(executePorts, rec.session_id, resolved, executed, { skipStart: true });

      awaiting = awaiting.filter((e) => e.function_call_id !== callId);
      rec.work = { prepared: work.prepared, executed };
      await executePorts.checkpoint(rec);
      resolvedCount += 1;
      resolvedThisPass = true;
    }
  }

  rec.awaiting_approval = awaiting;
  return resolvedCount;
}

export async function routeAfterApprovalProcessing(
  executePorts: FunctionExecutePorts,
  rec: FunctionBatchTurnRecord,
): Promise<void> {
  if (rec.awaiting_approval.length > 0) {
    return;
  }

  if (isBatchComplete(rec.work)) {
    await finalizeBatch(executePorts, rec);
  } else {
    transitionTo(rec, 'function_execute');
  }
}

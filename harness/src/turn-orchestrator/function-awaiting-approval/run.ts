/**
 * Consume `function_resolutions` rows for parked calls and route the
 * batch after each settled call.
 */

import { finalizeBatch, dispatchContext, runOneCall } from '../function-execute/run.js';
import type { FunctionExecutePorts } from '../function-execute/ports.js';
import type { PreparedCall } from '../function-execute/types.js';
import { isBatchComplete } from '../function-execute/types.js';
import type { FunctionResolution } from '../function-resolve.js';
import { transitionTo, type FunctionBatchTurnRecord } from '../state.js';
import type { AwaitingApprovalPorts } from './ports.js';

export function applyDecisionToPrepared(
  current: PreparedCall,
  decision: FunctionResolution,
): PreparedCall {
  if (decision.action === 'execute') {
    return { route: 'pre_approved', call: current.call };
  }
  return {
    route: 'synthetic',
    call: current.call,
    result: {
      content: decision.content,
      details: decision.details ?? {},
      terminate: false,
    },
    is_error: decision.is_error,
  };
}

/**
 * Apply every available resolution to the parked batch, returning how
 * many calls this wake settled.
 *
 * Re-scans until a full pass resolves nothing new: executing a released
 * call can settle a sibling as a side effect, and that sibling — already
 * read-and-skipped earlier in the same pass — would otherwise stay parked
 * until its own wake fires. When that wake was dropped (it lost the lease
 * race and exhausted the queue's retries), only a re-scan within this
 * wake can drain it.
 */
export async function processResolvedApprovals(
  readPorts: AwaitingApprovalPorts,
  executePorts: FunctionExecutePorts,
  rec: FunctionBatchTurnRecord,
): Promise<number> {
  const work = rec.work;
  let awaiting = [...rec.awaiting_approval];
  const executed = { ...work.executed };
  const ctx = dispatchContext(rec);
  let resolvedCount = 0;

  let resolvedThisPass = true;
  while (resolvedThisPass) {
    resolvedThisPass = false;

    for (const entry of [...awaiting]) {
      const callId = entry.function_call_id;

      if (executed[callId]) {
        awaiting = awaiting.filter((e) => e.function_call_id !== callId);
        // Crash-between-execute-and-delete replays land here.
        await readPorts.deleteDecision(rec.session_id, callId);
        continue;
      }

      const decision = await readPorts.readDecision(rec.session_id, callId);
      if (!decision) continue;

      const current = work.prepared.find((p) => p.call.id === callId);
      if (!current) continue;

      const resolved = applyDecisionToPrepared(current, decision);
      await runOneCall(executePorts, ctx, resolved, executed, { skipStart: true });

      awaiting = awaiting.filter((e) => e.function_call_id !== callId);
      rec.work = { prepared: work.prepared, executed };
      await executePorts.checkpoint(rec);
      await readPorts.deleteDecision(rec.session_id, callId);
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

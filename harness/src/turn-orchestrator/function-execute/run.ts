/**
 * Plan, execute, and finalize function call batches.
 */

import type { AssistantMessage, FunctionResultMessage } from '../../types/agent-message.js';
import type { FunctionCallContent } from '../../types/content.js';
import type { FunctionCall, FunctionResult } from '../../types/function.js';
import {
  type DispatchContext,
  TOOL_NAME,
  isErrorResult,
  missingFunctionResult,
  unwrapAgentTrigger,
} from '../agent-trigger.js';
import {
  emitTurnEndOnce,
  endTurnForMaxTurns,
  maxTurnsReached,
  resumeToAssistantStreaming,
  transitionToFinishing,
} from '../state-runtime/turn-end.js';
import {
  type AwaitingApprovalEntry,
  type FunctionBatchTurnRecord,
  type TurnStateRecord,
} from '../state.js';
import type { FunctionExecutePorts } from './ports.js';
import {
  preparedCallId,
  type BatchOutcome,
  type ExecutedCall,
  type FunctionBatchWork,
  type PendingApproval,
  type PreparedCall,
  type ResolveCallResult,
  type RunOneCallResult,
} from './types.js';

function isFunctionCallBlock(
  block: AssistantMessage['content'][number],
): block is FunctionCallContent {
  return block.type === 'function_call';
}

function toPreparedCall(block: FunctionCallContent): PreparedCall {
  const call: FunctionCall = {
    id: block.id,
    function_id: block.function_id,
    arguments: block.arguments,
  };
  if (block.function_id !== TOOL_NAME) {
    return { route: 'synthetic', call, result: missingFunctionResult() };
  }
  const unwrapped = unwrapAgentTrigger(call);
  if (!unwrapped.function_id) {
    return { route: 'synthetic', call: unwrapped, result: missingFunctionResult() };
  }
  return { route: 'dispatch', call: unwrapped };
}

/** Set fields expected when entering `function_execute` (mirrors assistant_streaming finalize). */
export function enterFunctionExecute(rec: TurnStateRecord, asst: AssistantMessage): void {
  const batch = rec as TurnStateRecord & {
    awaiting_approval: AwaitingApprovalEntry[];
    last_assistant: AssistantMessage;
    work: FunctionBatchWork;
  };
  batch.awaiting_approval = [];
  batch.last_assistant = asst;
  batch.work = {
    prepared: asst.content.filter(isFunctionCallBlock).map(toPreparedCall),
    executed: {},
  };
}

/** Hook-chain context for one batch, derived from the FSM record. */
export function dispatchContext(rec: FunctionBatchTurnRecord): DispatchContext {
  return {
    session_id: rec.session_id,
    turn_id: rec.turn_id,
    step: rec.turn_count,
    ...(rec.message_id ? { metadata: { message_id: rec.message_id } } : {}),
  };
}

async function resolvePreparedCall(
  ports: FunctionExecutePorts,
  prepared: PreparedCall,
  ctx: DispatchContext,
): Promise<ResolveCallResult> {
  switch (prepared.route) {
    case 'synthetic':
      return { kind: 'resolved', result: prepared.result, is_error: prepared.is_error ?? true };
    case 'pre_approved': {
      const result = await ports.triggerPreApproved(prepared.call);
      return { kind: 'resolved', result, is_error: isErrorResult(result) };
    }
    case 'dispatch': {
      const out = await ports.dispatch(prepared.call, ctx);
      if (out.kind === 'pending') {
        return { kind: 'pending' };
      }
      return { kind: 'resolved', result: out.result, is_error: isErrorResult(out.result) };
    }
  }
}

export type RunOneCallOptions = {
  /** Skip `function_execution_start` — used when resuming after approval (start already emitted). */
  skipStart?: boolean;
};

export async function runOneCall(
  ports: FunctionExecutePorts,
  ctx: DispatchContext,
  prepared: PreparedCall,
  executed: Record<string, ExecutedCall>,
  opts?: RunOneCallOptions,
): Promise<RunOneCallResult> {
  const call: FunctionCall = prepared.call;
  const { session_id } = ctx;

  const prior = executed[call.id];
  if (prior) {
    await ports.emitEnd(session_id, prior);
    return { kind: 'skipped' };
  }

  if (!opts?.skipStart) {
    await ports.emitStart(session_id, call);
  }
  const startedAt = Date.now();

  const resolved = await resolvePreparedCall(ports, prepared, ctx);
  if (resolved.kind === 'pending') {
    return { kind: 'pending', call };
  }

  const entry: ExecutedCall = {
    call,
    result: resolved.result,
    is_error: resolved.is_error,
    duration_ms: Date.now() - startedAt,
  };
  executed[call.id] = entry;
  await ports.emitEnd(session_id, entry);
  return { kind: 'executed', entry };
}

export async function runBatch(
  ports: FunctionExecutePorts,
  rec: FunctionBatchTurnRecord,
): Promise<BatchOutcome> {
  const { prepared } = rec.work;
  const executed = { ...rec.work.executed };
  const awaitingIds = new Set(rec.awaiting_approval.map((entry) => entry.function_call_id));
  const newPending: PendingApproval[] = [];
  const ctx = dispatchContext(rec);

  for (const item of prepared) {
    const callId = preparedCallId(item);
    if (executed[callId]) continue;
    if (awaitingIds.has(callId)) continue;

    const outcome = await runOneCall(ports, ctx, item, executed);

    if (outcome.kind === 'pending') {
      newPending.push({
        function_call_id: outcome.call.id,
        function_id: outcome.call.function_id,
        args: outcome.call.arguments,
      });
      continue;
    }

    if (outcome.kind === 'executed') {
      rec.work = { prepared, executed };
      await ports.checkpoint(rec);
    }
  }

  const batchWork = { prepared, executed };
  if (newPending.length > 0 || awaitingIds.size > 0) {
    return { kind: 'incomplete', work: batchWork, newPending };
  }
  return { kind: 'completed', work: batchWork };
}

function toFunctionResultMessage(
  entry: ExecutedCall,
  result: FunctionResult,
): FunctionResultMessage {
  return {
    role: 'function_result',
    function_call_id: entry.call.id,
    function_id: entry.call.function_id,
    content: result.content,
    details: result.details,
    is_error: entry.is_error,
    timestamp: Date.now(),
  };
}

/** Collect executed entries in batch order (caller must only invoke when batch is complete). */
function executedInBatchOrder(work: FunctionBatchWork): ExecutedCall[] {
  return work.prepared.map((item) => {
    const entry = work.executed[preparedCallId(item)];
    if (!entry) {
      throw new Error(`missing executed entry for call ${preparedCallId(item)}`);
    }
    return entry;
  });
}

export async function finalizeBatch(
  ports: FunctionExecutePorts,
  rec: FunctionBatchTurnRecord,
): Promise<void> {
  const executed = executedInBatchOrder(rec.work);
  const function_results: FunctionResultMessage[] = [];
  let allTerminate = true;
  const lastAssistant = rec.last_assistant;

  for (const entry of executed) {
    const result = entry.result;
    if (!result.terminate) allTerminate = false;
    function_results.push(toFunctionResultMessage(entry, result));
  }

  // Re-entry safety comes from the deterministic `fr-<function_call_id>`
  // entry ids: session::append is idempotent on entry_id, so a replayed batch
  // re-appends as a no-op (no duplicate rows, no duplicate events) — no
  // transcript reload or trailing-id scan needed.
  if (function_results.length > 0) {
    await ports.appendMessages(rec.session_id, function_results, {
      origin: { turn: rec.turn_count },
    });
  }

  rec.function_results = function_results;

  await emitTurnEndOnce(ports, rec, lastAssistant, function_results);

  if (allTerminate) {
    transitionToFinishing(rec);
  } else if (maxTurnsReached(rec)) {
    await endTurnForMaxTurns(ports, rec);
  } else {
    resumeToAssistantStreaming(rec);
  }

  (rec as TurnStateRecord).work = undefined;
}

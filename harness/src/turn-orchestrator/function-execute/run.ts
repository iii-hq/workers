/**
 * Plan, execute, and finalize function call batches.
 */

import { logger } from '../../runtime/otel.js';
import type { AssistantMessage, FunctionResultMessage } from '../../types/agent-message.js';
import type { FunctionCallContent } from '../../types/content.js';
import type { FunctionCall, FunctionResult } from '../../types/function.js';
import {
  TOOL_NAME,
  isErrorResult,
  missingFunctionResult,
  unwrapAgentTrigger,
} from '../agent-trigger.js';
import { emitTurnEndOnce } from '../state-runtime/turn-end.js';
import { persistedTrailingResultIds } from '../state-runtime/transcript.js';
import { transitionTo, type TurnStateRecord } from '../state.js';
import type { FunctionExecutePorts } from './ports.js';
import {
  emptyBatchWork,
  isBatchComplete,
  preparedCallId,
  type BatchOutcome,
  type ExecutedCall,
  type FunctionBatchWork,
  type PendingApproval,
  type PreparedCall,
  type ResolveCallResult,
  type RunOneCallResult,
} from './types.js';

export { isBatchComplete };

export class FunctionExecuteInvariantError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'FunctionExecuteInvariantError';
  }
}

function isFunctionCallBlock(
  block: AssistantMessage['content'][number],
): block is FunctionCallContent {
  return block.type === 'function_call';
}

function extractFunctionCalls(msg: AssistantMessage): FunctionCall[] {
  return msg.content.filter(isFunctionCallBlock).map((b) => ({
    id: b.id,
    function_id: b.function_id,
    arguments: b.arguments,
  }));
}

function toPreparedCall(raw: FunctionCall): PreparedCall {
  if (raw.function_id !== TOOL_NAME) {
    return { route: 'synthetic', call: raw, result: missingFunctionResult() };
  }
  const call = unwrapAgentTrigger(raw);
  if (!call.function_id) {
    return { route: 'synthetic', call, result: missingFunctionResult() };
  }
  return { route: 'dispatch', call };
}

/** Build prepared calls from the assistant message that requested them. */
export function planBatchFromAssistant(asst: AssistantMessage): PreparedCall[] {
  return extractFunctionCalls(asst).map(toPreparedCall);
}

/** Use existing work or plan a new batch from last_assistant. */
export function loadOrPlanWork(rec: TurnStateRecord): FunctionBatchWork {
  if (rec.work) {
    return rec.work;
  }
  const asst = rec.last_assistant;
  if (!asst) {
    throw new FunctionExecuteInvariantError('function_execute without last_assistant or work');
  }
  return emptyBatchWork(planBatchFromAssistant(asst));
}

async function resolvePreparedCall(
  ports: FunctionExecutePorts,
  prepared: PreparedCall,
  session_id: string,
): Promise<ResolveCallResult> {
  switch (prepared.route) {
    case 'synthetic':
      return { kind: 'resolved', result: prepared.result, is_error: true };
    case 'pre_approved': {
      const result = await ports.triggerPreApproved(prepared.call);
      return { kind: 'resolved', result, is_error: isErrorResult(result) };
    }
    case 'dispatch': {
      const out = await ports.dispatch(prepared.call, session_id);
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
  session_id: string,
  prepared: PreparedCall,
  executed: Record<string, ExecutedCall>,
  opts?: RunOneCallOptions,
): Promise<RunOneCallResult> {
  const call: FunctionCall = prepared.call;

  const prior = executed[call.id];
  if (prior) {
    await ports.emitEnd(session_id, prior);
    return { kind: 'skipped' };
  }

  if (!opts?.skipStart) {
    await ports.emitStart(session_id, call);
  }
  const startedAt = Date.now();

  const resolved = await resolvePreparedCall(ports, prepared, session_id);
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
  rec: TurnStateRecord,
  work: FunctionBatchWork,
): Promise<BatchOutcome> {
  const executed = { ...work.executed };
  const awaitingIds = new Set(
    (rec.awaiting_approval ?? []).map((entry) => entry.function_call_id),
  );
  const newPending: PendingApproval[] = [];

  for (const prepared of work.prepared) {
    const callId = preparedCallId(prepared);
    if (executed[callId]) continue;
    if (awaitingIds.has(callId)) continue;

    const outcome = await runOneCall(ports, rec.session_id, prepared, executed);

    if (outcome.kind === 'pending') {
      newPending.push({
        function_call_id: outcome.call.id,
        function_id: outcome.call.function_id,
        args: outcome.call.arguments,
      });
      continue;
    }

    if (outcome.kind === 'executed') {
      rec.work = { prepared: work.prepared, executed };
      await ports.checkpoint(rec);
    }
  }

  const batchWork = { prepared: work.prepared, executed };
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

/** Collect executed entries in batch order (assistant tool order). */
function executedInBatchOrder(work: FunctionBatchWork): ExecutedCall[] {
  const ordered: ExecutedCall[] = [];
  for (const prepared of work.prepared) {
    const entry = work.executed[preparedCallId(prepared)];
    if (entry) ordered.push(entry);
  }
  return ordered;
}

export async function finalizeBatch(
  ports: FunctionExecutePorts,
  rec: TurnStateRecord,
  work: FunctionBatchWork,
): Promise<void> {
  const executed = executedInBatchOrder(work);
  const function_results: FunctionResultMessage[] = [];
  let allTerminate = executed.length > 0;

  for (const entry of executed) {
    const result = entry.result;
    if (!result.terminate) allTerminate = false;
    function_results.push(toFunctionResultMessage(entry, result));
  }

  const messages = await ports.loadMessages(rec.session_id);
  const alreadyPersisted = persistedTrailingResultIds(messages);
  const fresh = function_results.filter((r) => !alreadyPersisted.has(r.function_call_id));
  if (fresh.length < function_results.length) {
    logger.warn('finalizeBatch: skipped duplicate function_results (re-entry detected)', {
      session_id: rec.session_id,
      total: function_results.length,
      skipped: function_results.length - fresh.length,
    });
  }
  if (fresh.length > 0) {
    await ports.appendMessages(rec.session_id, fresh);
  }

  const asst = rec.last_assistant;
  rec.function_results = function_results;
  rec.work = undefined;

  if (asst) {
    await emitTurnEndOnce(ports, rec, asst, function_results);
  }

  if (allTerminate) {
    await ports.finishSession(rec);
  } else {
    transitionTo(rec, 'steering_check');
  }
}

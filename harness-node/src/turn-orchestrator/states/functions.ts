/**
 * `function_prepare`, `function_execute`, `function_finalize`. Mirrors
 * `turn-orchestrator/src/states/functions.rs`.
 */

import type { ISdk } from '../../runtime/iii.js';
import type { AgentEvent } from '../../types/agent-event.js';
import type {
  AgentMessage,
  AssistantMessage,
  FunctionResultMessage,
} from '../../types/agent-message.js';
import type { FunctionCall, FunctionResult } from '../../types/function.js';
import { TOOL_NAME, dispatchWithHook, isErrorResult } from '../agent-call.js';
import { emit } from '../events.js';
import { publishAfter } from '../hook.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, transitionTo } from '../state.js';

function unwrapAgentCall(fc: FunctionCall): FunctionCall {
  if (fc.function_id !== TOOL_NAME) return fc;
  const args = (fc.arguments ?? {}) as Record<string, unknown>;
  const fn = typeof args.function === 'string' ? args.function : '';
  const payload = args.payload ?? {};
  return { id: fc.id, function_id: fn, arguments: payload };
}

export async function handlePrepare(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  rec.function_results = [];
  const raw = rec.pending_function_calls;
  rec.pending_function_calls = raw.map(unwrapAgentCall);

  const prepared: Array<readonly [FunctionCall, FunctionResult | null]> =
    rec.pending_function_calls.map((fc) => [fc, null] as const);

  await persistence.saveRecord(iii, rec);
  await persistence.saveExecutedCalls(iii, rec.session_id, []);
  await persistence.savePreparedCalls(iii, rec.session_id, prepared);

  transitionTo(rec, 'function_execute');
}

export async function handleExecute(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const runRequest = await persistence.loadRunRequest(iii, rec.session_id);
  const approval_required = Array.isArray(runRequest.approval_required)
    ? (runRequest.approval_required as string[]).filter((x) => typeof x === 'string')
    : [];

  const prepared = await persistence.loadPreparedCalls(iii, rec.session_id);
  const results = await persistence.loadExecutedCalls(iii, rec.session_id);

  for (const [fc] of prepared) {
    await emit(iii, rec.session_id, {
      type: 'function_execution_start',
      function_call_id: fc.id,
      function_id: fc.function_id,
      args: fc.arguments,
    });
    const existing = persistence.findExecutedCall(results, fc.id);
    if (existing) {
      await emit(
        iii,
        rec.session_id,
        buildFunctionExecutionEnd(fc, existing.result, existing.is_error),
      );
      continue;
    }
    // Augment the per-call args with session/fc context — same as Rust.
    let augmented_args: unknown;
    if (fc.arguments && typeof fc.arguments === 'object' && !Array.isArray(fc.arguments)) {
      augmented_args = { ...(fc.arguments as Record<string, unknown>) };
    } else {
      augmented_args = { arguments: fc.arguments };
    }
    if (typeof augmented_args === 'object' && augmented_args !== null) {
      const obj = augmented_args as Record<string, unknown>;
      obj.session_id = rec.session_id;
      obj.function_call_id = fc.id;
      obj.function_id = fc.function_id;
      obj.function_call = {
        id: fc.id,
        function_id: fc.function_id,
        arguments: fc.arguments,
      };
    }
    const augmentedFc: FunctionCall = {
      id: fc.id,
      function_id: fc.function_id,
      arguments: augmented_args,
    };
    const result = await dispatchWithHook(iii, augmentedFc, approval_required);
    const is_error = isErrorResult(result);
    persistence.upsertExecutedCall(results, { function_call: fc, result, is_error });
    // Kick off persistence in parallel with the user-facing emit so the UI's
    // fcall-end lands ~one trigger round-trip sooner. We still await both
    // before the next iteration so ordering and durability are preserved.
    const savePromise = persistence.saveExecutedCalls(iii, rec.session_id, results);
    await emit(iii, rec.session_id, buildFunctionExecutionEnd(fc, result, is_error));
    await savePromise;
  }
  transitionTo(rec, 'function_finalize');
}

function buildFunctionExecutionEnd(
  fc: FunctionCall,
  result: FunctionResult,
  is_error: boolean,
): AgentEvent {
  return {
    type: 'function_execution_end',
    function_call_id: fc.id,
    function_id: fc.function_id,
    result,
    is_error,
  };
}

function buildFinalizeLifecycle(
  asst: AssistantMessage,
  results: FunctionResultMessage[],
): AgentEvent[] {
  const out: AgentEvent[] = [];
  for (const r of results) {
    out.push({ type: 'message_start', message: r });
    out.push({ type: 'message_end', message: r });
  }
  out.push({ type: 'turn_end', message: asst, function_results: results });
  return out;
}

export async function handleFinalize(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const executed = await persistence.loadExecutedCalls(iii, rec.session_id);
  const function_results: FunctionResultMessage[] = [];
  let all_terminate = executed.length > 0;
  for (const e of executed) {
    let result = e.result;
    const merged = await publishAfter(iii, e.function_call, result);
    if (
      merged &&
      typeof merged === 'object' &&
      Array.isArray((merged as Record<string, unknown>).content)
    ) {
      result = merged as FunctionResult;
    }
    if (!result.terminate) all_terminate = false;
    function_results.push({
      role: 'function_result',
      function_call_id: e.function_call.id,
      function_id: e.function_call.function_id,
      content: result.content,
      details: result.details,
      is_error: e.is_error,
      timestamp: Date.now(),
    });
  }
  const messages = await persistence.loadMessages(iii, rec.session_id);
  for (const r of function_results) messages.push(r as AgentMessage);
  await persistence.saveMessages(iii, rec.session_id, messages);

  const asst = rec.last_assistant;
  if (!asst) {
    rec.function_results = function_results;
    rec.pending_function_calls = [];
    transitionTo(rec, 'tearing_down');
    return;
  }
  for (const evt of buildFinalizeLifecycle(asst, function_results)) {
    await emit(iii, rec.session_id, evt);
  }
  rec.turn_end_emitted = true;
  rec.function_results = function_results;
  rec.pending_function_calls = [];
  transitionTo(rec, all_terminate ? 'tearing_down' : 'steering_check');
}

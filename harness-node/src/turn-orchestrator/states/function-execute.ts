/**
 * `turn::function_execute`. Run prepared function calls, finalize results, route onward.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import type { ISdk } from '../../runtime/iii.js';
import type { AgentEvent } from '../../types/agent-event.js';
import type {
  AgentMessage,
  AssistantMessage,
  FunctionResultMessage,
} from '../../types/agent-message.js';
import type { FunctionCall, FunctionResult } from '../../types/function.js';
import { text } from '../../types/content.js';
import { dispatchWithHook, isErrorResult } from '../agent-trigger.js';
import { registerApprovalResume } from '../approval-resume.js';
import { emit } from '../events.js';
import { publishAfter } from '../hook.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, transitionTo } from '../state.js';
import {
  TurnStepPayloadSchema,
  type TurnStepPayload,
  type TurnStepResult,
  staleSkipResult,
} from '../turn-step-payload.js';

function triggerErrorResult(function_id: string, err: unknown): FunctionResult {
  const message =
    err && typeof err === 'object' && typeof (err as Record<string, unknown>).message === 'string'
      ? ((err as Record<string, unknown>).message as string)
      : String(err);
  const details = {
    error: 'trigger_failed',
    function: function_id,
    message,
  };
  return {
    content: [text(JSON.stringify(details))],
    details,
    terminate: false,
  };
}

function decodeOrPassthroughResult(value: unknown): FunctionResult {
  if (
    value &&
    typeof value === 'object' &&
    Array.isArray((value as Record<string, unknown>).content)
  ) {
    const obj = value as Record<string, unknown>;
    return {
      content: obj.content as FunctionResult['content'],
      details: obj.details ?? {},
      terminate: typeof obj.terminate === 'boolean' ? obj.terminate : false,
    };
  }
  const textBody = typeof value === 'string' ? value : JSON.stringify(value);
  return {
    content: [text(textBody)],
    details: value,
    terminate: false,
  };
}

function buildFunctionExecutionEnd(
  fc: FunctionCall,
  result: FunctionResult,
  is_error: boolean,
  duration_ms: number,
): AgentEvent {
  return {
    type: 'function_execution_end',
    function_call_id: fc.id,
    function_id: fc.function_id,
    result,
    is_error,
    duration_ms,
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

async function finalizeExecutedCalls(iii: ISdk, rec: TurnStateRecord): Promise<void> {
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
    transitionTo(rec, all_terminate ? 'tearing_down' : 'steering_check');
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

export async function handleExecute(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const prepared = await persistence.loadPreparedCalls(iii, rec.session_id);
  const results = await persistence.loadExecutedCalls(iii, rec.session_id);

  for (const entry of prepared) {
    const fc = entry.function_call;
    await emit(iii, rec.session_id, {
      type: 'function_execution_start',
      function_call_id: fc.id,
      function_id: fc.function_id,
      args: fc.arguments,
    });
    const startedAt = Date.now();

    const existing = persistence.findExecutedCall(results, fc.id);
    if (existing) {
      await emit(
        iii,
        rec.session_id,
        buildFunctionExecutionEnd(fc, existing.result, existing.is_error, existing.duration_ms),
      );
      continue;
    }

    if (entry.pre_approved === true) {
      let result: FunctionResult;
      let is_error: boolean;
      let duration_ms: number;
      try {
        const value = await iii.trigger<unknown, unknown>({
          function_id: fc.function_id,
          payload: fc.arguments ?? {},
        });
        result = decodeOrPassthroughResult(value);
        is_error = isErrorResult(result);
      } catch (err) {
        result = triggerErrorResult(fc.function_id, err);
        is_error = true;
      }

      duration_ms = Date.now() - startedAt;
      persistence.upsertExecutedCall(results, {
        function_call: fc,
        result,
        is_error,
        duration_ms,
      });

      await persistence.saveExecutedCalls(iii, rec.session_id, results);
      await emit(iii, rec.session_id, buildFunctionExecutionEnd(fc, result, is_error, duration_ms));
      continue;
    }

    if (entry.blocked) {
      const result = entry.blocked;
      const is_error = true;
      const duration_ms = Date.now() - startedAt;
      persistence.upsertExecutedCall(results, {
        function_call: fc,
        result,
        is_error,
        duration_ms,
      });
      await persistence.saveExecutedCalls(iii, rec.session_id, results);
      await emit(iii, rec.session_id, buildFunctionExecutionEnd(fc, result, is_error, duration_ms));
      continue;
    }

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
    const out = await dispatchWithHook(iii, augmentedFc, rec.session_id);

    if (out.kind === 'pending') {
      rec.awaiting_approval = rec.awaiting_approval ?? [];
      rec.awaiting_approval.push({
        function_call_id: fc.id,
        function_id: fc.function_id,
        args: fc.arguments,
      });
      registerApprovalResume(iii, rec.session_id, fc.id);
      transitionTo(rec, 'function_awaiting_approval');
      return;
    }

    const result = out.result;
    const is_error = out.kind === 'deny' || isErrorResult(result);
    const duration_ms = Date.now() - startedAt;

    persistence.upsertExecutedCall(results, {
      function_call: fc,
      result,
      is_error,
      duration_ms,
    });

    const savePromise = persistence.saveExecutedCalls(iii, rec.session_id, results);
    await emit(iii, rec.session_id, buildFunctionExecutionEnd(fc, result, is_error, duration_ms));
    await savePromise;
  }
  await finalizeExecutedCalls(iii, rec);
}

export async function execute(iii: ISdk, payload: TurnStepPayload): Promise<TurnStepResult> {
  const rec = await persistence.loadRecord(iii, payload.session_id);
  if (!rec) {
    throw new Error(`turn::function_execute invariant: missing session ${payload.session_id}`);
  }
  const skipped = staleSkipResult('function_execute', rec);
  if (skipped) return skipped;

  const from_state = rec.state;
  try {
    await handleExecute(iii, rec);
  } catch (err) {
    throw new Error(`transition from ${from_state} failed: ${String(err)}`);
  }
  await persistence.saveRecord(iii, rec);
  return { ok: true, from_state, to_state: rec.state };
}

export function register(iii: ISdk): void {
  iii.registerFunction(
    'turn::function_execute',
    async (payload: unknown) => execute(iii, TurnStepPayloadSchema.parse(payload)),
    {
      description:
        'Run one durable FSM transition for session in state function_execute: dispatch prepared calls and finalize results.',
    },
  );
}

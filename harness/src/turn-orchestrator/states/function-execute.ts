/**
 * `turn::function_execute`. Run prepared function calls, finalize results, route onward.
 *
 * **Incoming**: flat `{ session_id }` via FIFO enqueue on `turn-step`.
 * **Outgoing**: `{ ok, from_state, to_state }` on success; stale skip when state drifted.
 */

import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { AgentEvent } from '../../types/agent-event.js';
import type {
  AgentMessage,
  AssistantMessage,
  FunctionResultMessage,
} from '../../types/agent-message.js';
import type { FunctionCall, FunctionResult } from '../../types/function.js';
import {
  dispatchWithHook,
  isErrorResult,
  missingFunctionResult,
  triggerFunctionCall,
  unwrapAgentTrigger,
} from '../agent-trigger.js';
import { emit } from '../events.js';
import { publishAfter } from '../hook.js';
import * as persistence from '../persistence.js';
import { runTransition } from '../run-transition.js';
import {
  type ExecutedEntry,
  type PreparedEntry,
  type TurnWork,
  type TurnStateRecord,
  transitionTo,
} from '../state.js';
import { TurnStepPayloadSchema, type TurnStepPayload } from '../schemas.js';

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

/**
 * Attach the call's identity + session to its arguments so the target function
 * receives the routing context. Pure: never mutates `fc` or its arguments.
 */
function augmentFunctionCall(fc: FunctionCall, session_id: string): FunctionCall {
  const baseArgs =
    fc.arguments && typeof fc.arguments === 'object' && !Array.isArray(fc.arguments)
      ? (fc.arguments as Record<string, unknown>)
      : { arguments: fc.arguments };
  const augmented = {
    ...baseArgs,
    session_id,
    function_call_id: fc.id,
    function_id: fc.function_id,
    function_call: { id: fc.id, function_id: fc.function_id, arguments: fc.arguments },
  };
  return { id: fc.id, function_id: fc.function_id, arguments: augmented };
}

function extractFunctionCalls(msg: AssistantMessage): FunctionCall[] {
  const out: FunctionCall[] = [];
  for (const b of msg.content) {
    if (b.type === 'function_call') {
      out.push({ id: b.id, function_id: b.function_id, arguments: b.arguments });
    }
  }
  return out;
}

function buildBatch(asst: AssistantMessage): PreparedEntry[] {
  return extractFunctionCalls(asst).map((raw) => {
    const function_call = unwrapAgentTrigger(raw);
    if (!function_call.function_id) {
      return { function_call, blocked: missingFunctionResult() };
    }
    return { function_call, blocked: null };
  });
}

function upsertExecutedCall(executed: ExecutedEntry[], entry: ExecutedEntry): void {
  const idx = executed.findIndex((e) => e.function_call.id === entry.function_call.id);
  if (idx >= 0) executed[idx] = entry;
  else executed.push(entry);
}

function ensureWork(rec: TurnStateRecord): TurnWork {
  if (!rec.work) {
    const asst = rec.last_assistant;
    if (!asst) throw new Error('function_execute without last_assistant');
    rec.work = { batch: buildBatch(asst), results: [] };
  }
  return rec.work;
}

async function commitExecutedCall(
  iii: ISdk,
  rec: TurnStateRecord,
  work: TurnWork,
  fc: FunctionCall,
  result: FunctionResult,
  startedAt: number,
  is_error?: boolean,
): Promise<void> {
  const duration_ms = Date.now() - startedAt;
  const error = is_error ?? isErrorResult(result);
  upsertExecutedCall(work.results, {
    function_call: fc,
    result,
    is_error: error,
    duration_ms,
  });
  await persistence.writeRecord(iii, rec);
  await emit(iii, rec.session_id, buildFunctionExecutionEnd(fc, result, error, duration_ms));
}

/** Run the registered after-hook and adopt its merged result when it returns one. */
async function applyAfterHook(iii: ISdk, entry: ExecutedEntry): Promise<FunctionResult> {
  const merged = await publishAfter(iii, entry.function_call, entry.result);
  if (
    merged &&
    typeof merged === 'object' &&
    Array.isArray((merged as Record<string, unknown>).content)
  ) {
    return merged as FunctionResult;
  }
  return entry.result;
}

function toFunctionResultMessage(
  entry: ExecutedEntry,
  result: FunctionResult,
): FunctionResultMessage {
  return {
    role: 'function_result',
    function_call_id: entry.function_call.id,
    function_id: entry.function_call.function_id,
    content: result.content,
    details: result.details,
    is_error: entry.is_error,
    timestamp: Date.now(),
  };
}

/**
 * Function_call_ids already persisted for the current turn. Results are appended
 * right after the assistant that requested them, so they form the trailing run
 * of `function_result` messages; the first non-result from the tail is the turn
 * boundary.
 */
function persistedResultIds(messages: AgentMessage[]): Set<string> {
  const ids = new Set<string>();
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m?.role === 'function_result') ids.add(m.function_call_id);
    else break;
  }
  return ids;
}

async function finalizeExecutedCalls(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const executed = rec.work?.results ?? [];
  const function_results: FunctionResultMessage[] = [];
  let allTerminate = executed.length > 0;
  for (const entry of executed) {
    const result = await applyAfterHook(iii, entry);
    if (!result.terminate) allTerminate = false;
    function_results.push(toFunctionResultMessage(entry, result));
  }

  // Idempotency: a durable retry / step-fanout race can replay finalize with the
  // same work after the results were appended but before the transition
  // persisted. Re-appending duplicate function_result blocks makes providers
  // reject the turn ("multiple tool_result blocks with id ..."), so drop any id
  // already present in this turn's trailing results.
  const messages = await persistence.loadMessages(iii, rec.session_id);
  const alreadyPersisted = persistedResultIds(messages);
  const fresh = function_results.filter((r) => !alreadyPersisted.has(r.function_call_id));
  if (fresh.length < function_results.length) {
    logger.warn('finalizeExecutedCalls: skipped duplicate function_results (re-entry detected)', {
      session_id: rec.session_id,
      total: function_results.length,
      skipped: function_results.length - fresh.length,
    });
  }
  await persistence.saveMessages(iii, rec.session_id, [...messages, ...fresh]);

  const asst = rec.last_assistant;
  rec.function_results = function_results;
  rec.work = undefined;

  if (asst) {
    await emit(iii, rec.session_id, { type: 'turn_end', message: asst, function_results });
    rec.turn_end_emitted = true;
  }
  transitionTo(rec, allTerminate ? 'tearing_down' : 'steering_check');
}

export async function handleExecute(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const work = ensureWork(rec);

  for (const entry of work.batch) {
    const fc = entry.function_call;

    // Re-entry (approval resume / crash mid-batch): a call already in
    // work.results replays its end event only. Emitting the start first
    // would make the UI show a phantom restart of a completed function.
    const existing = work.results.find((e) => e.function_call.id === fc.id);
    if (existing) {
      await emit(
        iii,
        rec.session_id,
        buildFunctionExecutionEnd(fc, existing.result, existing.is_error, existing.duration_ms),
      );
      continue;
    }

    await emit(iii, rec.session_id, {
      type: 'function_execution_start',
      function_call_id: fc.id,
      function_id: fc.function_id,
      args: fc.arguments,
    });
    const startedAt = Date.now();

    if (entry.pre_approved === true) {
      await commitExecutedCall(iii, rec, work, fc, await triggerFunctionCall(iii, fc), startedAt);
      continue;
    }

    if (entry.blocked) {
      await commitExecutedCall(iii, rec, work, fc, entry.blocked, startedAt, true);
      continue;
    }

    const out = await dispatchWithHook(iii, augmentFunctionCall(fc, rec.session_id));
    if (out.kind === 'pending') {
      rec.awaiting_approval = rec.awaiting_approval ?? [];
      rec.awaiting_approval.push({
        function_call_id: fc.id,
        function_id: fc.function_id,
        args: fc.arguments,
      });
      transitionTo(rec, 'function_awaiting_approval');
      return;
    }

    await commitExecutedCall(iii, rec, work, fc, out.result, startedAt);
  }
  await finalizeExecutedCalls(iii, rec);
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

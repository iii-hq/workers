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
import { dispatchWithHook, isErrorResult, missingFunctionResult, triggerFunctionCall, unwrapAgentTrigger } from '../agent-trigger.js';
import { registerApprovalResume } from '../approval-resume.js';
import { emit } from '../events.js';
import { publishAfter } from '../hook.js';
import * as persistence from '../persistence.js';
import type { ExecutedEntry } from '../persistence.js';
import { runTransition } from '../run-transition.js';
import { type PreparedEntry, type TurnWork, type TurnStateRecord, transitionTo } from '../state.js';
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

function augmentFunctionCall(fc: FunctionCall, session_id: string): FunctionCall {
  let augmented_args: unknown;
  if (fc.arguments && typeof fc.arguments === 'object' && !Array.isArray(fc.arguments)) {
    augmented_args = { ...(fc.arguments as Record<string, unknown>) };
  } else {
    augmented_args = { arguments: fc.arguments };
  }
  if (typeof augmented_args === 'object' && augmented_args !== null) {
    const obj = augmented_args as Record<string, unknown>;
    obj.session_id = session_id;
    obj.function_call_id = fc.id;
    obj.function_id = fc.function_id;
    obj.function_call = {
      id: fc.id,
      function_id: fc.function_id,
      arguments: fc.arguments,
    };
  }
  return { id: fc.id, function_id: fc.function_id, arguments: augmented_args };
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
  persistence.upsertExecutedCall(work.results, {
    function_call: fc,
    result,
    is_error: error,
    duration_ms,
  });
  await persistence.writeRecord(iii, rec);
  await emit(iii, rec.session_id, buildFunctionExecutionEnd(fc, result, error, duration_ms));
}

async function finalizeExecutedCalls(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const work = rec.work ?? { batch: [], results: [] };
  const executed: ExecutedEntry[] = work.results;
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
  // Idempotency guard: handleFinalize can re-enter (durable trigger retry,
  // step-fanout race, crash mid-finalize before transitionTo persists).
  // executedCalls is only cleared at the start of the NEXT handlePrepare,
  // so a second run reads the SAME results and would push duplicates into
  // flat-state. Skip any function_result whose function_call_id is already
  // present. Anthropic rejects duplicate `tool_result` blocks with id:
  //   "each tool_use must have a single result. Found multiple tool_result
  //    blocks with id: toolu_..."
  // and any provider's wire-messages flush would produce them otherwise.
  // Only the most-recent function_result block matters for dedup —
  // duplicates only appear when the re-entry runs against a slice
  // we already wrote in this same finalize, so walking from the tail
  // and stopping once we pass the boundary of pre-existing results
  // is sufficient. Pre-fix this scanned every message from the head
  // on every finalize, which grew O(history) per turn for a guard
  // that only ever protects against ~10 entries.
  const incomingIds = new Set<string>();
  for (const r of function_results) incomingIds.add(r.function_call_id);
  const existingResultIds = new Set<string>();
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (!m) continue;
    if (m.role === 'function_result') {
      existingResultIds.add(m.function_call_id);
      continue;
    }
    if (m.role === 'assistant') {
      // Once we cross an assistant boundary BEFORE seeing any
      // pending incoming id we've passed the turn this finalize
      // is writing for — earlier function_result blocks can't be
      // duplicates of `function_results`.
      let unseen = false;
      for (const id of incomingIds) {
        if (!existingResultIds.has(id)) {
          unseen = true;
          break;
        }
      }
      if (!unseen) break;
    }
  }
  let appended = 0;
  for (const r of function_results) {
    if (existingResultIds.has(r.function_call_id)) continue;
    messages.push(r as AgentMessage);
    existingResultIds.add(r.function_call_id);
    appended++;
  }
  if (appended < function_results.length) {
    logger.warn('handleFinalize: skipped duplicate function_results (re-entry detected)', {
      session_id: rec.session_id,
      total: function_results.length,
      appended,
      skipped: function_results.length - appended,
    });
  }
  await persistence.saveMessages(iii, rec.session_id, messages);

  const asst = rec.last_assistant;
  rec.function_results = function_results;
  rec.pending_function_calls = [];
  rec.work = undefined;

  if (asst) {
    await emit(iii, rec.session_id, { type: 'turn_end', message: asst, function_results });
    rec.turn_end_emitted = true;
  }
  transitionTo(rec, all_terminate ? 'tearing_down' : 'steering_check');
}

export async function handleExecute(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const work = ensureWork(rec);

  for (const entry of work.batch) {
    const fc = entry.function_call;
    await emit(iii, rec.session_id, {
      type: 'function_execution_start',
      function_call_id: fc.id,
      function_id: fc.function_id,
      args: fc.arguments,
    });
    const startedAt = Date.now();

    const existing = persistence.findExecutedCall(work.results, fc.id);
    if (existing) {
      await emit(
        iii,
        rec.session_id,
        buildFunctionExecutionEnd(fc, existing.result, existing.is_error, existing.duration_ms),
      );
      continue;
    }

    if (entry.pre_approved === true) {
      await commitExecutedCall(
        iii,
        rec,
        work,
        fc,
        await triggerFunctionCall(iii, fc),
        startedAt,
      );
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
      registerApprovalResume(iii, rec.session_id, fc.id);
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

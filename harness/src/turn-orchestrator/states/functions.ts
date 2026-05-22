/**
 * `function_prepare`, `function_execute`, `function_finalize`. Mirrors
 * `turn-orchestrator/src/states/functions.rs`.
 */

import { STATE_SCOPE } from '../../approval-gate/schemas.js';
import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import type { AgentEvent } from '../../types/agent-event.js';
import type {
  AgentMessage,
  AssistantMessage,
  FunctionResultMessage,
} from '../../types/agent-message.js';
import { text } from '../../types/content.js';
import type { FunctionCall, FunctionResult } from '../../types/function.js';
import { TOOL_NAME, dispatchWithHook, isErrorResult } from '../agent-trigger.js';
import { registerApprovalResume } from '../approval-resume.js';
import type { TurnOrchestratorConfig } from '../config.js';
import { emit } from '../events.js';
import { publishAfter } from '../hook.js';
import type { PreparedEntry } from '../persistence.js';
import * as persistence from '../persistence.js';
import { type TurnStateRecord, transitionTo } from '../state.js';

type ApprovalDecisionRecord = {
  decision: 'allow' | 'deny' | 'aborted';
  reason: string | null;
};

function unwrapAgentTrigger(fc: FunctionCall): FunctionCall {
  if (fc.function_id !== TOOL_NAME) return fc;
  const args = (fc.arguments ?? {}) as Record<string, unknown>;
  const fn = typeof args.function === 'string' ? args.function : '';
  const payload = args.payload ?? {};
  return { id: fc.id, function_id: fn, arguments: payload };
}

export async function handlePrepare(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  rec.function_results = [];
  const raw = rec.pending_function_calls;
  rec.pending_function_calls = raw.map(unwrapAgentTrigger);

  const prepared: PreparedEntry[] = rec.pending_function_calls.map((fc) => ({
    function_call: fc,
    blocked: null,
  }));

  await persistence.saveRecord(iii, rec);
  await persistence.saveExecutedCalls(iii, rec.session_id, []);
  await persistence.savePreparedCalls(iii, rec.session_id, prepared);

  transitionTo(rec, 'function_execute');
}

export async function handleExecute(
  iii: ISdk,
  cfg: TurnOrchestratorConfig,
  rec: TurnStateRecord,
): Promise<void> {
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
    /* `startedAt` is captured right after the start emit so the measured
       window matches what the consumer sees on the wire. Each non-replay
       branch computes its own delta; `existing` reuses the persisted one. */
    const startedAt = Date.now();

    const existing = persistence.findExecutedCall(results, fc.id);
    if (existing) {
      /* Replay: reuse the persisted duration so a resumed run shows the
         original timing, not the ~0ms it takes to re-emit. */
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
      /* Denial is effectively instant — local delta captures whatever
         time the persist + emit roundtrip takes, which is honest. */
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
    const out = await dispatchWithHook(iii, augmentedFc, rec.session_id);

    if (out.kind === 'pending') {
      /* No end emit; `startedAt` is discarded. On resume, the loop re-enters
         and a fresh `function_execution_start` resets the timer — approval
         wait time is naturally excluded from the eventual duration. */
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

    // Kick off persistence in parallel with the user-facing emit so the UI's
    // fcall-end lands ~one trigger round-trip sooner. We still await both
    // before the next iteration so ordering and durability are preserved.
    const savePromise = persistence.saveExecutedCalls(iii, rec.session_id, results);
    await emit(iii, rec.session_id, buildFunctionExecutionEnd(fc, result, is_error, duration_ms));
    await savePromise;
  }
  transitionTo(rec, 'function_finalize');
}

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

async function readDecision(
  iii: ISdk,
  session_id: string,
  function_call_id: string,
): Promise<ApprovalDecisionRecord | null> {
  const key = `${session_id}/${function_call_id}`;
  const raw = await iii.trigger<unknown, unknown>({
    function_id: 'state::get',
    payload: { scope: STATE_SCOPE, key },
  });
  if (!raw || typeof raw !== 'object') return null;
  const obj = raw as Record<string, unknown>;
  const decision = obj.decision;
  if (decision !== 'allow' && decision !== 'deny' && decision !== 'aborted') return null;
  return {
    decision,
    reason: typeof obj.reason === 'string' ? obj.reason : null,
  };
}

function denialResultFromDecision(decision: ApprovalDecisionRecord): FunctionResult {
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

export async function handleAwaitingApproval(iii: ISdk, rec: TurnStateRecord): Promise<void> {
  const awaiting = rec.awaiting_approval ?? [];
  if (awaiting.length === 0) {
    transitionTo(rec, 'function_execute');
    return;
  }

  const decisions = await Promise.all(
    awaiting.map((entry) => readDecision(iii, rec.session_id, entry.function_call_id)),
  );

  if (decisions.some((decision) => decision === null)) {
    return;
  }

  const prepared = await persistence.loadPreparedCalls(iii, rec.session_id);
  for (let i = 0; i < awaiting.length; i++) {
    const entry = awaiting[i];
    const decision = decisions[i];
    if (!entry || !decision) continue;
    const idx = prepared.findIndex(
      (preparedEntry) => preparedEntry.function_call.id === entry.function_call_id,
    );
    if (idx < 0) continue;
    const current = prepared[idx];
    if (!current) continue;
    if (decision.decision === 'allow') {
      prepared[idx] = { ...current, pre_approved: true, blocked: null };
    } else {
      prepared[idx] = {
        ...current,
        pre_approved: false,
        blocked: denialResultFromDecision(decision),
      };
    }
  }

  await persistence.savePreparedCalls(iii, rec.session_id, prepared);

  rec.awaiting_approval = [];
  transitionTo(rec, 'function_execute');
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
  if (!asst) {
    rec.function_results = function_results;
    rec.pending_function_calls = [];
    await persistence.saveExecutedCalls(iii, rec.session_id, []);
    transitionTo(rec, all_terminate ? 'tearing_down' : 'steering_check');
    return;
  }
  for (const evt of buildFinalizeLifecycle(asst, function_results)) {
    await emit(iii, rec.session_id, evt);
  }
  rec.turn_end_emitted = true;
  rec.function_results = function_results;
  rec.pending_function_calls = [];
  // Clear persisted executedCalls now so a re-entry into handleFinalize
  // (durable retry, crash before transitionTo) finds an empty set and
  // produces zero new function_results to push. Belt+suspenders alongside
  // the idempotency guard above. handlePrepare also clears at the start
  // of the NEXT turn, but that's too late if re-entry happens before then.
  await persistence.saveExecutedCalls(iii, rec.session_id, []);
  transitionTo(rec, all_terminate ? 'tearing_down' : 'steering_check');
}

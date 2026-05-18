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
import { text } from '../../types/content.js';
import type { FunctionCall, FunctionResult } from '../../types/function.js';
import { TOOL_NAME, dispatchWithHook, isErrorResult } from '../agent-call.js';
import { emit } from '../events.js';
import { publishAfter } from '../hook.js';
import type { PreparedEntry } from '../persistence.js';
import * as persistence from '../persistence.js';
import { type TurnState, type TurnStateRecord, transitionTo } from '../state.js';

const APPROVAL_CONSUME_FN = 'approval::consume';
const APPROVAL_CONSUME_TIMEOUT_MS = 5_000;
const APPROVAL_STATE_SCOPE = 'approvals';

type ApprovalDecisionRecord = {
  decision: 'allow' | 'deny' | 'aborted';
  reason: string | null;
};

/**
 * Pure helper. Converts a merged blocking reply from the before-hook into a
 * `FunctionResult`. Mirrors
 * `turn-orchestrator/src/states/functions.rs::prefilled_result_for_block` (PR #150).
 *
 * - `status === 'pending'` → terminating pending placeholder. Marks the
 *   FunctionResult with `details.pending_approval = true` so handleFinalize
 *   can later replace it with the real resolved result.
 * - any other block → non-terminating hard-block. The LLM sees the reason
 *   and the turn continues with the next call.
 */
export function prefilledResultForBlock(
  merged: Record<string, unknown>,
  call_id: string,
  function_id: string,
): FunctionResult {
  if (merged.status === 'pending') {
    const body = {
      status: 'pending_approval',
      call_id,
      function_id,
      message: 'Awaiting human approval. The result will be reported in a future turn.',
    };
    return {
      content: [text(JSON.stringify(body, null, 2))],
      details: { pending_approval: true, call_id },
      terminate: true,
    };
  }
  const reason = typeof merged.reason === 'string' ? merged.reason : 'blocked';
  return {
    content: [text(reason)],
    details: { blocked: true },
    terminate: false,
  };
}

/**
 * `false` for pending-approval placeholders (so handleExecute does NOT mark
 * them as `is_error: true`), `true` for hard-block prefills. Mirrors
 * `prefilled_result_is_error`.
 */
export function prefilledResultIsError(result: FunctionResult): boolean {
  if (!result.details || typeof result.details !== 'object') return true;
  return !(result.details as Record<string, unknown>).pending_approval;
}

/**
 * Builds the deny envelope the orchestrator uses when the hook bus itself
 * fails (publish error, no approval-gate reply, etc.). Mirrors
 * `fail_closed_block_reply`.
 */
export function failClosedBlockReply(phase: string, error: string): Record<string, unknown> {
  return {
    block: true,
    status: 'denied',
    denial: {
      kind: 'state_error',
      detail: { phase, error },
    },
    reason: `hook bus unavailable during ${phase}: ${error}`,
  };
}

function isApprovalGateReply(reply: unknown): boolean {
  if (!reply || typeof reply !== 'object') return false;
  const r = reply as Record<string, unknown>;
  return r.approval_gate === true || r.subscriber === 'approval-gate';
}

/**
 * Returns an error string when the publish-collect response indicates a
 * fail-closed condition (publish failed, or — when
 * `requireApprovalGateReply` — no reply came from approval-gate).
 * Returns `undefined` when the response is healthy. Mirrors
 * `publish_failure_from_response`.
 */
export function publishFailureFromResponse(
  response: Record<string, unknown>,
  requireApprovalGateReply: boolean,
): string | undefined {
  const publish = response.publish as Record<string, unknown> | undefined;
  if (publish && publish.ok === false) {
    return typeof publish.error === 'string' ? publish.error : 'publish failed';
  }
  if (response.publish_failed === true) {
    return 'publish failed';
  }
  if (requireApprovalGateReply) {
    const replies = Array.isArray(response.replies) ? response.replies : [];
    const gate_replied = replies.some(isApprovalGateReply);
    if (!gate_replied) {
      return 'publish succeeded but approval-gate did not reply';
    }
  }
  return undefined;
}

/**
 * Maps `approval::consume` entries into the same `(FunctionCall, prefilled?)`
 * pairs that `handlePrepare` produces. Mirrors
 * `prepared_calls_from_approval_entries`.
 *
 * - `decision: 'allow'` → dispatch via the normal path (prefilled is null).
 * - `decision: 'deny'` → emit a denial FunctionResult; the LLM sees
 *   `approval denied: <reason>` or `approval timed out` and continues.
 */
export function preparedCallsFromApprovalEntries(entries: unknown[]): PreparedEntry[] {
  const out: PreparedEntry[] = [];
  for (const entry of entries) {
    if (!entry || typeof entry !== 'object') continue;
    const e = entry as Record<string, unknown>;
    const function_call_id =
      (typeof e.function_call_id === 'string' && e.function_call_id) ||
      (typeof e.tool_call_id === 'string' && e.tool_call_id) ||
      null;
    if (!function_call_id) continue;
    const function_id = typeof e.function_id === 'string' ? e.function_id : '';
    const args = e.args ?? {};
    const decision = typeof e.decision === 'string' ? e.decision : 'deny';
    const fc: FunctionCall = { id: function_call_id, function_id, arguments: args };

    if (decision === 'allow') {
      out.push({ function_call: fc, blocked: null });
      continue;
    }

    const reason = typeof e.reason === 'string' ? e.reason : 'denied';
    const message =
      reason === 'timed_out' || decision === 'timed_out'
        ? 'approval timed out before resolution'
        : `approval denied: ${reason}`;
    const result: FunctionResult = {
      content: [text(message)],
      details: {
        approval_denied: true,
        decision,
        reason,
        resolved_via_approval_gate: true,
        call_id: function_call_id,
      },
      terminate: false,
    };
    out.push({ function_call: fc, blocked: result });
  }
  return out;
}

/**
 * Triggers `approval::consume` for the given session and maps the entries
 * into prepared FunctionCall pairs. Mirrors
 * `consume_resolved_approval_entries`.
 */
export async function consumeResolvedApprovalEntries(
  iii: ISdk,
  session_id: string,
): Promise<PreparedEntry[]> {
  const response = (await iii.trigger<unknown, unknown>({
    function_id: APPROVAL_CONSUME_FN,
    payload: { session_id },
    timeoutMs: APPROVAL_CONSUME_TIMEOUT_MS,
  })) as Record<string, unknown> | undefined;
  if (response && response.ok === false) {
    const err = typeof response.error === 'string' ? response.error : 'approval::consume failed';
    throw new Error(err);
  }
  const entries = response && Array.isArray(response.entries) ? response.entries : [];
  return preparedCallsFromApprovalEntries(entries);
}

/**
 * Strips prior pending-approval FunctionResult messages whose call_id
 * matches one of the incoming resolved replacements. Prevents duplicate
 * function-result messages in the transcript when a session resumes.
 * Mirrors `replace_pending_approval_placeholders`.
 */
export function replacePendingApprovalPlaceholders(
  messages: AgentMessage[],
  replacements: FunctionResultMessage[],
): void {
  if (replacements.length === 0) return;
  const ids = new Set(replacements.map((r) => r.function_call_id));
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (!m || (m as { role?: string }).role !== 'function_result') continue;
    const fr = m as FunctionResultMessage;
    if (!ids.has(fr.function_call_id)) continue;
    const details =
      fr.details && typeof fr.details === 'object' ? (fr.details as Record<string, unknown>) : null;
    if (details?.pending_approval === true) {
      messages.splice(i, 1);
    }
  }
}

/**
 * Extracted final-transition helper. Mirrors `next_state_after_finalize`.
 */
export function nextStateAfterFinalize(
  _hasLastAssistant: boolean,
  allTerminate: boolean,
): TurnState {
  return allTerminate ? 'tearing_down' : 'steering_check';
}

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

  const prepared: PreparedEntry[] = rec.pending_function_calls.map((fc) => ({
    function_call: fc,
    blocked: null,
  }));

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

  for (const entry of prepared) {
    const fc = entry.function_call;
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
    const result = await dispatchWithHook(iii, augmentedFc, approval_required, rec.session_id);
    const is_error = isErrorResult(result);
    persistence.upsertExecutedCall(results, { function_call: fc, result, is_error });
    await persistence.saveExecutedCalls(iii, rec.session_id, results);
    await emit(iii, rec.session_id, buildFunctionExecutionEnd(fc, result, is_error));
  }
  transitionTo(rec, 'function_finalize');
}

async function readDecision(
  iii: ISdk,
  session_id: string,
  function_call_id: string,
): Promise<ApprovalDecisionRecord | null> {
  const key = `${session_id}/${function_call_id}`;
  const raw = await iii.trigger<unknown, unknown>({
    function_id: 'state::get',
    payload: { scope: APPROVAL_STATE_SCOPE, key },
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
  const reason = decision.reason ?? (decision.decision === 'aborted' ? 'session_aborted' : 'denied');
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
  // PR #150: strip any prior pending-approval placeholder for the same
  // call_id before appending the resolved result, so the transcript
  // doesn't carry duplicate function-result messages on resume.
  replacePendingApprovalPlaceholders(messages, function_results);
  for (const r of function_results) messages.push(r as AgentMessage);
  await persistence.saveMessages(iii, rec.session_id, messages);

  const asst = rec.last_assistant;
  if (!asst) {
    rec.function_results = function_results;
    rec.pending_function_calls = [];
    transitionTo(rec, nextStateAfterFinalize(false, all_terminate));
    return;
  }
  for (const evt of buildFinalizeLifecycle(asst, function_results)) {
    await emit(iii, rec.session_id, evt);
  }
  rec.turn_end_emitted = true;
  rec.function_results = function_results;
  rec.pending_function_calls = [];
  transitionTo(rec, nextStateAfterFinalize(true, all_terminate));
}

/**
 * Approval-gate intercept helper. Mirrors `approval-gate/src/lib.rs::handle_intercept`.
 *
 * Called from the gate's `agent::before_function_call` subscriber (and from
 * tests directly). The semantics are fail-closed: if the state bus write
 * fails we return a denial envelope rather than letting the call through.
 */

import { logger } from '../runtime/otel.js';
import type { StateBus } from './state-bus.js';
import {
  type IncomingCall,
  SUBSCRIBER_NAME,
  blockReplyFor,
  buildPendingRecord,
  pendingKey,
} from './types.js';

export type InterceptReply =
  | ReturnType<typeof blockReplyFor>
  | {
      block: true;
      status: 'pending';
      reason: string;
      function_call_id: string;
      tool_call_id: string;
      function_id: string;
      subscriber: typeof SUBSCRIBER_NAME;
      approval_gate: true;
    }
  | {
      block: true;
      status: 'denied';
      reason: string;
      subscriber: typeof SUBSCRIBER_NAME;
      approval_gate: true;
      denial: {
        kind: 'state_error';
        detail: { phase: 'pending_write'; error: string };
      };
    };

function callRequiresApproval(call: IncomingCall): boolean {
  return call.approval_required.includes(call.function_id);
}

export async function handleIntercept(
  bus: StateBus,
  state_scope: string,
  call: IncomingCall,
  now_ms: number,
  timeout_ms: number,
): Promise<InterceptReply> {
  if (!callRequiresApproval(call)) {
    return blockReplyFor({ kind: 'allow' });
  }

  // PR #150 resume: when the orchestrator resurrects a session after the
  // human approved, it re-dispatches the SAME function_call_id. Without
  // this short-circuit the gate writes a fresh pending entry and the user
  // gets prompted forever in a loop. If an entry already exists for the
  // same key and was consumed with `decision: 'allow'`, treat as allow.
  const existing = (await bus.get(
    state_scope,
    pendingKey(call.session_id, call.function_call_id),
  )) as Record<string, unknown> | null;
  if (existing && existing.status === 'consumed' && existing.decision === 'allow') {
    return blockReplyFor({ kind: 'allow' });
  }
  if (existing && existing.status === 'consumed' && existing.decision === 'deny') {
    const reason = typeof existing.reason === 'string' ? existing.reason : 'denied';
    return blockReplyFor({ kind: 'deny', reason });
  }

  const record = buildPendingRecord(
    call.session_id,
    call.function_call_id,
    call.function_id,
    call.args,
    now_ms,
    timeout_ms,
  );

  try {
    await bus.set(state_scope, pendingKey(call.session_id, call.function_call_id), record);
  } catch (err) {
    const error = err instanceof Error ? err.message : String(err);
    logger.error('approval-gate: failed to write pending record', {
      session_id: call.session_id,
      function_call_id: call.function_call_id,
      error,
    });
    return {
      block: true,
      status: 'denied',
      reason: 'approval-gate: state_write_failed',
      subscriber: SUBSCRIBER_NAME,
      approval_gate: true,
      denial: {
        kind: 'state_error',
        detail: { phase: 'pending_write', error },
      },
    };
  }

  return {
    block: true,
    status: 'pending',
    reason: 'approval required',
    function_call_id: call.function_call_id,
    tool_call_id: call.function_call_id,
    function_id: call.function_id,
    subscriber: SUBSCRIBER_NAME,
    approval_gate: true,
  };
}

/**
 * Pending-record utilities, resolve handler, and the resume-session polling
 * loop. Mirrors `approval-gate/src/lib.rs::{handle_resolve, handle_list_pending,
 * resume_session}` (post PR #150).
 */

import { requireString } from '../runtime/handler.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { emitApprovalResolved, emitApprovalWakeFailed } from './events.js';
import type { StateBus } from './state-bus.js';
import { pendingKey } from './types.js';

export async function handleResolve(
  bus: StateBus,
  state_scope: string,
  payload: unknown,
): Promise<unknown> {
  if (!payload || typeof payload !== 'object') {
    return { ok: false, error: 'missing_id' };
  }
  const obj = payload as Record<string, unknown>;
  const session_id = typeof obj.session_id === 'string' ? obj.session_id : '';
  const function_call_id =
    (typeof obj.function_call_id === 'string' && obj.function_call_id) ||
    (typeof obj.tool_call_id === 'string' && obj.tool_call_id) ||
    '';
  if (!session_id || !function_call_id) return { ok: false, error: 'missing_id' };
  const decision = obj.decision;
  if (decision !== 'allow' && decision !== 'deny') {
    return { ok: false, error: 'bad_decision' };
  }
  const key = pendingKey(session_id, function_call_id);
  const existing = await bus.get(state_scope, key);
  if (!existing || typeof existing !== 'object') {
    return { ok: false, error: 'not_found' };
  }
  const e = { ...(existing as Record<string, unknown>) };
  if (e.status !== 'pending') return { ok: false, error: 'already_resolved' };

  const now = Date.now();
  const expires_at = typeof e.expires_at === 'number' ? e.expires_at : Number.POSITIVE_INFINITY;
  if (now >= expires_at) {
    e.status = 'resolved';
    e.decision = 'deny';
    e.reason = 'timed_out';
    e.resolved_at = now;
    try {
      await bus.set(state_scope, key, e);
    } catch (err) {
      logger.error('approval-gate: failed to persist timed_out state', {
        session_id,
        function_call_id,
        err: String(err),
      });
    }
    return { ok: false, error: 'timed_out' };
  }

  e.status = 'resolved';
  e.decision = decision;
  if (typeof obj.reason === 'string') e.reason = obj.reason;
  e.resolved_at = now;
  try {
    await bus.set(state_scope, key, e);
  } catch (err) {
    logger.error('approval-gate: failed to write resolved state', { err: String(err) });
    return { ok: false, error: 'state_write_failed' };
  }
  return { ok: true };
}

export async function handleListPending(
  bus: StateBus,
  state_scope: string,
  payload: unknown,
): Promise<unknown> {
  const obj = (payload ?? {}) as Record<string, unknown>;
  const session_id = typeof obj.session_id === 'string' ? obj.session_id : '';
  if (!session_id) return { pending: [] };
  const all = await bus.listPrefix(state_scope, `${session_id}/`);
  const pending = all.filter(
    (v) => v && typeof v === 'object' && (v as Record<string, unknown>).status === 'pending',
  );
  return { pending };
}

const STEP_TOPIC = 'turn::step_requested';

/**
 * Wakes the orchestrator after an approval was resolved. iii-native:
 * fire-and-forget `iii::durable::publish` on `turn::step_requested`. The
 * orchestrator's existing durable subscriber on that topic wakes, detects
 * the terminal record itself (via `buildResumePlan`), rebuilds it, and
 * walks the FSM into `handle_awaiting` which consumes the resolved entry.
 *
 * Replaces the prior 250 ms polling loop against `run::resume`. iii has
 * no `wait_until(state)` primitive but it does have durable pub/sub, so
 * we use that: one publish, no client-side waiting, no RPC ping-pong.
 *
 * Throws only if the publish trigger itself fails. The caller (the
 * resolve wrapper) catches the rejection and emits
 * `approval_wake_failed`.
 */
export async function resumeSession(iii: ISdk, session_id: string): Promise<void> {
  // No explicit timeout — `iii::durable::publish` enqueues into the bus
  // without waiting for subscribers, so it returns in single-digit ms.
  // iii's default trigger timeout is the right backstop for an unhealthy
  // bus; we don't try to tune it here.
  await iii.trigger<unknown, unknown>({
    function_id: 'iii::durable::publish',
    payload: {
      topic: STEP_TOPIC,
      data: { session_id },
    },
  });
}

/**
 * Closure-style wrapper used during `register`. Calls `handleResolve`; on
 * success, emits `approval_resolved` and triggers `resumeSession`. On wake
 * failure, emits `approval_wake_failed`. Mirrors the closure in
 * `approval-gate/src/lib.rs::register` around `FN_RESOLVE`.
 */
export async function handleResolveWithEvents(
  iii: ISdk,
  bus: StateBus,
  state_scope: string,
  payload: unknown,
): Promise<unknown> {
  const out = await handleResolve(bus, state_scope, payload);
  const result = out as Record<string, unknown>;
  if (result.ok !== true) return out;

  const obj = (payload ?? {}) as Record<string, unknown>;
  const session_id = typeof obj.session_id === 'string' ? obj.session_id : '';
  const function_call_id =
    (typeof obj.function_call_id === 'string' && obj.function_call_id) ||
    (typeof obj.tool_call_id === 'string' && obj.tool_call_id) ||
    '';
  if (!session_id || !function_call_id) return out;

  const stored = (await bus.get(state_scope, pendingKey(session_id, function_call_id))) as Record<
    string,
    unknown
  > | null;
  const decision =
    stored?.decision === 'allow' || stored?.decision === 'deny'
      ? (stored.decision as 'allow' | 'deny')
      : 'deny';
  const reason = typeof stored?.reason === 'string' ? (stored.reason as string) : null;

  await emitApprovalResolved(iii, session_id, {
    function_call_id,
    decision,
    reason,
  });

  try {
    await resumeSession(iii, session_id);
  } catch (err) {
    const error = err instanceof Error ? err.message : String(err);
    await emitApprovalWakeFailed(iii, session_id, error);
  }

  return out;
}

// Re-export for ergonomic imports
export { requireString };

/**
 * Pending-record utilities + the await-decision poll loop. Mirrors
 * `approval-gate/src/lib.rs::{handle_resolve, handle_list_pending,
 * await_decision}`.
 */

import { requireString } from '../runtime/handler.js';
import { logger } from '../runtime/otel.js';
import type { StateBus } from './state-bus.js';
import { pendingKey } from './types.js';

const POLL_INTERVAL_MS = 250;

export type AwaitedDecision = { kind: 'allow' } | { kind: 'deny'; reason: string };

export async function awaitDecision(
  bus: StateBus,
  state_scope: string,
  session_id: string,
  function_call_id: string,
  expires_at: number,
): Promise<AwaitedDecision> {
  const key = pendingKey(session_id, function_call_id);
  for (;;) {
    const rec = await bus.get(state_scope, key);
    if (!rec || typeof rec !== 'object') {
      return { kind: 'deny', reason: 'state_unavailable' };
    }
    const r = rec as Record<string, unknown>;
    if (r.status === 'allow') return { kind: 'allow' };
    if (r.status === 'deny') {
      const reason = typeof r.reason === 'string' ? r.reason : 'user';
      return { kind: 'deny', reason };
    }
    if (Date.now() >= expires_at) return { kind: 'deny', reason: 'timeout' };
    await sleep(POLL_INTERVAL_MS);
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

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
  e.status = decision;
  if (typeof obj.reason === 'string') e.reason = obj.reason;
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

// Re-export for ergonomic imports
export { requireString };

/**
 * Approval resolution helpers. `approval::resolve` now writes the final
 * decision record directly to the same key the paused turn reads.
 */

import { requireString } from '../runtime/handler.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { emitApprovalResolved } from './events.js';
import type { StateBus } from './state-bus.js';
import { pendingKey } from './types.js';

const STEP_TOPIC = 'turn::step_requested';

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
  const reason = typeof obj.reason === 'string' ? obj.reason : null;
  const key = pendingKey(session_id, function_call_id);

  try {
    await bus.set(state_scope, key, { decision, reason });
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

export async function resumeSession(iii: ISdk, session_id: string): Promise<void> {
  await iii.trigger<unknown, unknown>({
    function_id: 'iii::durable::publish',
    payload: { topic: STEP_TOPIC, data: { session_id } },
  });
}

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
  const decision =
    obj.decision === 'allow' || obj.decision === 'deny'
      ? (obj.decision as 'allow' | 'deny')
      : 'deny';
  const reason = typeof obj.reason === 'string' ? obj.reason : null;

  await emitApprovalResolved(iii, session_id, { function_call_id, decision, reason });
  await resumeSession(iii, session_id);

  return out;
}

export { requireString };

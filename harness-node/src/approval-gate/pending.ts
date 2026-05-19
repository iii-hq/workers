/**
 * Approval resolution helpers. `approval::resolve` now writes the final
 * decision record directly to the same key the paused turn reads.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { pendingKey } from './types.js';

export async function handleResolve(
  iii: ISdk,
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
    await iii.trigger<unknown, unknown>({
      function_id: 'state::set',
      payload: { scope: state_scope, key, value: { decision, reason } },
    });
  } catch (err) {
    logger.error('approval-gate: failed to write resolved state', { err: String(err) });
    return { ok: false, error: 'state_write_failed' };
  }
  return { ok: true };
}

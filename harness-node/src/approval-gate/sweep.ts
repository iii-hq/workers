/**
 * Approval-gate sweep handler. Mirrors `approval-gate/src/lib.rs::handle_sweep_session`.
 *
 * Force-resolves pending approvals for a session as `decision: 'deny',
 * reason: 'timed_out'`. Wired into `router::abort`'s side-effects so an
 * aborted session never leaks pending approval entries.
 */

import { logger } from '../runtime/otel.js';
import type { StateBus } from './state-bus.js';
import { pendingKey } from './types.js';

export type SweepResult = { ok: true; swept: number } | { ok: false; error: string; swept: number };

export async function handleSweepSession(
  bus: StateBus,
  state_scope: string,
  payload: unknown,
): Promise<SweepResult> {
  const session_id =
    (typeof payload === 'object' && payload !== null
      ? (payload as Record<string, unknown>).session_id
      : undefined) ?? '';
  if (typeof session_id !== 'string' || !session_id) {
    return { ok: false, error: 'missing_session_id', swept: 0 };
  }

  let rows: unknown[];
  try {
    rows = await bus.listPrefix(state_scope, `${session_id}/`);
  } catch (err) {
    logger.error('approval-gate: sweep listPrefix failed', {
      session_id,
      error: err instanceof Error ? err.message : String(err),
    });
    return { ok: false, error: 'list_failed', swept: 0 };
  }

  const now = Date.now();
  let swept = 0;
  for (const row of rows) {
    if (!row || typeof row !== 'object') continue;
    const r = row as Record<string, unknown>;
    if (r.session_id !== session_id) continue;
    if (r.status !== 'pending') continue;
    const function_call_id = typeof r.function_call_id === 'string' ? r.function_call_id : null;
    if (!function_call_id) continue;

    const updated = {
      ...r,
      status: 'resolved',
      decision: 'deny',
      reason: 'timed_out',
      resolved_at: now,
    };
    try {
      await bus.set(state_scope, pendingKey(session_id, function_call_id), updated);
      swept += 1;
    } catch (err) {
      logger.warn('approval-gate: sweep set failed for one entry', {
        session_id,
        function_call_id,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }

  return { ok: true, swept };
}

/**
 * Approval-gate consume handler. Mirrors `approval-gate/src/lib.rs::handle_consume`.
 *
 * Returns resolved approval decisions for a session once. Each returned entry
 * is marked `status: 'consumed'` so a subsequent call sees an empty result.
 * The orchestrator calls this from `handle_awaiting` and `tearing_down`
 * (when `approval_required` is non-empty) to drain resolved approvals.
 */

import { logger } from '../runtime/otel.js';
import type { StateBus } from './state-bus.js';
import { pendingKey } from './types.js';

export type ConsumeResultEntry = {
  function_call_id: string;
  tool_call_id: string;
  function_id: unknown;
  args: unknown;
  decision: unknown;
  reason: unknown;
};

export type ConsumeResult =
  | { ok: true; entries: ConsumeResultEntry[] }
  | { ok: false; error: string; entries: ConsumeResultEntry[] };

export async function handleConsume(
  bus: StateBus,
  state_scope: string,
  payload: unknown,
): Promise<ConsumeResult> {
  const session_id =
    (typeof payload === 'object' && payload !== null
      ? (payload as Record<string, unknown>).session_id
      : undefined) ?? '';
  if (typeof session_id !== 'string' || !session_id) {
    return { ok: false, error: 'missing_session_id', entries: [] };
  }

  let rows: unknown[];
  try {
    rows = await bus.listPrefix(state_scope, `${session_id}/`);
  } catch (err) {
    logger.error('approval-gate: consume listPrefix failed', {
      session_id,
      error: err instanceof Error ? err.message : String(err),
    });
    return { ok: false, error: 'list_failed', entries: [] };
  }

  const entries: ConsumeResultEntry[] = [];
  for (const row of rows) {
    if (!row || typeof row !== 'object') continue;
    const r = row as Record<string, unknown>;
    if (r.session_id !== session_id) continue;
    if (r.status !== 'resolved') continue;
    const function_call_id = typeof r.function_call_id === 'string' ? r.function_call_id : null;
    if (!function_call_id) continue;

    entries.push({
      function_call_id,
      tool_call_id: function_call_id,
      function_id: r.function_id ?? null,
      args: r.args ?? {},
      decision: r.decision ?? 'deny',
      reason: r.reason ?? null,
    });

    const updated = { ...r, status: 'consumed', consumed_at: Date.now() };
    try {
      await bus.set(state_scope, pendingKey(session_id, function_call_id), updated);
    } catch (err) {
      logger.warn(
        'approval-gate: failed to mark consumed; entry will be re-returned next consume',
        {
          session_id,
          function_call_id,
          error: err instanceof Error ? err.message : String(err),
        },
      );
    }
  }

  return { ok: true, entries };
}

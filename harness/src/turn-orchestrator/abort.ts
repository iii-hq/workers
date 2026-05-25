/**
 * `router::abort` side-effects. The abort path writes the per-session abort
 * signal and, when a turn is paused on approvals, writes an aborted decision to
 * the `approvals` scope per parked call — the reactive approval trigger
 * (turn::on_approval) then wakes the session.
 */

import { STATE_SCOPE, pendingKey } from '../approval-gate/schemas.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import * as persistence from './persistence.js';
import { AGENT_SCOPE, abortSignalKey } from './state.js';

export async function performAbortSideEffects(iii: ISdk, session_id: string): Promise<void> {
  await trigger(iii, 'state::set', {
    scope: AGENT_SCOPE,
    key: abortSignalKey(session_id),
    value: true,
  });

  const rec = await persistence.loadRecord(iii, session_id);
  if (!rec || rec.state !== 'function_awaiting_approval' || !rec.awaiting_approval?.length) {
    return;
  }

  for (const entry of rec.awaiting_approval) {
    await trigger(iii, 'state::set', {
      scope: STATE_SCOPE,
      key: pendingKey(session_id, entry.function_call_id),
      value: { decision: 'aborted', reason: 'session_aborted' },
    });
  }
}

async function trigger(iii: ISdk, function_id: string, payload: unknown): Promise<void> {
  try {
    await iii.trigger<unknown, unknown>({ function_id, payload });
  } catch (err) {
    logger.warn(`abort side-effect failed: ${function_id}`, { err: String(err) });
  }
}

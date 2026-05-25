/**
 * Approval resolution handler. `approval::resolve` persists the decision to the
 * shared `approvals` scope; the turn-orchestrator's reactive trigger
 * (turn::on_approval) wakes the parked session.
 */

import type { ISdk } from 'iii-sdk';
import { logger } from '../runtime/otel.js';
import {
  STATE_SCOPE,
  type ResolvePayloadInput,
  ResolvePayloadSchema,
  pendingKey,
  resolveFunctionOptions,
} from './schemas.js';

export type ResolveResult =
  | { ok: true }
  | { ok: false; error: 'invalid_payload' | 'resume_failed' };

export async function handleResolveRequest(
  iii: ISdk,
  payload: ResolvePayloadInput,
): Promise<ResolveResult> {
  const parsed = ResolvePayloadSchema.safeParse(payload);
  if (!parsed.success) return { ok: false, error: 'invalid_payload' };

  const { session_id, function_call_id, decision, reason } = parsed.data;

  try {
    await iii.trigger({
      function_id: 'state::set',
      payload: {
        scope: STATE_SCOPE,
        key: pendingKey(session_id, function_call_id),
        value: { decision, reason },
      },
    });
  } catch (err) {
    logger.error('approval-gate: decision write failed', { err: String(err), session_id });
    return { ok: false, error: 'resume_failed' };
  }
  return { ok: true };
}

export async function register(iii: ISdk): Promise<void> {
  iii.registerFunction(
    'approval::resolve',
    async (payload: ResolvePayloadInput) => handleResolveRequest(iii, payload),
    resolveFunctionOptions,
  );
}

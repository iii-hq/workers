/**
 * Approval resolution handler. `approval::resolve` routes the decision to
 * the per-call resume function owned by the turn-orchestrator.
 */

import type { ISdk } from 'iii-sdk';
import { logger } from '../runtime/otel.js';
import {
  type ResolvePayloadInput,
  ResolvePayloadSchema,
  approvalResumeFnId,
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
  const resumeFnId = approvalResumeFnId(session_id, function_call_id);

  try {
    await iii.trigger({
      function_id: resumeFnId,
      payload: { decision, reason },
    });
  } catch (err) {
    logger.error('approval-gate: resume fn invoke failed', { err: String(err), resumeFnId });
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

/**
 * Approval resolution handler. `approval::resolve` writes the final
 * decision record directly to the `approvals` state scope; the engine's
 * state trigger picks it up and wakes the paused turn.
 */

import type { ISdk } from 'iii-sdk';
import { unwrapBody } from '../runtime/handler.js';
import { logger } from '../runtime/otel.js';
import { ResolvePayloadSchema, type StateSetApprovalPayload, pendingKey } from './schemas.js';

export type ResolveResult =
  | { ok: true }
  | { ok: false; error: 'missing_id' | 'bad_decision' | 'state_write_failed' };

/**
 * Map a Zod parse failure to the stable wire error code today's callers
 * expect. Granularity matches the pre-refactor handler:
 * - decision is present but not 'allow' / 'deny' → `bad_decision`
 * - anything else (missing ids, malformed payload) → `missing_id`
 *
 * Inspects the raw body rather than walking Zod issues because a fully
 * missing payload also produces a `decision`-pathed issue, which would
 * misclassify "no payload at all" as a bad decision.
 */
function classifyParseError(body: Record<string, unknown>): 'missing_id' | 'bad_decision' {
  const d = body.decision;
  if (d !== undefined && d !== 'allow' && d !== 'deny') return 'bad_decision';
  return 'missing_id';
}

export async function handleResolve(
  iii: ISdk,
  state_scope: string,
  payload: unknown,
): Promise<ResolveResult> {
  const body = unwrapBody(payload);
  const parsed = ResolvePayloadSchema.safeParse(body);
  if (!parsed.success) {
    return { ok: false, error: classifyParseError(body) };
  }
  const { session_id, function_call_id, decision, reason } = parsed.data;
  const key = pendingKey(session_id, function_call_id);

  try {
    await iii.trigger<StateSetApprovalPayload, void>({
      function_id: 'state::set',
      payload: { scope: state_scope, key, value: { decision, reason } },
    });
  } catch (err) {
    logger.error('approval-gate: failed to write resolved state', { err: String(err) });
    return { ok: false, error: 'state_write_failed' };
  }
  return { ok: true };
}

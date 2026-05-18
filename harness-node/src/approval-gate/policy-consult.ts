/**
 * Consult the harness's `policy::check_permissions`. Returns `null` when
 * the policy worker is unreachable (callers fall back to the legacy
 * `approval_required` substring check).
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { MatchedConstraint } from './types.js';

export type PolicyOutcome =
  | { kind: 'allow'; rule_id: string }
  | { kind: 'deny'; rule_id: string; matched_constraint: MatchedConstraint | null }
  | { kind: 'needs_approval' };

export function parsePolicyReply(value: unknown): PolicyOutcome {
  if (!value || typeof value !== 'object') return { kind: 'needs_approval' };
  const obj = value as Record<string, unknown>;
  if (obj.decision === 'allow') {
    return {
      kind: 'allow',
      rule_id: typeof obj.rule_id === 'string' ? obj.rule_id : '',
    };
  }
  if (obj.decision === 'deny') {
    const mc =
      obj.matched_constraint && typeof obj.matched_constraint === 'object'
        ? (obj.matched_constraint as Record<string, unknown>)
        : null;
    const matched: MatchedConstraint | null =
      mc && typeof mc.field === 'string' && typeof mc.operator === 'string' && 'value' in mc
        ? { field: mc.field, operator: mc.operator, value: mc.value }
        : null;
    return {
      kind: 'deny',
      rule_id: typeof obj.rule_id === 'string' ? obj.rule_id : '',
      matched_constraint: matched,
    };
  }
  return { kind: 'needs_approval' };
}

export async function consultPolicy(
  iii: ISdk,
  policy_function_id: string,
  function_id: string,
  args: unknown,
): Promise<PolicyOutcome | null> {
  try {
    const resp = await iii.trigger<unknown, unknown>({
      function_id: policy_function_id,
      payload: { function_id, args },
      timeoutMs: 5_000,
    });
    return parsePolicyReply(resp);
  } catch (err) {
    logger.debug('policy worker unreachable; falling back to approval_required list', {
      policy_function_id,
      err: String(err),
    });
    return null;
  }
}

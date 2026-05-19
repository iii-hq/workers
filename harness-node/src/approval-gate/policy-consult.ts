/**
 * Decode the wire reply from `policy::check_permissions` into a typed
 * `PolicyOutcome`. The actual `iii.trigger` to the policy function lives
 * in the orchestrator's `consultBefore` (`harness-node/src/turn-orchestrator/hook.ts`)
 * — that's the only caller of this module.
 */

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

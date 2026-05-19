/**
 * Wire types shared across the gate, the orchestrator hook, and the
 * provider serialisers. Mirrors `approval-gate/src/lib.rs`.
 */

export const FN_RESOLVE = 'approval::resolve';
export const STATE_SCOPE = 'approvals';
export const DENIAL_SCHEMA_VERSION = 1;

export type DeniedBy = 'permissions' | 'user' | 'gate_unavailable';

export type MatchedConstraint = {
  field: string;
  operator: string;
  value: unknown;
};

export type DenialEnvelope = {
  schema_version: number;
  status: 'denied';
  denied_by: DeniedBy;
  function_id: string;
  rule_id?: string;
  rule_action?: 'deny';
  matched_constraint?: MatchedConstraint;
  args_excerpt?: unknown;
  reason: string;
};

export type WireDecision = 'allow' | 'deny';

export function pendingKey(session_id: string, function_call_id: string): string {
  if (session_id.includes('/')) throw new Error('session_id must not contain "/"');
  if (function_call_id.includes('/')) throw new Error('function_call_id must not contain "/"');
  return `${session_id}/${function_call_id}`;
}

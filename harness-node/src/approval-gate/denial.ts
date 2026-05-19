/**
 * Denial envelope assembly. The recursive redaction tree lives in
 * `./redact.ts`; this module only composes the two wire envelopes
 * (permissions-side and user-side) and the human-readable deny reason.
 */

import { redact } from './redact.js';
import { DENIAL_SCHEMA_VERSION, type DenialEnvelope, type MatchedConstraint } from './schemas.js';

export function redactedArgsExcerpt(args: unknown): unknown {
  return redact(args);
}

export function reasonForPermissionsDeny(
  function_id: string,
  rule_id: string,
  matched: MatchedConstraint | null,
): string {
  if (matched) {
    return `Permission denied: ${function_id} matched rule ${rule_id} on ${matched.field} ${matched.operator} ${JSON.stringify(matched.value)}. Try different arguments or use a different function.`;
  }
  return `Permission denied: ${function_id} matched rule ${rule_id}. This function is blocked by policy; try a different function.`;
}

export function permissionsDenyEnvelope(
  function_id: string,
  rule_id: string,
  matched_constraint: MatchedConstraint | null,
  args: unknown,
): DenialEnvelope {
  return {
    schema_version: DENIAL_SCHEMA_VERSION,
    status: 'denied',
    denied_by: 'permissions',
    function_id,
    rule_id,
    rule_action: 'deny',
    matched_constraint: matched_constraint ?? undefined,
    args_excerpt: redactedArgsExcerpt(args),
    reason: reasonForPermissionsDeny(function_id, rule_id, matched_constraint),
  };
}

export function userDenyEnvelope(
  function_id: string,
  reason: string | null,
  args: unknown,
): DenialEnvelope {
  return {
    schema_version: DENIAL_SCHEMA_VERSION,
    status: 'denied',
    denied_by: 'user',
    function_id,
    args_excerpt: redactedArgsExcerpt(args),
    reason: reason ?? 'Rejected by operator.',
  };
}

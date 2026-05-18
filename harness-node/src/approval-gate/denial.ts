/**
 * Denial envelope assembly + arg redaction. Mirrors
 * `approval-gate/src/lib.rs::{redact, reason_for_permissions_deny,
 * permissions_deny_envelope, user_deny_envelope}`.
 */

import { DENIAL_SCHEMA_VERSION, type DenialEnvelope, type MatchedConstraint } from './types.js';

const ARGS_EXCERPT_LEN_CAP = 256;

const REDACT_KEYS = new Set([
  'password',
  'token',
  'api_key',
  'apikey',
  'secret',
  'auth',
  'authorization',
  'access_key',
  'access_token',
  'refresh_token',
  'private_key',
]);

function clip(s: string): string {
  if (s.length <= ARGS_EXCERPT_LEN_CAP) return s;
  return `${[...s].slice(0, ARGS_EXCERPT_LEN_CAP).join('')}…`;
}

export function redact(value: unknown): unknown {
  if (typeof value === 'string') return clip(value);
  if (Array.isArray(value)) return value.map(redact);
  if (value && typeof value === 'object') {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      const lower = k.toLowerCase();
      const isSecret =
        REDACT_KEYS.has(lower) || Array.from(REDACT_KEYS).some((s) => lower.endsWith(`_${s}`));
      out[k] = isSecret ? '<redacted>' : redact(v);
    }
    return out;
  }
  return value;
}

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

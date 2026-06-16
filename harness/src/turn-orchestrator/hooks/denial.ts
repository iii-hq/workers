/**
 * Denial envelope shapes shared by the pre_trigger hook chain and the
 * generic trigger failure path. The full approval policy (permission
 * modes, allow-lists, yaml rules, redaction) lives in the standalone
 * approval-gate worker; the harness only renders denial outcomes into
 * function results.
 */

import type { ContentBlock } from '../../types/content.js';
import type { FunctionResult } from '../../types/function.js';

export const DENIAL_SCHEMA_VERSION = 1;

export type DeniedBy = 'permissions' | 'user' | 'hook' | 'gate_unavailable';

export type DenialEnvelope = {
  schema_version: typeof DENIAL_SCHEMA_VERSION;
  status: 'denied';
  denied_by: DeniedBy;
  function_id: string;
  rule_id?: string;
  rule_action?: 'deny';
  matched_constraint?: { field: string; operator: string; value: unknown };
  args_excerpt?: unknown;
  reason: string;
};

export function gateUnavailableEnvelope(function_id: string, reason: string): DenialEnvelope {
  return {
    schema_version: DENIAL_SCHEMA_VERSION,
    status: 'denied',
    denied_by: 'gate_unavailable',
    function_id,
    reason,
  };
}

/**
 * Denial returned by a pre_trigger hook. The hook wire carries only
 * `{ decision: "deny", reason }` — the hook owner renders its full
 * envelope into `reason`; harness-side we keep `status: 'denied'` so
 * downstream rendering (`[PERMISSION_DENIED]` in types/wire.ts) and
 * `isErrorResult` classify it correctly.
 */
export function hookDenyEnvelope(function_id: string, reason: string): DenialEnvelope {
  return {
    schema_version: DENIAL_SCHEMA_VERSION,
    status: 'denied',
    denied_by: 'hook',
    function_id,
    reason,
  };
}

export function denialResult(denial: DenialEnvelope): FunctionResult {
  const text: ContentBlock = { type: 'text', text: denial.reason };
  return { content: [text], details: denial, terminate: false };
}

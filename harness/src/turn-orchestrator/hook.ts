/**
 * Approval consultation. Calls `policy::check_permissions` directly and maps
 * the reply to allow / deny / pending. Fail-closed on transport errors:
 * unreachable policy → deny with `gate_unavailable`.
 */

import { permissionsDenyEnvelope } from '../approval-gate/denial.js';
import { DENIAL_SCHEMA_VERSION, type DenialEnvelope } from '../approval-gate/schemas.js';
import type {
  CheckPermissionsPayload,
  PolicyCheckReply,
} from '../harness/policy/check-permissions.js';
import type { ISdk } from '../runtime/iii.js';
export type { DenialEnvelope } from '../approval-gate/schemas.js';
import { logger } from '../runtime/otel.js';
import type { FunctionCall } from '../types/function.js';

/** Fail-closed budget for the synchronous policy consult before a call. */
export const POLICY_TIMEOUT_MS = 5_000;

export type HookOutcome =
  | { kind: 'allow' }
  | { kind: 'pending' }
  | { kind: 'deny'; denial: DenialEnvelope };

export function gateUnavailableEnvelope(function_id: string, reason: string): DenialEnvelope {
  return {
    schema_version: DENIAL_SCHEMA_VERSION,
    status: 'denied',
    denied_by: 'gate_unavailable',
    function_id,
    reason,
  };
}

export async function consultBefore(iii: ISdk, function_call: FunctionCall): Promise<HookOutcome> {
  try {
    const reply = await iii.trigger<CheckPermissionsPayload, PolicyCheckReply>({
      function_id: 'policy::check_permissions',
      payload: {
        function_id: function_call.function_id,
        args: function_call.arguments as CheckPermissionsPayload['args'],
      },
      timeoutMs: POLICY_TIMEOUT_MS,
    });
    switch (reply.decision) {
      case 'allow':
        return { kind: 'allow' };
      case 'deny':
        return {
          kind: 'deny',
          denial: permissionsDenyEnvelope(
            function_call.function_id,
            reply.rule_id,
            reply.matched_constraint ?? null,
            function_call.arguments,
          ),
        };
      case 'needs_approval':
        return { kind: 'pending' };
    }
  } catch (err) {
    logger.warn('policy consult failed; failing closed', {
      function_id: function_call.function_id,
      err: String(err),
    });
    return {
      kind: 'deny',
      denial: gateUnavailableEnvelope(
        function_call.function_id,
        `policy unreachable: ${String(err)}`,
      ),
    };
  }
}

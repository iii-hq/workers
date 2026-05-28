/**
 * Approval consultation. Resolves the approval verdict for one agent
 * function call. Evaluation order (race-safe via a single settings
 * snapshot per call):
 *
 *   1. Deny if the agent is trying to invoke a human-only approval
 *      function (`approval::set_mode`, `approval::add_always_allow`,
 *      etc.) — self-escalation defense.
 *   2. Snapshot per-session approval settings.
 *   3. `mode === 'full'` → allow.
 *   4. `mode === 'auto'` and `function_id` is in the user's curated
 *      `always_allow` list → allow. In any other mode the list is
 *      dormant — the user can build it up but it only takes effect
 *      under Auto.
 *   5. Fall through to `policy::check_permissions` (yaml rules) — the
 *      pre-existing behavior.
 *
 * Fail-closed on transport errors: unreachable policy → deny with
 * `gate_unavailable`.
 */

import { permissionsDenyEnvelope } from '../approval-gate/denial.js';
import { DENIAL_SCHEMA_VERSION, type DenialEnvelope } from '../approval-gate/schemas.js';
import { isHumanOnlyApprovalFunction } from '../approval-gate/settings/human-only.js';
import { readSettings } from '../approval-gate/settings/store.js';
import { settingsVerdict } from '../approval-gate/settings/verdict.js';
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

function humanOnlyDenial(function_id: string, args: unknown): DenialEnvelope {
  return permissionsDenyEnvelope(function_id, 'human_only_function', null, args);
}

function extractSessionId(args: unknown): string | null {
  if (args && typeof args === 'object' && !Array.isArray(args)) {
    const candidate = (args as Record<string, unknown>).session_id;
    if (typeof candidate === 'string' && candidate.length > 0) return candidate;
  }
  return null;
}

export async function consultBefore(iii: ISdk, function_call: FunctionCall): Promise<HookOutcome> {
  if (isHumanOnlyApprovalFunction(function_call.function_id)) {
    return {
      kind: 'deny',
      denial: humanOnlyDenial(function_call.function_id, function_call.arguments),
    };
  }

  const session_id = extractSessionId(function_call.arguments);
  const settings = session_id ? await readSettings(iii, session_id) : null;

  if (settings && settingsVerdict(settings, function_call.function_id) === 'allow') {
    return { kind: 'allow' };
  }

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

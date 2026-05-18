/**
 * Approval consultation. Calls `policy::check_permissions` directly and maps
 * the reply to allow / deny / pending. Fail-closed on transport errors:
 * unreachable policy → deny with `gate_unavailable`, unless the call appears
 * in the legacy `approval_required` list (then treat as pending so the user
 * can resolve it).
 *
 * `publishAfter` still goes through hook-fanout because the after-hook is a
 * pluggable merge point with multiple potential consumers.
 */

import { permissionsDenyEnvelope } from '../approval-gate/denial.js';
import { parsePolicyReply } from '../approval-gate/policy-consult.js';
import { DENIAL_SCHEMA_VERSION } from '../approval-gate/types.js';
import type { DenialEnvelope } from '../approval-gate/types.js';
import type { ISdk } from '../runtime/iii.js';
export type { DeniedBy, DenialEnvelope } from '../approval-gate/types.js';
import { logger } from '../runtime/otel.js';
import type { FunctionCall } from '../types/function.js';

export const TOPIC_AFTER = 'agent::after_function_call';
export const HOOK_TIMEOUT_MS = 10_000;

export type HookOutcome =
  | { kind: 'allow' }
  | { kind: 'pending' }
  | { kind: 'deny'; denial: DenialEnvelope | Record<string, unknown> };

export function gateUnavailableEnvelope(function_id: string, reason: string): DenialEnvelope {
  return {
    schema_version: DENIAL_SCHEMA_VERSION,
    status: 'denied',
    denied_by: 'gate_unavailable',
    function_id,
    reason,
  };
}

export async function consultBefore(
  iii: ISdk,
  function_call: FunctionCall,
  approval_required: string[],
  // session_id is accepted for future correlation; the current policy wire format only uses function_id + args.
  session_id: string | undefined,
  policy_function_id: string,
): Promise<HookOutcome> {
  let raw: unknown;
  try {
    // 5s is a safe budget for a synchronous policy check; HOOK_TIMEOUT_MS is reserved for publishAfter's fanout deadline.
    raw = await iii.trigger<unknown, unknown>({
      function_id: policy_function_id,
      payload: { function_id: function_call.function_id, args: function_call.arguments },
      timeoutMs: 5_000,
    });
  } catch (err) {
    logger.warn('policy consult failed; checking legacy approval_required fallback', {
      function_id: function_call.function_id,
      err: String(err),
    });
    if (approval_required.includes(function_call.function_id)) {
      return { kind: 'pending' };
    }
    return {
      kind: 'deny',
      denial: gateUnavailableEnvelope(
        function_call.function_id,
        `policy unreachable: ${String(err)}`,
      ),
    };
  }

  const decision = parsePolicyReply(raw);
  if (decision.kind === 'allow') return { kind: 'allow' };
  if (decision.kind === 'deny') {
    return {
      kind: 'deny',
      denial: permissionsDenyEnvelope(
        function_call.function_id,
        decision.rule_id,
        decision.matched_constraint,
        function_call.arguments,
      ),
    };
  }
  // needs_approval. Honor the legacy per-run approval_required allowlist: if
  // the caller pre-declared their own list and this function isn't on it,
  // return allow so no caller observes a change in semantics.
  if (approval_required.length > 0 && !approval_required.includes(function_call.function_id)) {
    return { kind: 'allow' };
  }
  return { kind: 'pending' };
}

export async function publishAfter(
  iii: ISdk,
  function_call: FunctionCall,
  result: unknown,
): Promise<unknown> {
  const payload = {
    topic: TOPIC_AFTER,
    payload: { function_call, result },
    merge_rule: 'field_merge',
    timeout_ms: HOOK_TIMEOUT_MS,
  };
  try {
    const resp = await iii.trigger<unknown, { merged?: unknown }>({
      function_id: 'hook-fanout::publish_collect',
      payload,
    });
    return resp?.merged ?? null;
  } catch {
    return null;
  }
}

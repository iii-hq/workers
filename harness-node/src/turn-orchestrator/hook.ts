/**
 * `agent::before_function_call` hook. Mirrors
 * `turn-orchestrator/src/hook.rs`.
 *
 * Fail-closed: any hook failure (timeout, error, no subscriber) returns
 * `Deny` with `denied_by: gate_unavailable` so dispatch never silently
 * allows a call.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { FunctionCall } from '../types/function.js';

export const TOPIC_BEFORE = 'agent::before_function_call';
export const TOPIC_AFTER = 'agent::after_function_call';
export const HOOK_TIMEOUT_MS = 10_000;
export const DENIAL_SCHEMA_VERSION = 1;

export type DenialEnvelope = {
  schema_version: number;
  status: 'denied';
  denied_by: 'permissions' | 'user' | 'gate_unavailable';
  function_id: string;
  rule_id?: string;
  rule_action?: 'deny';
  matched_constraint?: { field: string; operator: string; value: unknown };
  args_excerpt?: unknown;
  reason: string;
};

export type HookOutcome =
  | { kind: 'allow' }
  | { kind: 'pending'; merged: Record<string, unknown> }
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
  session_id?: string,
): Promise<HookOutcome> {
  // PR #150: include session_id at the inner-payload root so approval-gate's
  // extractCall can route it. Without this, the gate sees a null session_id,
  // returns {block:false}, and the call passes through unapproved.
  const inner = session_id
    ? { session_id, function_call, approval_required }
    : { function_call, approval_required };
  const payload = {
    topic: TOPIC_BEFORE,
    payload: inner,
    merge_rule: 'first_block_wins',
    timeout_ms: HOOK_TIMEOUT_MS,
  };
  let merged: Record<string, unknown>;
  try {
    const resp = await iii.trigger<unknown, { merged?: unknown }>({
      function_id: 'hook-fanout::publish_collect',
      payload,
    });
    merged =
      resp?.merged && typeof resp.merged === 'object'
        ? (resp.merged as Record<string, unknown>)
        : {};
  } catch (err) {
    logger.warn('before_function_call hook failed; failing closed', {
      function_id: function_call.function_id,
      err: String(err),
    });
    return {
      kind: 'deny',
      denial: gateUnavailableEnvelope(
        function_call.function_id,
        `permission gate did not respond: ${String(err)}`,
      ),
    };
  }
  const blocked = merged.block === true;
  if (!blocked) return { kind: 'allow' };
  // PR #150: distinguish pending (awaiting human approval) from hard deny.
  // Pending replies carry `status: 'pending'` and must be surfaced as a
  // terminating prefilled placeholder so the LLM stops and the UI can
  // render an approval prompt instead of treating it as an error.
  if (merged.status === 'pending') {
    return { kind: 'pending', merged };
  }
  const denial =
    (merged.denial as DenialEnvelope | undefined) ??
    gateUnavailableEnvelope(
      function_call.function_id,
      typeof merged.reason === 'string' ? merged.reason : 'Permission denied.',
    );
  return { kind: 'deny', denial };
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
// reload marker 1779108905

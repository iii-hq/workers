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
): Promise<HookOutcome> {
  const inner = { function_call, approval_required };
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
